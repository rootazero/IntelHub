// SPDX-License-Identifier: proprietary
// GEV P8 — annotation engine mount adapter. Bridges the shared AnnotationSpec
// into the vendor `createAnnotationEngine` (drawTool-style manual specs), and
// keeps our own id → spec book so unmount-by-id works even though the vendor
// engine only exposes a whole-board `clear()`.

import { createAnnotationEngine } from "gev-engine/src/annotations/annotationEngine.js";
import { createHybridAnnotationRenderer } from "gev-engine/src/annotations/hybridAnnotationRenderer.js";
import type { AnnotationSpec } from "./draw-tool";

export type AnnotationEngineEvent =
  | { kind: "mount"; id: string; spec: AnnotationSpec }
  | { kind: "unmount"; id: string; spec?: AnnotationSpec };

export interface AnnotationEngineHandle {
  // Returns the engine-local (adapter-local) id for this mount.
  mount(spec: AnnotationSpec): string;
  unmount(id: string): void;
  list(): Array<{ id: string; spec: AnnotationSpec }>;
  subscribe(cb: (events: ReadonlyArray<AnnotationEngineEvent>) => void): () => void;
  destroy(): void;
}

// Minimal shape the vendor engine's renderer boot requires. The real
// createHybridAnnotationRenderer reaches into viewer.scene for the Cesium
// primitives; the adapter itself only needs the viewer to expose scene.canvas
// so the renderer can bind (and the TypeError below is the mount-time guard).
interface EngineViewer {
  scene?: { canvas?: unknown };
  [key: string]: unknown;
}

// vendor spec shape (drawMode.finishSpec / annotationEngine manual branch):
// `type` is 'route' for a line, geometry is `path`/`ring`/lat-lon.
function toVendorSpec(spec: AnnotationSpec) {
  const type = spec.shape === "line" ? "route" : spec.shape;
  return {
    type,
    manual: true,
    ring: spec.shape === "area" ? spec.vertices.map((v) => [v.lon, v.lat]) : undefined,
    path: spec.shape === "line" ? spec.vertices.map((v) => [v.lon, v.lat]) : undefined,
    latitude: spec.shape === "pin" ? spec.vertices[0]?.lat : undefined,
    longitude: spec.shape === "pin" ? spec.vertices[0]?.lon : undefined,
    label: spec.label ?? null,
    color: spec.color,
  };
}

export function mountAnnotationEngine(viewer: unknown): AnnotationEngineHandle {
  const v = viewer as EngineViewer | null | undefined;
  if (!v || typeof v.scene?.canvas === "undefined") {
    throw new TypeError("mountAnnotationEngine: viewer with scene.canvas required");
  }
  const renderer = createHybridAnnotationRenderer(v);
  const engine = createAnnotationEngine({
    viewer: v,
    renderer,
    placeSearch: undefined,
    resolveTarget: undefined,
  });

  let counter = 0;
  const mounted = new Map<string, AnnotationSpec>();
  const subs = new Set<(events: ReadonlyArray<AnnotationEngineEvent>) => void>();

  const emit = (events: ReadonlyArray<AnnotationEngineEvent>) => {
    subs.forEach((cb) => cb(events));
  };

  // Internal placement. `notify` is false for the clear-then-replay path so an
  // unmount does not manufacture a spurious "mount" event for every survivor.
  // `idOverride` preserves a survivor's adapter id across replay (stable ids).
  function place(spec: AnnotationSpec, notify: boolean, idOverride?: string): string {
    const id = idOverride ?? `p8-${++counter}`;
    // Fire-and-forget: the engine resolves manual specs without network and
    // reports failures in its result object (it never rejects), so the adapter
    // needs neither the result nor a rejection handler for this path.
    void engine.annotate([toVendorSpec(spec)], { persist: true, flyTo: false });
    mounted.set(id, spec);
    if (notify) emit([{ kind: "mount", id, spec }]);
    return id;
  }

  function unmount(id: string): void {
    const spec = mounted.get(id);
    if (!spec) return;
    // vendor engine has clear() (whole board) but NO per-id remove/unmount —
    // verified against annotationEngine.js (only `clear` + `destroy`). So we
    // clear all, drop the target, then replay the survivors (keeping their ids).
    // Acceptable for the board scale this adapter targets (≲500 marks); revisit
    // if that grows.
    engine.clear();
    mounted.delete(id);
    const remaining = Array.from(mounted.entries());
    mounted.clear();
    for (const [rid, rspec] of remaining) place(rspec, false, rid);
    emit([{ kind: "unmount", id, spec }]);
  }

  return {
    mount(spec) {
      return place(spec, true);
    },
    unmount,
    list() {
      return Array.from(mounted.entries()).map(([id, spec]) => ({ id, spec }));
    },
    subscribe(cb) {
      subs.add(cb);
      return () => {
        subs.delete(cb);
      };
    },
    destroy() {
      try {
        engine.clear();
      } catch {
        /* noop */
      }
      try {
        engine.destroy?.();
      } catch {
        /* noop */
      }
      mounted.clear();
      subs.clear();
    },
  };
}
