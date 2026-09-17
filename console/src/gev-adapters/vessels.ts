// Real (GEV P3 T2) vessels layer source: the IntelHub AIS live snapshot.
//
// Replaces the T3 `stubs.vessels` entry. Transport is the injected
// authenticated `apiFetch` (see ./http.ts) hitting the hub REST endpoints:
//
//   GET /api/v1/gev/ais-live?maxRows=N   → Redis snapshot envelope
//     { rows: [{mmsi, lat, lon, name, imo, type_specific, destination,
//               speed(knots!), course, heading, last_position_epoch(秒)}],
//       newestPositionAt(ISO), refreshing, status,
//       lastMessageAt, nextAttemptAt(ms), silentForMs, reconnectAttempt }
//   GET /api/v1/gev/ais-live/track?mmsi= → {samples:[{lat,lon,t(秒)}]}
//
// Row→record mapping and envelope→snapshot mapping are a TypeScript port of
// the engine's own standalone source (gev-engine/src/sources/live/vessels.js
// normalizeVesselObservation / vesselSnapshot / normalizeVesselTrack) plus
// the contract helpers in gev-engine/src/sources/live/contract.js — the
// adapter must emit byte-identical semantics because the vendor layer code
// is what consumes it (ingestion.js:41-59 destructures the snapshot;
// tracking.js:124-133 reads only track.records[].latitude/longitude).
//
// Degraded states RESOLVE (never throw): the engine's ingestion catch maps a
// thrown error to markUnavailable, but the hub's degraded envelopes
// (missing-key / connecting / 502) carry meaningful transportStatus values
// the layer renders as chips — surfacing them requires a resolved snapshot.

import type { ApiFetch } from "./http";
import type { SnapshotEnvelope, SnapshotFreshness } from "./types";

/** Engine knot→m/s factor (sources/live/vessels.js:15). */
export const KNOTS_TO_MPS = 0.514444;

const SOURCE_LABEL = "AISStream via IntelHub";
const COVERAGE = "received AIS positions";
const DEFAULT_MAX_ROWS = 12000;
/** Engine vessels/ingestion.js REFRESH_MS cadence is 60s with a 10s hard
 * timeout — the fetch just forwards whatever `signal` the layer combined. */

/** One row of the hub `/api/v1/gev/ais-live` envelope (ais.rs
 * `vessels_envelope`, field-for-field contracts.md §1). */
export interface AisRow {
  mmsi?: string | number | null;
  input_identifier?: string | null;
  lat?: number | null;
  lon?: number | null;
  name?: string | null;
  input_name?: string | null;
  imo?: string | number | null;
  type_specific?: string | null;
  type?: string | null;
  destination?: string | null;
  /** Knots — upstream units, converted here. */
  speed?: number | null;
  course?: number | null;
  heading?: number | null;
  /** Unix seconds. */
  last_position_epoch?: number | null;
  last_position_UTC?: string | null;
}

export interface AisLiveEnvelope {
  rows?: unknown;
  newestPositionAt?: string | number | null;
  refreshing?: boolean;
  status?: unknown;
  lastMessageAt?: string | number | null;
  nextAttemptAt?: number | null;
  silentForMs?: number | null;
  reconnectAttempt?: number | null;
}

/** Engine observation shape (sources/live/vessels.js:4-26). */
export interface VesselRecord {
  id: string;
  reference: string;
  latitude: number;
  longitude: number;
  name: string;
  imo: string;
  type: string;
  destination: string;
  speedMps: number | null;
  courseDeg: number | null;
  headingDeg: number | null;
  observedAtMs: number | null;
  altitudeDatum: "sea-surface";
}

/**
 * Snapshot the vessels ingestion destructures (ingestion.js:41-59): the
 * shared SnapshotEnvelope keys plus the AIS transport diagnostics, which the
 * layer maps to its feed state (`status: snapshot.transportStatus`,
 * `refreshing: snapshot.stale`, `rawRowCount`, …).
 */
export interface VesselsSnapshot extends SnapshotEnvelope<VesselRecord> {
  transportStatus: string | null;
  lastMessageAt: number | string | null;
  nextAttemptAt: number | null;
  silentForMs: number | null;
  reconnectAttempt: number | null;
  rawRowCount: number;
}

