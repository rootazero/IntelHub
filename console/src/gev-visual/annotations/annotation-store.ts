// SPDX-License-Identifier: proprietary
// GEV P8 — annotation REST store adapter. Thin fetch wrapper over the
// `/api/v1/annotations/*` endpoints (Task 2); translates the wire row shape
// into the shared AnnotationSpec.

import type { AnnotationSpec } from "./draw-tool";

export type ApiFetch = (input: string, init?: RequestInit) => Promise<Response>;

// Wire row shape from hub-core `db::annotations::AnnotationRow` (Serialize):
// `geometry` is `{vertices:[{lon,lat,height?}]}` — NOT a raw array.
interface ServerRow {
  id: string;
  agent_id: string | null;
  shape: string;
  label: string | null;
  color: string;
  geometry: { vertices: Array<{ lon: number; lat: number; height?: number }> };
  ttl_ms: number | null;
  meta: Record<string, unknown>;
  created_at: string;
  expires_at: string | null;
}

function rowToSpec(r: ServerRow): AnnotationSpec {
  return {
    id: r.id,
    shape: r.shape as AnnotationSpec["shape"],
    vertices: r.geometry.vertices,
    label: r.label ?? undefined,
    color: r.color as AnnotationSpec["color"],
    ttl_ms: r.ttl_ms ?? undefined,
    meta: r.meta,
  };
}

export interface AnnotationStore {
  list(opts?: { since?: string; bbox?: [number, number, number, number] }): Promise<AnnotationSpec[]>;
  create(spec: AnnotationSpec): Promise<AnnotationSpec>;
  patch(id: string, patch: { label?: string | null; color?: string; ttl_ms?: number | null }): Promise<AnnotationSpec>;
  remove(id: string): Promise<void>;
  get(id: string): Promise<AnnotationSpec>;
}

export function createAnnotationStore(apiFetch: ApiFetch): AnnotationStore {
  async function parseJsonOrThrow(resp: Response, verb: string): Promise<unknown> {
    if (!resp.ok) throw new Error(`${verb} failed: ${resp.status}`);
    return resp.json();
  }

  return {
    async list(opts) {
      const qs = new URLSearchParams();
      if (opts?.since) qs.set("since", opts.since);
      // hub takes `south,west,north,east` comma-separated floats.
      if (opts?.bbox) qs.set("bbox", opts.bbox.join(","));
      const query = qs.toString();
      const url = `/api/v1/annotations${query ? `?${query}` : ""}`;
      const body = (await parseJsonOrThrow(await apiFetch(url), "list")) as {
        annotations: ServerRow[];
      };
      return body.annotations.map(rowToSpec);
    },
    async create(spec) {
      const resp = await apiFetch("/api/v1/annotations", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          id: spec.id,
          shape: spec.shape,
          label: spec.label ?? null,
          color: spec.color,
          geometry: { vertices: spec.vertices },
          ttl_ms: spec.ttl_ms ?? null,
          meta: spec.meta ?? {},
        }),
      });
      return rowToSpec((await parseJsonOrThrow(resp, "create")) as ServerRow);
    },
    async patch(id, patch) {
      const resp = await apiFetch(`/api/v1/annotations/${id}`, {
        method: "PATCH",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(patch),
      });
      return rowToSpec((await parseJsonOrThrow(resp, "patch")) as ServerRow);
    },
    async remove(id) {
      const resp = await apiFetch(`/api/v1/annotations/${id}`, { method: "DELETE" });
      if (!resp.ok) throw new Error(`remove failed: ${resp.status}`);
    },
    async get(id) {
      const resp = await apiFetch(`/api/v1/annotations/${id}`);
      return rowToSpec((await parseJsonOrThrow(resp, "get")) as ServerRow);
    },
  };
}
