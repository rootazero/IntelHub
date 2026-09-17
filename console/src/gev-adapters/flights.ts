// Real (wave-1) flights layer source: the IntelHub P1 adsb.lol snapshot.
//
// Replaces the T3 `stubs.flights` entry. The transport is the injected
// authenticated `apiFetch` (see ./http.ts) hitting the hub REST endpoint
// `GET /api/v1/globe/aircraft`, which serves the Redis snapshot the P1 adsb
// collector accumulates (`hub:globe:aircraft`, TTL 300s; hub-core api.rs
// `console_globe_aircraft`):
//
//   { ts, count, coverage: "hotspots+mil+squawk", cycle_secs, last_tick,
//     aircraft: [{ hex, flight, lat, lon, alt_m, gs, track, squawk, mil, age_s }] }
//
// Coverage is the collector's rotation over military + emergency squawks +
// hotspots, not a global sweep — the engine labels and downgrades freshness on
// its own, so `coverage` is passed through for the HUD rather than filtered on.
// The query the engine passes (viewer bounds, flights/queries.js `_flightQuery`)
// is ignored: an upstream snapshot is not bbox-scoped.
//
// The returned envelope is exactly what
// gev-engine/src/layers/flights/ingestion.js:35-53 destructures; the record
// mapping (knots → m/s, age_s → epoch ms) lives in ./aircraft-map.ts.

import type { ApiFetch } from "./http";
import type { AdsbEnvelope, GevAircraftRecord } from "./aircraft-map";
import { snapshotEpochMs, toEnvelope, toGevRecord } from "./aircraft-map";
import type { SnapshotEnvelope } from "./types";

export interface FlightsSource {
  label: string;
  getSnapshot(
    query?: unknown,
    options?: { signal?: AbortSignal },
  ): Promise<SnapshotEnvelope<GevAircraftRecord>>;
}

export const flightsSource = (apiFetch: ApiFetch): FlightsSource => ({
  label: "adsb.lol via IntelHub",
  async getSnapshot(_query: unknown, { signal }: { signal?: AbortSignal } = {}) {
    const res = await apiFetch("/api/v1/globe/aircraft", { signal });
    if (!res.ok) throw new Error(`IntelHub flights HTTP ${res.status}`);
    const env: AdsbEnvelope = await res.json();
    // The hub's degraded body is `{ stale: true, aircraft: [] }`, and a proxy
    // error envelope could carry a non-array; either way the layer must show
    // "no aircraft" instead of throwing inside the map.
    const points = Array.isArray(env.aircraft) ? env.aircraft : [];
    const observedAtMs = snapshotEpochMs(env);
    return toEnvelope(
      points.map((p) => toGevRecord(p, observedAtMs)),
      env,
      "intelhub-adsb",
      Date.now(),
    );
  },
});