/** contract.js `finite`: null for null/''/boolean/non-finite, else Number(v). */
function finite(value: unknown): number | null {
  if (value == null || value === "" || typeof value === "boolean") return null;
  const number = Number(value);
  return Number.isFinite(number) ? number : null;
}

/** contract.js `epoch`: positive, within the Date range, scaled. */
function epoch(value: unknown, scale = 1): number | null {
  const number = finite(value);
  const milliseconds = number == null ? null : number * scale;
  return milliseconds != null && milliseconds > 0 && milliseconds <= 8640000000000000
    ? milliseconds
    : null;
}

function coordinates(latitude: number | null, longitude: number | null): boolean {
  return (
    latitude !== null &&
    longitude !== null &&
    Math.abs(latitude) <= 90 &&
    Math.abs(longitude) <= 180
  );
}

/**
 * TS port of `normalizeVesselObservation` (sources/live/vessels.js:4-26):
 * rows without an id or valid coordinates are DROPPED (the hub already
 * guarantees both, but the contract says the client degrades, never trusts).
 */
export function normalizeVesselObservation(row: AisRow): VesselRecord | null {
  const id = String(row?.mmsi || row?.input_identifier || "").trim();
  const latitude = finite(row?.lat);
  const longitude = finite(row?.lon);
  if (!id || !coordinates(latitude, longitude)) return null;
  const speedKts = finite(row.speed);
  const parsedUtc = row.last_position_UTC ? Date.parse(row.last_position_UTC) : NaN;
  return {
    id,
    reference: id,
    latitude: latitude as number,
    longitude: longitude as number,
    name: String(row.name || row.input_name || id),
    imo: String(row.imo || ""),
    type: String(row.type_specific || row.type || ""),
    destination: String(row.destination || ""),
    speedMps: speedKts == null ? null : speedKts * KNOTS_TO_MPS,
    courseDeg: finite(row.course),
    headingDeg: finite(row.heading),
    observedAtMs: epoch(row.last_position_epoch, 1000) ?? epoch(parsedUtc),
    altitudeDatum: "sea-surface",
  };
}

/** TS port of `normalizeVesselTrack` (sources/live/vessels.js:81-95). */
export function normalizeVesselTrack(samples: unknown): Array<{
  latitude: number;
  longitude: number;
  observedAtMs: number | null;
  altitudeDatum: "sea-surface";
}> {
  if (!Array.isArray(samples)) return [];
  return samples.flatMap((sample) => {
    const latitude = finite((sample as { lat?: unknown })?.lat);
    const longitude = finite((sample as { lon?: unknown })?.lon);
    if (!coordinates(latitude, longitude)) return [];
    return [
      {
        latitude: latitude as number,
        longitude: longitude as number,
        observedAtMs: epoch((sample as { t?: unknown })?.t, 1000),
        altitudeDatum: "sea-surface" as const,
      },
    ];
  });
}

/** Degraded-but-resolved snapshot (the stub's contract, now with transport). */
function degradedSnapshot(
  status: number,
  transportStatus: string | null,
  nowMs: number,
): VesselsSnapshot {
  return {
    records: [],
    complete: false,
    rejectedCount: 0,
    source: SOURCE_LABEL,
    coverage: COVERAGE,
    observedAtMs: null,
    ageMs: null,
    stale: true,
    freshness: "unknown",
    status,
    transportStatus,
    lastMessageAt: null,
    nextAttemptAt: null,
    silentForMs: null,
    reconnectAttempt: null,
    rawRowCount: 0,
  };
}

/**
 * TS port of `vesselSnapshot` (sources/live/vessels.js:29-78). `payload` is
 * trusted to carry a `rows` array — callers (getSnapshot) validate that
 * first; a malformed body is degraded before reaching here.
 */
