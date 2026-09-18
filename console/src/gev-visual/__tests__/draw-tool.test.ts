import { describe, expect, test, vi } from "vitest";

// Fake vendor drawMode replicating the REAL contract
// (gev-engine/src/annotations/drawMode.js):
//   * createDrawSession(shape) -> { shape: normalizeShape(shape), vertices: [] }
//   * addVertex(session, v) -> { added, reason? } (pushes a normalized vertex)
//   * finishSpec(session, {label,color}) -> vendor spec with `type:'route'` for
//     a line and `ring`/`path`/latitude+longitude — NOT `shape:'line'` /
//     `vertices`. An area ring is CLOSED (first point repeated at the end) via
//     closeRing, exactly like the real module.
// This shape fidelity is the lenient-mock guard: an adapter that expected the
// vendor to emit `type:'line'` + `vertices` would silently break against the
// real module, and these tests would catch the mismatch in the translation.
vi.mock("gev-engine/src/annotations/drawMode.js", () => {
  const normalizeShape = (shape: unknown) => {
    const s = String(shape ?? "").toLowerCase();
    if (s === "line" || s === "path" || s === "route") return "line";
    if (s === "pin" || s === "point" || s === "marker") return "pin";
    return "area";
  };
  const createDrawSession = (shape = "area") => ({
    shape: normalizeShape(shape),
    vertices: [] as Array<{ lon: number; lat: number; height?: number }>,
  });
  const addVertex = (session: any, vertex: any) => {
    if (!session || !Number.isFinite(vertex?.lon) || !Number.isFinite(vertex?.lat)) {
      return { added: false, reason: "invalid" };
    }
    session.vertices.push({ lon: vertex.lon, lat: vertex.lat, height: 0 });
    return { added: true };
  };
  const closeRing = (pairs: Array<[number, number]>) => {
    if (!Array.isArray(pairs) || pairs.length < 3) return pairs;
    const first = pairs[0];
    const last = pairs[pairs.length - 1];
    if (first[0] === last[0] && first[1] === last[1]) return pairs;
    return [...pairs, [first[0], first[1]]];
  };
  const finishSpec = (session: any, { label = "", color = "primary" } = {}) => {
    const text = String(label || "").trim();
    const pts = session.vertices.map((v: any) => [v.lon, v.lat] as [number, number]);
    if (session.shape === "area") {
      return { type: "area", manual: true, ring: closeRing(pts), label: text || null, color };
    }
    if (session.shape === "line") {
      return { type: "route", manual: true, path: pts, label: text || null, color };
    }
    const [lon, lat] = pts[0];
    return { type: "pin", manual: true, latitude: lat, longitude: lon, label: text || null, color };
  };
  return {
    DRAW_SHAPES: ["area", "line", "pin"],
    MIN_VERTICES: { area: 3, line: 2, pin: 1 },
    normalizeShape,
    createDrawSession,
    addVertex,
    finishSpec,
    closeRing,
  };
});

import { mountDrawTool } from "../annotations/draw-tool";

function fakeViewer() {
  return {
    scene: {
      pickPosition: vi.fn(() => ({ x: 0, y: 0, z: 0 })),
    },
  };
}

describe("mountDrawTool", () => {
  test("constructor throws TypeError on viewer without pickPosition", () => {
    expect(() => mountDrawTool({} as any)).toThrow(TypeError);
    expect(() => mountDrawTool({ scene: {} } as any)).toThrow(TypeError);
    expect(() => mountDrawTool({ scene: { pickPosition: "not-a-fn" } } as any)).toThrow(TypeError);
  });

  test("start(pin) creates a session with 0 vertices and state 'drawing'", () => {
    const h = mountDrawTool(fakeViewer());
    const previews: Array<Array<{ lon: number; lat: number }>> = [];
    const states: string[] = [];
    h.onPreview((v) => previews.push(v));
    h.onState((s) => states.push(s));
    h.start("pin");
    expect(states).toEqual(["drawing"]);
    expect(previews).toEqual([[]]);
  });

  test("start(line) + two addClickWorld calls fire onPreview with 2 vertices", () => {
    const h = mountDrawTool(fakeViewer());
    const previews: Array<Array<{ lon: number; lat: number }>> = [];
    h.onPreview((v) => previews.push(v));
    h.start("line");
    expect(h.addClickWorld(1, 2)).toBe(true);
    expect(h.addClickWorld(3, 4)).toBe(true);
    expect(previews[previews.length - 1]).toEqual([
      { lon: 1, lat: 2 },
      { lon: 3, lat: 4 },
    ]);
  });

  test("start(area) + 3 clicks + finish -> shape 'area' with 4 vertices (closing vertex kept)", () => {
    const h = mountDrawTool(fakeViewer());
    h.start("area");
    h.addClickWorld(0, 0);
    h.addClickWorld(1, 0);
    h.addClickWorld(0, 1);
    const spec = h.finish();
    expect(spec).not.toBeNull();
    expect(spec!.shape).toBe("area");
    // The vendor closes the ring (first point repeated at the end) because the
    // outline renderer draws one edge per consecutive pair — an open ring would
    // render with the closing side missing. The adapter must forward it verbatim.
    expect(spec!.vertices).toHaveLength(4);
    expect(spec!.vertices).toEqual([
      { lon: 0, lat: 0 },
      { lon: 1, lat: 0 },
      { lon: 0, lat: 1 },
      { lon: 0, lat: 0 },
    ]);
    expect(spec!.id).toBeTruthy();
  });

  test("cancel resets the session and emits state 'idle'", () => {
    const h = mountDrawTool(fakeViewer());
    const states: string[] = [];
    h.onState((s) => states.push(s));
    h.start("line");
    h.addClickWorld(1, 2);
    h.cancel();
    expect(states[states.length - 1]).toBe("idle");
    expect(h.finish()).toBeNull();
  });

  test("destroy is idempotent and disables the handle", () => {
    const h = mountDrawTool(fakeViewer());
    h.start("line");
    h.addClickWorld(1, 2);
    expect(() => h.destroy()).not.toThrow();
    expect(() => h.destroy()).not.toThrow();
    expect(h.addClickWorld(3, 4)).toBe(false);
    expect(h.finish()).toBeNull();
    expect(() => h.start("line")).toThrow();
  });
});
