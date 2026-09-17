// Shared adapter contracts for the IntelHub ↔ GEV layer-source boundary.
//
// Shapes here mirror what the vendored engine's per-layer ingestion actually
// destructures. The canonical method list lives in
// `gev-engine/src/app/constructCatalog.js` (SOURCE_METHODS); T12 adds a runtime
// contract guard that introspects that list, so keep these types aligned with
// the engine rather than with this comment.

export type SnapshotFreshness = "live" | "stale" | "unavailable";
export type SnapshotStatus = "ok" | "degraded" | "unavailable";

/**
 * AIS-style snapshot envelope consumed by the aircraft/vessel feed layers.
 *
 * Read by `layers/vessels/ingestion.js` (records, source, observedAtMs,
 * freshness, complete, stale, status) and by
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
  observedAtMs: number;
  ageMs: number;
  stale: boolean;
  freshness: SnapshotFreshness | "unknown";
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
  observedAtMs: 0,
  ageMs: 0,
  stale: true,
  freshness: "unavailable",
  status: "unavailable",
});
