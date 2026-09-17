// Contract-valid, read-only stubs for the 15 GEV layer sources.
//
// Goal (P2 T3): every layer can be enabled and renders empty WITHOUT throwing
// and WITHOUT fabricating data. These are pure data objects — no engine
// imports, no I/O. Wave-1 layers (earthquakes/satellites/flights/military) are
// replaced by real implementations in T4-T6 and re-registered in `index.ts`.
//
// Each shape below was checked against the engine's actual ingestion path (the
// layer destructures fields, it does not validate an envelope class), so a
// stub that merely satisfies the SOURCE_METHODS name list is not enough.
// Load-bearing evidence per layer is cited inline.

import { emptyEnvelope } from "./types";
import type { SnapshotEnvelope } from "./types";

// ── Envelope snapshot layers ───────────────────────────────────────────────
// flights/military ingestion read the envelope keys (status, ageMs,
// coverage, source). emptyEnvelope supplies every required key with an
// empty record set. vessels graduated to a real adapter in GEV P3 T2
// (./vessels.ts).

const envelopeSource = (layer: string) => ({
  async getSnapshot(): Promise<SnapshotEnvelope> {
    return emptyEnvelope(`intelhub-stub:${layer}`);
  },
});

export const flights = envelopeSource("flights");
export const military = envelopeSource("military");

// ── FIRMS ──────────────────────────────────────────────────────────────────
// firms/ingestion.js:loadHeatmap → `payload.keyRequired` short-circuit, else
// `payload.stale`, `payload.fires`, `payload.fetchedAt`. Real /api/firms
// returns `{ fires, fetchedAt, stale }`; empty + stale is the honest stub.

export const firms = {
  async getSnapshot(): Promise<{
    fires: unknown[];
    fetchedAt: number;
    stale: boolean;
  }> {
    return { fires: [], fetchedAt: Date.now(), stale: true };
  },
};

// ── Earthquakes (USGS) ─────────────────────────────────────────────────────
// earthquakes/index.js:71 → `for (const {...} of rows)`; the real
// source.js normalizes a GeoJSON FeatureCollection into a plain row array.

export const earthquakes = {
  async getSnapshot(): Promise<unknown[]> {
    return [];
  },
};

// ── Submarine cables (TeleGeography) ───────────────────────────────────────
// submarineCables/ingestion.js:load destructures `{ cables, landingPoints }`
// and requires each to be a GeoJSON FeatureCollection (Array.isArray
// (.features)). Empty features → Cesium loads zero entities, nothing throws.
// `source.label` is also read for datasource names.

const emptyFeatureCollection = () => ({
  type: "FeatureCollection" as const,
  features: [] as unknown[],
});

export const cables = {
  label: "IntelHub stub",
  async fetch(): Promise<{
    cables: { type: "FeatureCollection"; features: unknown[] };
    landingPoints: { type: "FeatureCollection"; features: unknown[] };
  }> {
    return {
      cables: emptyFeatureCollection(),
      landingPoints: emptyFeatureCollection(),
    };
  },
};

// ── ALPR (Overpass) ────────────────────────────────────────────────────────
// layers/alpr/index.js:188 → validateAlprSnapshot (records.js:126) requires
// `Array.isArray(records)` and boolean `stale`/`saturated`; an empty records
// array passes and the layer renders nothing.

export const alpr = {
  async fetch(): Promise<{
    records: unknown[];
    stale: boolean;
    saturated: boolean;
  }> {
    return { records: [], stale: false, saturated: false };
  },
};

// ── Launches ───────────────────────────────────────────────────────────────
// launches/source.js getLaunches → array OR `{ results }`;
// launches/model.js:169 normalizeRocketLaunches accepts either.
// getActiveTle returns raw TLE text (orbits.parseTLE("") → no entries).

export const launches = {
  async getLaunches(): Promise<{ results: unknown[] }> {
    return { results: [] };
  },
  async getActiveTle(): Promise<string> {
    return "";
  },
};

// ── CCTV ───────────────────────────────────────────────────────────────────
// cctv/catalog.js:86 `data?.sources` must be an array (else []); cctv/health.js
// `data?.cameras` likewise. getFrameUrl/getMediaUrl return strings that the
// frame loader consumes; "" means "no frame" without a thrown TypeError.

export const cctv = {
  async getCatalog(): Promise<{ sources: unknown[] }> {
    return { sources: [] };
  },
  async getHealth(): Promise<{ cameras: unknown[] }> {
    return { cameras: [] };
  },
  getFrameUrl: (): string => "",
  getMediaUrl: (): string => "",
};

// ── Radio ──────────────────────────────────────────────────────────────────
// radio/ingestion.js:24 → body.stations must be a non-empty usable array, plus
// string `updatedAt`, boolean `stale`/`degraded`, `acceptedGeneration` and
// `catalogInstance`. An empty directory cannot satisfy the non-empty check, so
// the layer lands in its own caught "temporarily unavailable" state and renders
// empty — the same degradation it shows for a real upstream outage.
// recordClick resolves void.

