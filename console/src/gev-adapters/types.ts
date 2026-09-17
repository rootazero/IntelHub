// Shared adapter contracts for the IntelHub ↔ GEV layer-source boundary.
//
// Shapes here mirror what the vendored engine's per-layer ingestion actually
// destructures. The canonical method list lives in
// `gev-engine/src/app/constructCatalog.js` (SOURCE_METHODS); T12 adds a runtime
// contract guard that introspects that list, so keep these types aligned with
// the engine rather than with this comment.
//
// Vocabulary note (T3 review I-1): the values below are the engine's own, not a
// private IntelHub vocabulary. The engine produces `current`/`stale`/`unknown`
// freshness (sources/live/aircraft.js:97-101, sources/live/vessels.js:64-68)
// and treats `freshness === "unknown"` as a degraded signal
// (layers/flights/ingestion.js:41, layers/vessels/ingestion.js:137). Likewise
// `observedAtMs`/`ageMs` are `null` when the snapshot time is unknown
// (aircraft.js:90-95) — never NaN, never 0.

/** Snapshot age state, exactly the engine's vocabulary. */
export type SnapshotFreshness = "current" | "stale" | "unknown";

/**
 * HTTP-ish status code for the snapshot, not a private enum: the engine stores
 * it in `feed._lastStatus = snapshot.status ?? 200`
 * (layers/flights/ingestion.js:38, layers/military/ingestion.js:37) and surfaces
 * it verbatim in the layer status query (layers/flights/queries.js:934).
 */
export type SnapshotStatus = number;

/**
 * AIS-style snapshot envelope consumed by the aircraft/vessel feed layers.
 *
 * Read by `layers/vessels/ingestion.js` (records, source, observedAtMs,
 * freshness, complete, stale — its own transport state arrives as
 * `transportStatus`) and by
 * `layers/flights|military/ingestion.js` (status, observedAtMs, ageMs, stale,
 * freshness, source, coverage) plus the snapshot renderers (records,
 * observedAtMs). Only the fields the engine reads are required here; extra
 * AIS diagnostics (rawRowCount, transportStatus, lastMessageAt, …) are
 * optional because the degraded stub does not produce them.
 */
export interface SnapshotEnvelope<T = unknown> {
  records: T[];
  complete: boolean;
  rejectedCount: number;
  source: string;
  coverage?: unknown;
  /** Snapshot epoch; `null` when the source has no usable time. */
  observedAtMs: number | null;
  /** `now - observedAtMs`, clamped at 0; `null` when `observedAtMs` is null. */
  ageMs: number | null;
  stale: boolean;
  freshness: SnapshotFreshness;
  status: SnapshotStatus;
}

/**
 * One row of `GET /api/v1/globe/aircraft` (`aircraft[]`), per
 * hub-core monitor/sources/adsb.rs `snapshot_envelope()`.
 *
 * `lat`/`lon` are non-null by construction — the collector's `parse_ac_array`
 * drops rows without them. `flight`/`gs`/`track`/`squawk` are Option-derived,
 * so they really are JSON null when adsb.lol omits them; typing them
 * non-null is how a `null * KNOTS_TO_MPS === 0` regression sneaks in.
 *
 * `seen` is listed for parity with the raw adsb.lol row, but the hub envelope
 * does not carry it (it publishes the derived `age_s` instead).
 */
export interface AdsbPoint {
  hex: string;
  flight?: string | null;
  lat: number;
  lon: number;
  alt_m?: number | null;
  /** Ground speed in knots. */
  gs?: number | null;
  track?: number | null;
  squawk?: string | null;
  mil?: boolean;
  seen?: number;
  /** Age of the observation in seconds, relative to the snapshot `ts`. */
  age_s?: number | null;
}

/** Engine observation shape, as destructured by flights/military records.js. */
export interface GevAircraftRecord {
  id: string;
  reference: string;
  latitude: number;
  longitude: number;
  callsign: string | null;
  originCountry: string | null;
  positionTimeMs: number | null;
  contactTimeMs: number | null;
  baroAltitudeM: number | null;
  ellipsoidAltitudeM: number | null;
  onGround: boolean;
  speedMps: number | null;
  courseDeg: number | null;
  verticalRateMps: number | null;
  category: number | null;
  typeCode: string | null;
  registration: string | null;
  operator: string | null;
}

/** The hub envelope fields the aircraft mapping reads (see `aircraft-map.ts`). */
export interface AdsbEnvelope {
  /** RFC3339 snapshot epoch. Absent in the hub's degraded `{stale:true}` body. */
  ts?: string | null;
  count?: number;
  coverage?: unknown;
  cycle_secs?: number;
  last_tick?: string;
  aircraft?: AdsbPoint[];
  /** Present (true) when the hub had no live snapshot to serve. */
  stale?: boolean;
}

/** One row of `GET /api/v1/gev/earthquakes` (see hub-core api.rs `quake_row`). */
export interface EarthquakeRow {
  /** USGS feature id, used by the engine as the Cesium entity key. */
  stableId: string;
  /** Same USGS feature id, surfaced as the analyst-facing id seam. */
  usgsId: string;
  lon: number;
  lat: number;
  /** Null for every USGS row: depth lives in GeoJSON geometry, not properties. */
  depthKm: number | null;
  mag: number | null;
  place: string | null;
  /** USGS epoch milliseconds, never an ISO string. */
  time: number | null;
}

/**
 * Return shape of the satellites source's `readGroup` — exactly what the
 * engine's satellites ingestion destructures (it maps non-ok to an empty
 * group without throwing, so transport failures resolve, not reject).
 */
export interface ReadGroupResult {
  ok: boolean;
  status: number;
  text: string;
}

/** Contract-valid "enabled but empty" snapshot: no records, unavailable state. */
export const emptyEnvelope = <T = unknown>(
  source: string,
): SnapshotEnvelope<T> => ({
  records: [],
  complete: false,
  rejectedCount: 0,
  source,
  observedAtMs: null,
  ageMs: null,
  stale: true,
  // "unknown" (not "unavailable") is what the engine reads as degraded; the
  // stub renders empty without claiming a time it does not have.
  freshness: "unknown",
  status: 503, // Service Unavailable — the stub has no upstream behind it
});
