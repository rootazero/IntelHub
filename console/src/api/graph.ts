// SP9 Knowledge Graph Memory — typed wrappers for the 5 REST endpoints
// backing the /graph console page. The backend returns `entity_id`
// (UUID string); there is no exported `Uuid` type in this codebase, so
// we use plain `string` for IDs.

import { api } from "../api";

export interface GraphEdge {
  rel_type: string;
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

export async function getNeighbors(
  root: string,
  depth = 2,
  atTime?: string,
): Promise<EntitySummary[]> {
  const q = new URLSearchParams({ root, depth: String(depth) });
  if (atTime) q.set("at_time", atTime);
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