export const radio = {
  async getDirectory(): Promise<{
    stations: unknown[];
    updatedAt: string;
    stale: boolean;
    degraded: boolean;
    acceptedGeneration: null;
    catalogInstance: null;
  }> {
    return {
      stations: [],
      updatedAt: new Date().toISOString(),
      stale: true,
      degraded: true,
      acceptedGeneration: null,
      catalogInstance: null,
    };
  },
  async recordClick(): Promise<void> {
    // no-op: nothing to report while the directory is stubbed
  },
};

// ── Traffic (Overpass roads + TomTom flow) ─────────────────────────────────
// traffic/ingestion.js fetchRoads → `source.requestRoads(...)` must resolve a
// Response-like `{ ok, status, headers, json() }` whose json() yields
// `{ roads: [...] }` (Array required). `{ roads: [] }` keeps the road layer
// empty and non-throwing.
// traffic/flow.js ensureFlowStatus → `source.getStatus()` must yield
// `{ hasKey: boolean }`; `false` selects the engine's built-in simulated
// (keyless) mode and skips live flow entirely — the honest "no live data" path.
// traffic/flowSource.js fetchFlowForBounds resolves a FLAT ARRAY of flow
// segments (traffic/flow.js destructures the result directly and passes it to
// matchFlowToRoads, which requires an array), so the degraded value is `[]`.
// getFlowSessionStats → `{ tilesFetched }`; resetFlowTileCache is void.

export const traffic = {
  async requestRoads(): Promise<{
    ok: boolean;
    status: number;
    headers: Headers;
    json: () => Promise<{ roads: unknown[] }>;
  }> {
    return {
      ok: true,
      status: 200,
      headers: new Headers(),
      json: async () => ({ roads: [] }),
    };
  },
  async getStatus(): Promise<{ hasKey: boolean }> {
    return { hasKey: false };
  },
  async fetchFlowForBounds(): Promise<unknown[]> {
    return [];
  },
  getFlowSessionStats(): { tilesFetched: number } {
    return { tilesFetched: 0 };
  },
  resetFlowTileCache(): void {
    // no cache to clear in the stub
  },
};

// ── Bikeshare (GBFS) ───────────────────────────────────────────────────────
// bikeshare/model.js:74 extractStationsArray reads `payload.data.stations`
// (also tolerates a bare array / nested value.stations). Empty stations → no
// markers, no throw.

export const bikeshare = {
  async getStations(): Promise<{ data: { stations: unknown[] } }> {
    return { data: { stations: [] } };
  },
};

// ── Military installations (OSM + Google Places) ───────────────────────────
// installations/ingestion.js:55-69 → payload.saturated (bool) and
// `payload.records.filter(...)`. The real source spreads
// normalizeMilitaryInstallations()'s `{ records, droppedCount }` and adds
// `status` + `saturated`. searchNearby (ingestion reads `payload.places`)
// returns `{ places: [] }`.

export const installations = {
  async getMappedSites(): Promise<{
    records: unknown[];
    droppedCount: number;
    status: "unavailable";
    saturated: boolean;
  }> {
    return {
      records: [],
      droppedCount: 0,
      status: "unavailable",
      saturated: false,
    };
  },
  async searchNearby(): Promise<{ places: unknown[] }> {
    return { places: [] };
  },
};

// ── Satellites (CelesTrak) ─────────────────────────────────────────────────
// satellites/ingestion.js:31 → `source.readGroup(path, { signal })` returns
// `{ ok, status, text }`; ingestion maps non-ok to an empty group
// (`{ entries: [], ok: false }`) without throwing. 501 explicitly marks the
// endpoint as not-yet-implemented by IntelHub.

export const satellites = {
  async readGroup(): Promise<{ ok: boolean; status: number; text: string }> {
    return { ok: false, status: 501, text: "" };
  },
};

// ── Transit (GTFS-realtime vehicles) ───────────────────────────────────────
// layers/transit/index.js:34-41 validates `requestSnapshot` + `getHistory`
// are functions — and without an explicit entry the catalog passes
// `sources.transit === undefined`, so the layer silently falls back to the
// engine's own createTransitSource() (source.js default parameter), which
// polls the same-origin UNauthenticated /api/transit/vehicles/<feed>. That is
// the exact failure T3 I-2 rules out: the stub keeps the layer enabled and
// honest instead.
// ingestion.js:439-452 reads `{ ok, status, headers, json() }` off the
// requestSnapshot result and takes `snapshot.vehicles || []` (line 205) from
// json(); a 503 + `{ vehicles: [] }` lands the feed in its ordinary
// "temporarily unavailable" state. trails.js:257 iterates `payload.epochs`
// from getHistory, so the degraded history is `{ epochs: [] }`.

export const transit = {
  async requestSnapshot(): Promise<{
    ok: boolean;
    status: number;
    headers: Headers;
    json: () => Promise<{ vehicles: unknown[] }>;
  }> {
    return {
      ok: false,
      status: 503,
      headers: new Headers(),
      json: async () => ({ vehicles: [] }),
    };
  },
  async getHistory(): Promise<{ epochs: unknown[] }> {
    return { epochs: [] };
  },
};
