// Real (wave-1) military layer source: military aircraft inside the IntelHub P1
// adsb snapshot.
//
// Replaces the T3 `stubs.military` entry. Same endpoint and same envelope
// contract as ./flights.ts (`GET /api/v1/globe/aircraft`), but the record set
// is narrowed to `mil === true` — the hub's `dbFlags & 1` bit, which adsb.lol
// sets for military/state aircraft (hub-core monitor/sources/adsb.rs
// `parse_ac_array`). The engine keys the military layer off the same bit at the
// source (`sources/live/aircraft.js` readsb path + the mil registry), and its
// flights layer suppresses any hex the military layer already owns
// (flights/snapshotRenderer.js:57), so both layers can read one snapshot
// without double-drawing.
//
// The mapping is shared with the flights source (./aircraft-map.ts) — the
// engine's military records.js destructures the identical observation shape.
// `getTrack`/`getIdentities` are deliberately not implemented: the engine
// resolves track history from its own fix history and marks the optional
// methods as skippable.

import type { ApiFetch } from "./http";
import { snapshotEpochMs, toEnvelope, toGevRecord } from "./aircraft-map";
import type { AdsbEnvelope, GevAircraftRecord, SnapshotEnvelope } from "./types";

export interface MilitarySource {
  label: string;
  getSnapshot(
    query?: unknown,
    options?: { signal?: AbortSignal },
  ): Promise<SnapshotEnvelope<GevAircraftRecord>>;
}

export const militarySource = (apiFetch: ApiFetch): MilitarySource => ({
  label: "adsb.lol mil via IntelHub",
  async getSnapshot(_q: unknown, { signal }: { signal?: AbortSignal } = {}) {
    const res = await apiFetch("/api/v1/globe/aircraft", { signal });
    if (!res.ok) throw new Error(`IntelHub military HTTP ${res.status}`);
    const env: AdsbEnvelope = await res.json();
    const points = Array.isArray(env.aircraft) ? env.aircraft : [];
    const mil = points.filter((p) => p.mil === true);
    const observedAtMs = snapshotEpochMs(env);
    return toEnvelope(
      mil.map((p) => toGevRecord(p, observedAtMs)),
      env,
      "intelhub-adsb-mil",
      Date.now(),
    );
  },
});
