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
