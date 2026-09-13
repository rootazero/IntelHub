// SP9 Knowledge Graph Memory — typed wrappers for the 6 REST endpoints
// backing the /graph console page. The backend returns `entity_id`
// (UUID string); there is no exported `Uuid` type in this codebase, so
// we use plain `string` for IDs.

import { api } from "../api";

export interface GraphEdge {
  rel_type: string;
  /** SP10: real Neo4j endpoint IDs (was hard-coded `root→neighbor` in SP9). */
  source_id: string;
  target_id: string;
  valid_from?: string | null;
  valid_until?: string | null;
  confidence?: number | null;
}

export interface EntitySummary {
  entity_id: string;
  name: string;
  kind: string;
  edges: GraphEdge[];
}

export interface EntitySearchHit {
  entity_id: string;
  kind: string;
  name: string;
  aliases: unknown[];
  score: number;
}

export interface TimelineEntry {
  event_id?: string;
  occurred_at?: string;
  [key: string]: unknown;
}

export interface PathResult {
  names: string[];
  edges: Array<{ rel_type: string; confidence?: number }>;
  hops: number;
}

/// List entities for the /graph picker. Pass `q` to search by name (alias-aware),
/// or omit for the default landing view (most-recently-created entities).
/// `kind` optionally filters by kind string.
export async function listEntities(opts: {
  q?: string;
  kind?: string;
  limit?: number;
}): Promise<EntitySearchHit[]> {
  const params = new URLSearchParams();
  if (opts.q) params.set("q", opts.q);
  if (opts.kind) params.set("kind", opts.kind);
  if (opts.limit) params.set("limit", String(opts.limit));
  const qs = params.toString();
  return api<EntitySearchHit[]>(
    `/api/v1/graph/entities/search${qs ? `?${qs}` : ""}`,
  );
}

export async function getNeighbors(
  root: string,
  depth = 2,
  opts?: { atTime?: string; relTypes?: string[]; minConfidence?: number },
): Promise<EntitySummary[]> {
  const q = new URLSearchParams({ root, depth: String(depth) });
  if (opts?.atTime) q.set("at_time", opts.atTime);
  if (opts?.relTypes && opts.relTypes.length > 0) {
    q.set("rel_types", opts.relTypes.join(","));
  }
  if (opts?.minConfidence != null) {
    q.set("min_confidence", String(opts.minConfidence));
  }
  return api<EntitySummary[]>(`/api/v1/graph/neighbors?${q}`);
}

export async function getEntityTimeline(
  id: string,
  from?: string,
  to?: string,
): Promise<TimelineEntry[]> {
  const q = new URLSearchParams();
  if (from) q.set("from", from);
  if (to) q.set("to", to);
  return api<TimelineEntry[]>(`/api/v1/graph/entity/${id}/timeline?${q}`);
}

export async function findPath(
  from: string,
  to: string,
  maxHops = 5,
): Promise<PathResult> {
  const q = new URLSearchParams({ from, to, max_hops: String(maxHops) });
  return api<PathResult>(`/api/v1/graph/path?${q}`);
}

export async function getInvestigationGraph(
  id: string,
): Promise<{ nodes: unknown[]; edges: unknown[] }> {
  return api(`/api/v1/graph/investigation/${id}`);
}

export async function getEvidenceForEntity(
  entity: string,
  relation?: string,
): Promise<unknown[]> {
  const q = new URLSearchParams({ entity });
  if (relation) q.set("relation", relation);
  return api<unknown[]>(`/api/v1/graph/evidence?${q}`);
}