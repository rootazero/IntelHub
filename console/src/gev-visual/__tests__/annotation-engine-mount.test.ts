import { beforeEach, describe, expect, test, vi } from "vitest";
import { mountAnnotationEngine } from "../annotations/annotation-engine-mount";
import type { AnnotationSpec } from "../annotations/draw-tool";

// Shared harness state, hoisted so the vi.mock factories can read/write it
// without TDZ surprises.
const hoisted = vi.hoisted(() => ({
  annotateCalls: [] as Array<{ specs: any[]; opts: any }>,
  clearCalls: 0,
  destroyCalls: 0,
  engineOpts: null as any,
}));

// Fake renderer: the real createHybridAnnotationRenderer(viewer) bootstraps
// Cesium screen/world renderers off the viewer. The adapter only threads it
// into the engine opts, so the mock returns a minimal placeholder.
vi.mock("gev-engine/src/annotations/hybridAnnotationRenderer.js", () => ({
  createHybridAnnotationRenderer: (viewer: unknown) => ({ kind: "hybrid", viewer }),
}));

// Fake engine replicating the REAL contract
// (gev-engine/src/annotations/annotationEngine.js):
//   * createAnnotationEngine({viewer, renderer, placeSearch, resolveTarget})
//   * returns { annotate, clear, destroy, list, count, ... } — annotate takes
//     (requests[], opts) and returns a result object; clear() wipes the whole
//     board; there is NO per-id remove/unmount (the adapter must replay).
// The shape fidelity is the lenient-mock guard: an adapter that called
// engine.remove(id) would throw here (no such method) and fail the tests.
vi.mock("gev-engine/src/annotations/annotationEngine.js", () => ({
  createAnnotationEngine: (opts: any) => {
    hoisted.engineOpts = opts;
    return {
      annotate: (specs: any[], o: any) => {
        hoisted.annotateCalls.push({ specs, opts: o });
        return { ok: true, drawn: specs.length, ids: [], results: [] };
      },
      clear: () => {
        hoisted.clearCalls += 1;
      },
      destroy: () => {
        hoisted.destroyCalls += 1;
      },
      list: () => [],
    };
  },
}));

function fakeViewer() {
  return { scene: { canvas: { clientWidth: 800, clientHeight: 600 } } };
}

function makeSpec(overrides: Partial<AnnotationSpec> = {}): AnnotationSpec {
  return {
    id: "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee",
    shape: "line",
    vertices: [
      { lon: 0, lat: 0 },
      { lon: 1, lat: 1 },
    ],
    label: "Route A",
    color: "cyan",
    ...overrides,
  };
}

beforeEach(() => {
  hoisted.annotateCalls = [];
  hoisted.clearCalls = 0;
  hoisted.destroyCalls = 0;
  hoisted.engineOpts = null;
});

describe("mountAnnotationEngine", () => {
  test("constructor rejects a viewer without scene.canvas", () => {
    expect(() => mountAnnotationEngine(undefined as any)).toThrow(TypeError);
    expect(() => mountAnnotationEngine({} as any)).toThrow(TypeError);
    expect(() => mountAnnotationEngine({ scene: {} } as any)).toThrow(TypeError);
  });

  test("mount returns an adapter id and threads a manual vendor spec", () => {
    const viewer = fakeViewer();
    const h = mountAnnotationEngine(viewer);
    const id = h.mount(makeSpec());
    expect(id).toMatch(/^p8-\d+$/);
    expect(hoisted.annotateCalls).toHaveLength(1);
    const { specs, opts } = hoisted.annotateCalls[0];
    expect(specs).toHaveLength(1);
    // line -> type 'route' + path (vendor naming, not `shape:'line'`+`vertices`)
    expect(specs[0].type).toBe("route");
    expect(specs[0].manual).toBe(true);
    expect(specs[0].path).toEqual([
      [0, 0],
      [1, 1],
    ]);
    expect(specs[0].label).toBe("Route A");
    expect(specs[0].color).toBe("cyan");
    expect(opts).toEqual({ persist: true, flyTo: false });
    // engine created with renderer + both optional resolvers undefined.
    expect(hoisted.engineOpts.viewer).toBe(viewer);
    expect(hoisted.engineOpts.renderer).toBeDefined();
    expect(hoisted.engineOpts.placeSearch).toBeUndefined();
    expect(hoisted.engineOpts.resolveTarget).toBeUndefined();
  });

  test("list reflects mounted specs", () => {
    const h = mountAnnotationEngine(fakeViewer());
    const specA = makeSpec();
    const specB = makeSpec({ shape: "pin", vertices: [{ lon: 9, lat: 9 }] });
    const idA = h.mount(specA);
    const idB = h.mount(specB);
    const list = h.list();
    expect(list).toHaveLength(2);
    expect(list.map((e) => e.id)).toEqual([idA, idB]);
    expect(list.find((e) => e.id === idA)?.spec).toBe(specA);
  });

  test("unmount removes the target and replays the survivors via clear()", () => {
    const h = mountAnnotationEngine(fakeViewer());
    const idA = h.mount(makeSpec());
    const idB = h.mount(makeSpec({ shape: "pin", vertices: [{ lon: 9, lat: 9 }] }));
    h.unmount(idA);
    const list = h.list();
    expect(list).toHaveLength(1);
    expect(list[0].id).toBe(idB);
    // vendor has no per-id remove -> whole-board clear, then replay.
    expect(hoisted.clearCalls).toBe(1);
  });

  test("subscribe fires mount then unmount events and unsubscribes", () => {
    const h = mountAnnotationEngine(fakeViewer());
    const events: any[] = [];
    const off = h.subscribe((evs) => events.push(...evs));
    const id = h.mount(makeSpec());
    expect(events).toHaveLength(1);
    expect(events[0].kind).toBe("mount");
    expect(events[0].id).toBe(id);
    h.unmount(id);
    expect(events).toHaveLength(2);
    expect(events[1].kind).toBe("unmount");
    expect(events[1].id).toBe(id);
    off();
    h.mount(makeSpec());
    expect(events).toHaveLength(2);
  });

  test("destroy is idempotent and clears the engine", () => {
    const h = mountAnnotationEngine(fakeViewer());
    h.mount(makeSpec());
    expect(() => h.destroy()).not.toThrow();
    expect(() => h.destroy()).not.toThrow();
    expect(hoisted.clearCalls).toBeGreaterThanOrEqual(1);
    expect(hoisted.destroyCalls).toBeGreaterThanOrEqual(1);
    expect(h.list()).toHaveLength(0);
  });
});
