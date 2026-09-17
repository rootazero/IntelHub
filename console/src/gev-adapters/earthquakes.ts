// Real (wave-1) earthquakes layer source: USGS quakes served by IntelHub.
//
// Replaces the T3 `stubs.earthquakes` entry. The transport is the injected
// authenticated `apiFetch` (see ./http.ts) hitting the hub REST endpoint
// `GET /api/v1/gev/earthquakes`, which projects `geo_events` rows with
// `kind='quake'` (produced by hub-core monitor/sources/usgs.rs) into exactly the
// row shape the vendored engine destructures in
// `gev-engine/src/layers/earthquakes/index.js:86`:
//
//   { stableId, usgsId, lon, lat, depthKm, mag, place, time }
//
// This module is a pure pass-through on purpose: the endpoint already emits
// engine-ready rows, so re-normalizing here would create a second place where
// the contract can drift (the endpoint's `quake_row` + `normalizeEarthquakeSnapshot`
// in the engine are the two authorities). The only local policy is the
// defensive "must be an array" guard below.

import type { ApiFetch } from "./http";
import type { EarthquakeRow } from "./types";

export interface EarthquakesSource {
  label: string;
  getSnapshot(options?: { signal?: AbortSignal }): Promise<EarthquakeRow[]>;
}

export const earthquakesSource = (apiFetch: ApiFetch): EarthquakesSource => ({
  label: "USGS via IntelHub",
  async getSnapshot({ signal }: { signal?: AbortSignal } = {}) {
    const res = await apiFetch("/api/v1/gev/earthquakes", { signal });
    if (!res.ok) throw new Error(`IntelHub earthquakes HTTP ${res.status}`);
    const rows: unknown = await res.json();
    // A non-array body (error envelope, proxy HTML, schema change) must degrade
    // to "no quakes" rather than reach the engine's `for (const {...} of rows)`,
    // which would throw and park the whole layer in its error state.
    return Array.isArray(rows) ? (rows as EarthquakeRow[]) : [];
  },
});
