// Pure mapping from the IntelHub P1 adsb snapshot to the GEV aircraft
// observation + envelope contract.
//
// The vendored engine's flights/military ingestion calls
// `source.getSnapshot(query, { signal })` and reads the envelope keys
// `status, observedAtMs, ageMs, stale, freshness, source, coverage` plus
// `records` (gev-engine/src/layers/flights/ingestion.js:35-53,
// layers/military/ingestion.js:34-52). Every record then goes straight into
// `records.receive(observation, …)` (flights/snapshotRenderer.js:53,
// military/index.js), so the record shape here IS the engine's observation
// shape — mirrors the engine's own readsb normalizer
// (gev-engine/src/sources/live/aircraft.js:31-74), whose upstream payload is
// the same adsb.lol row the hub collector ingests.
//
// Vocabulary is the engine's, not a private one (T3 review I-1):
//   * `freshness` ∈ "current" | "stale" | "unknown" — the values the engine
//     produces (aircraft.js:97-101, live/vessels.js:64-68) and tests for
//     (`snapshot.freshness === "unknown"` is the degraded signal).
//   * `observedAtMs` / `ageMs` are `null` when the snapshot has no usable
//     time (aircraft.js:90-95), never NaN/0.
//   * `status` is an HTTP-ish code: `feed._lastStatus = snapshot.status ?? 200`
//     (flights/ingestion.js:38) and it is surfaced verbatim in the layer's
//     status query (flights/queries.js:934).

import type { SnapshotEnvelope } from "./types";

/** Knots → m/s. Verbatim the engine's factor (aircraft.js:63). */
export const KNOTS_TO_MPS = 0.514444;

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

/** `null` for non-numeric/NaN JSON values, mirroring the engine's `finite()`. */
const finiteOrNull = (value: number | null | undefined): number | null =>
  typeof value === "number" && Number.isFinite(value) ? value : null;

/**
 * Map one hub row to an engine observation.
 *
 * `observedAtMs` is the snapshot epoch (`Date.parse(env.ts)`) or `null` when
 * the snapshot has no usable time; an unknown epoch yields unknown fix times
 * rather than times measured from 0.
 *
 * Two deliberate readings of the hub data, both matching engine behaviour:
 *   * `age_s` (time since the last transponder message) is the only age the
 *     hub publishes, so it is used as the position age and the snapshot epoch
 *     doubles as the contact time. The engine's `contactTimeMs` comes from the
 *     same underlying `seen` counter (aircraft.js:57-59).
 *   * `onGround` is `false`: the hub's `parse_ac_array` folds adsb.lol's
 *     `alt_baro: "ground"` to 0 m, and the engine's grounded path keys off
 *     `alt <= 0` in its own record policy.
 */
export function toGevRecord(
  p: AdsbPoint,
  observedAtMs: number | null,
): GevAircraftRecord {
  const ageMs = finiteOrNull(p.age_s);
  const speedKts = finiteOrNull(p.gs);
  return {
    id: p.hex.toLowerCase(),
    reference: p.hex,
    latitude: p.lat,
    longitude: p.lon,
    callsign: (p.flight ?? "").trim() || null,
    originCountry: null,
    positionTimeMs:
      observedAtMs == null || ageMs == null
        ? null
        : observedAtMs - Math.round(ageMs * 1000),
    contactTimeMs: observedAtMs,
    baroAltitudeM: finiteOrNull(p.alt_m),
    ellipsoidAltitudeM: null,
    onGround: false, // adsb.lol snapshot has no ground bit; the engine degrades on alt <= 0
    speedMps: speedKts == null ? null : speedKts * KNOTS_TO_MPS,
    courseDeg: finiteOrNull(p.track),
    verticalRateMps: null,
    category: null,
    typeCode: null,
    registration: null,
    operator: null,
  };
}

/** The hub envelope fields this mapping reads (see `aircraft-map.ts` header). */
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

/**
 * Parse the snapshot epoch from a hub envelope. Returns `null` for the hub's
 * degraded `{stale:true, aircraft:[]}` body (no `ts`) and for any unparseable
 * value, so `Date.parse(undefined)`'s NaN never reaches a record timestamp.
 */
export function snapshotEpochMs(env: AdsbEnvelope): number | null {
  const parsed = typeof env.ts === "string" ? Date.parse(env.ts) : Number.NaN;
  return Number.isFinite(parsed) ? parsed : null;
}

/**
 * Wrap mapped records in the GEV snapshot envelope.
 *
 * `complete`/`rejectedCount` describe admission, not freshness: the hub
 * collector already dropped unusable rows before publishing, so this adapter
 * admits every row it is given. Freshness travels in `stale`/`freshness`,
 * which is what the engine's backoff reads (flights/ingestion.js:41).
 */
export function toEnvelope(
  records: GevAircraftRecord[],
  env: AdsbEnvelope,
  source: string,
  nowMs: number,
): SnapshotEnvelope<GevAircraftRecord> {
  const observedAtMs = snapshotEpochMs(env);
  const ageMs = observedAtMs == null ? null : Math.max(0, nowMs - observedAtMs);
  const stale = env.stale === true;
  return {
    records,
    complete: true,
    rejectedCount: 0,
    source,
    coverage: env.coverage ?? null,
    observedAtMs,
    ageMs,
    stale,
    // "unknown" is the engine's own no-time value (aircraft.js:98) and it is
    // honoured as a degraded signal (`snapshot.stale || freshness === "unknown"`).
    freshness: stale ? "stale" : observedAtMs == null ? "unknown" : "current",
    status: stale ? 503 : 200,
  };
}