export function toVesselsSnapshot(
  payload: AisLiveEnvelope,
  httpStatus: number,
  nowMs: number,
): VesselsSnapshot {
  const rows = payload.rows as AisRow[];
  const records: VesselRecord[] = [];
  const ids = new Set<string>();
  let rejected = 0;
  for (const row of rows) {
    const record = normalizeVesselObservation(row);
    if (record && !ids.has(record.id)) {
      records.push(record);
      ids.add(record.id);
    } else {
      rejected += 1;
    }
  }
  const newestIso = payload.newestPositionAt;
  const parsedNewest =
    typeof newestIso === "string" ? Date.parse(newestIso) : NaN;
  const observedAtMs =
    epoch(newestIso) ??
    epoch(parsedNewest) ??
    records.reduce<number | null>(
      (newest, row) => Math.max(newest ?? 0, row.observedAtMs ?? 0) || null,
      null,
    );
  // Ruling 8: freshness derives from the envelope's refreshing flag and
  // timestamps — the adapter does not age snapshots on its own (unlike the
  // flights layer's 120s rule; the AIS envelope IS the transport state).
  const refreshing = Boolean(payload?.refreshing);
  const freshness: SnapshotFreshness = refreshing
    ? "stale"
    : observedAtMs == null
      ? "unknown"
      : "current";
  return {
    records,
    source: SOURCE_LABEL,
    coverage: COVERAGE,
    complete: records.length === rows.length && !refreshing,
    rejectedCount: rejected,
    observedAtMs,
    ageMs: observedAtMs == null ? null : Math.max(0, nowMs - observedAtMs),
    stale: refreshing,
    freshness,
    status: httpStatus,
    transportStatus:
      typeof payload?.status === "string" ? payload.status : null,
    lastMessageAt: payload?.lastMessageAt ?? null,
    nextAttemptAt: finite(payload?.nextAttemptAt),
    silentForMs: finite(payload?.silentForMs),
    reconnectAttempt: finite(payload?.reconnectAttempt),
    rawRowCount: rows.length,
  };
}

export interface VesselsSource {
  label: string;
  getSnapshot(
    query?: unknown,
    options?: { signal?: AbortSignal },
  ): Promise<VesselsSnapshot>;
  getTrack(
    reference: string,
    options?: { signal?: AbortSignal },
  ): Promise<{ records: ReturnType<typeof normalizeVesselTrack>; complete: boolean }>;
}

export const vesselsSource = (apiFetch: ApiFetch): VesselsSource => ({
  label: SOURCE_LABEL,

  async getSnapshot(query?: unknown, { signal }: { signal?: AbortSignal } = {}) {
    const maxRows = finite((query as { maxRows?: unknown })?.maxRows) ?? DEFAULT_MAX_ROWS;
    const res = await apiFetch(`/api/v1/gev/ais-live?maxRows=${maxRows}`, {
      signal,
    });
    // Body may be absent/unparseable on a proxy-level failure — degrade, and
    // still mine a string `status` field out of it when present (the hub's
    // non-200 bodies carry {error}, not {status}, but a future gateway could).
    let body: unknown = null;
    try {
      body = await res.json();
    } catch {
      body = null;
    }
    const envelopeStatus =
      typeof (body as AisLiveEnvelope | null)?.status === "string"
        ? ((body as AisLiveEnvelope).status as string)
        : null;
    if (!res.ok) {
      return degradedSnapshot(res.status, envelopeStatus, Date.now());
    }
    if (!body || !Array.isArray((body as AisLiveEnvelope).rows)) {
      // Malformed 200 (proxy page, hub bug): same degraded-by-contract
      // framing the stub produced — 503, empty records, unknown freshness.
      return degradedSnapshot(503, envelopeStatus, Date.now());
    }
    return toVesselsSnapshot(body as AisLiveEnvelope, res.status, Date.now());
  },

  async getTrack(reference: string, { signal }: { signal?: AbortSignal } = {}) {
    const res = await apiFetch(
      `/api/v1/gev/ais-live/track?mmsi=${encodeURIComponent(reference)}`,
      { signal },
    );
    // tracking.js:124-133 swallows any failure and keeps the live-only
    // trail, so throwing here is safe AND matches the vendor source.
    if (!res.ok) throw new Error(`IntelHub vessels HTTP ${res.status}`);
    let body: { samples?: unknown } = {};
    try {
      body = await res.json();
    } catch {
      body = {};
    }
    return { records: normalizeVesselTrack(body?.samples), complete: false };
  },
});
