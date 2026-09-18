// SPDX-License-Identifier: proprietary
// GEV P8 — manual drawing tool adapter. React-shell seam over vendor drawMode
// pure functions; DOM/canvas click integration lives in HudDrawToolbar.tsx.

import {
  createDrawSession,
  addVertex as vendorAddVertex,
  finishSpec as vendorFinishSpec,
  normalizeShape,
} from "gev-engine/src/annotations/drawMode.js";

export type DrawMode = "pin" | "line" | "area" | null;

export interface AnnotationSpec {
  id: string;
  shape: "pin" | "line" | "area";
  vertices: Array<{ lon: number; lat: number; height?: number }>;
  label?: string;
  color: "primary" | "amber" | "cyan" | "green" | "red";
  ttl_ms?: number;
  meta?: Record<string, unknown>;
}

export interface DrawToolHandle {
  start(mode: "pin" | "line" | "area"): void;
  // Returns false when there is no active session (or the tool is destroyed),
  // or when the coordinate is not a finite lon/lat on the globe.
  addClickWorld(lon: number, lat: number): boolean;
  finish(opts?: { label?: string; persist?: boolean }): AnnotationSpec | null;
  cancel(): void;
  onPreview(cb: (vertices: Array<{ lon: number; lat: number }>) => void): () => void;
  onState(cb: (state: "idle" | "drawing" | "finishing") => void): () => void;
  destroy(): void;
}

// Vendor `createDrawSession` / `addVertex` mutate a session of this shape.
interface VendorSession {
  shape: "pin" | "line" | "area";
  vertices: Array<{ lon: number; lat: number; height?: number }>;
}

// Vendor `finishSpec(session, {label,color})` return shape (drawMode.js).
// NOTE: vendor uses `type: 'route'` for a line and `path`/`ring`/lat-lon —
// this adapter is the single translation point into AnnotationSpec.
interface VendorSpec {
  type: "pin" | "area" | "route";
  manual: true;
  ring?: Array<[number, number]>;
  path?: Array<[number, number]>;
  latitude?: number;
  longitude?: number;
  label: string | null;
  color: string;
}

// The same pick-surface shape vendor's own drawTool relies on (via
// pickWorldFromScreen): `scene.pickPosition(Cartesian2)` → Cartesian3-like.
// We never call it here — the HUD converts screen → world and calls
// addClickWorld() with ready lon/lat — but we require the capability to exist
// so the shell cannot silently mount on a viewer that cannot pick at all.
export interface PickableViewer {
  scene: {
    pickPosition: (win: { x: number; y: number }) => { x: number; y: number; z: number } | undefined;
  };
}

const MIN_LON = -180;
const MAX_LON = 180;
const MIN_LAT = -90;
const MAX_LAT = 90;

// Mirrors drawMode.isFiniteCoordinate — finite and on the globe.
function isFiniteCoord(lon: number, lat: number): boolean {
  return (
    Number.isFinite(lon) &&
    Number.isFinite(lat) &&
    lon >= MIN_LON &&
    lon <= MAX_LON &&
    lat >= MIN_LAT &&
    lat <= MAX_LAT
  );
}

// Mirrors drawMode.MIN_VERTICES (stable contract).
const MIN_VERTICES: Record<"pin" | "line" | "area", number> = {
  area: 3,
  line: 2,
  pin: 1,
};

// vendor `finishSpec` closes an area ring (first point repeated at the end).
// AnnotationSpec.vertices are DISTINCT vertices — the store's pinned
// `{vertices:[{lon,lat}]}` contract — so the closing duplicate is stripped
// here, in one place, before the spec leaves the adapter.
function dedupeClosedRing(pairs: Array<[number, number]>): Array<{ lon: number; lat: number }> {
  if (pairs.length < 2) return pairs.map(([lon, lat]) => ({ lon, lat }));
  const first = pairs[0];
  const last = pairs[pairs.length - 1];
  const closed = first[0] === last[0] && first[1] === last[1];
  const effective = closed ? pairs.slice(0, -1) : pairs;
  return effective.map(([lon, lat]) => ({ lon, lat }));
}

const SHAPE_MAP: Record<VendorSpec["type"], AnnotationSpec["shape"]> = {
  pin: "pin",
  area: "area",
  route: "line",
};

export function mountDrawTool(viewer: PickableViewer): DrawToolHandle {
  if (!viewer?.scene?.pickPosition || typeof viewer.scene.pickPosition !== "function") {
    throw new TypeError("mountDrawTool: viewer.scene.pickPosition is required");
  }

  let session: VendorSession | null = null;
  let destroyed = false;
  const previewSubs = new Set<(v: Array<{ lon: number; lat: number }>) => void>();
  const stateSubs = new Set<(s: "idle" | "drawing" | "finishing") => void>();

  const emitPreview = () => {
    const verts = session?.vertices.map((v) => ({ lon: v.lon, lat: v.lat })) ?? [];
    previewSubs.forEach((cb) => cb(verts));
  };
  const emitState = (s: "idle" | "drawing" | "finishing") => {
    stateSubs.forEach((cb) => cb(s));
  };
  const canFinish = (shape: "pin" | "line" | "area", n: number): boolean =>
    n >= MIN_VERTICES[shape];

  return {
    start(mode) {
      if (destroyed) throw new Error("draw-tool destroyed");
      // normalizeShape coerces unknown/empty input to 'area' (vendor contract);
      // DrawMode only ever carries the three real shapes here.
      const m = normalizeShape(mode) as "pin" | "line" | "area";
      session = createDrawSession(m) as unknown as VendorSession;
      emitPreview();
      emitState("drawing");
    },
    addClickWorld(lon, lat) {
      if (!session || destroyed) return false;
      if (!isFiniteCoord(lon, lat)) return false;
      vendorAddVertex(session, { lon, lat, height: 0 });
      emitPreview();
      return true;
    },
    finish(opts) {
      if (!session || destroyed) return null;
      if (!canFinish(session.shape, session.vertices.length)) return null;
      emitState("finishing");
      const vendorSpec = vendorFinishSpec(session, {
        label: opts?.label ?? "",
        color: "primary",
      }) as VendorSpec | null;
      if (!vendorSpec) {
        // vendor refused (e.g. a degenerate shape) — keep the session so the
        // user can add/adjust vertices rather than silently dropping it.
        emitState("drawing");
        return null;
      }
      const shape = SHAPE_MAP[vendorSpec.type];
      let vertices: Array<{ lon: number; lat: number }>;
      if (shape === "pin") {
        vertices = [{ lon: vendorSpec.longitude ?? 0, lat: vendorSpec.latitude ?? 0 }];
      } else if (shape === "line") {
        vertices = (vendorSpec.path ?? []).map(([lon, lat]) => ({ lon, lat }));
      } else {
        vertices = dedupeClosedRing(vendorSpec.ring ?? []);
      }
      const spec: AnnotationSpec = {
        id: crypto.randomUUID(),
        shape,
        vertices,
        label: vendorSpec.label ?? undefined,
        color: "primary",
      };
      session = null;
      emitPreview();
      emitState("idle");
      return spec;
    },
    cancel() {
      if (destroyed) return;
      session = null;
      emitPreview();
      emitState("idle");
    },
    onPreview(cb) {
      previewSubs.add(cb);
      return () => {
        previewSubs.delete(cb);
      };
    },
    onState(cb) {
      stateSubs.add(cb);
      return () => {
        stateSubs.delete(cb);
      };
    },
    destroy() {
      if (destroyed) return;
      destroyed = true;
      session = null;
      previewSubs.clear();
      stateSubs.clear();
    },
  };
}
