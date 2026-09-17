// Explicit vitest imports (repo convention, cf. ./vessels.test.ts).
import { expect, test, vi } from "vitest";
import type { ApiFetch } from "../http";
import { createIntelHubLayerSources } from "../index";
import { rewriteTrafficPath, trafficSource } from "../traffic";

const jsonResponse = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), { status });

test("rewriteTrafficPath maps the three engine endpoint families", () => {
  expect(rewriteTrafficPath("/api/overpass")).toBe("/api/v1/gev/overpass");
  expect(rewriteTrafficPath("/api/tomtom/status")).toBe("/api/v1/gev/tomtom/status");
  expect(rewriteTrafficPath("/api/tomtom/flow/12/654/1583.pbf")).toBe(
    "/api/v1/gev/tomtom/flow/12/654/1583.pbf",
  );
  // non-traffic paths pass through untouched
  expect(rewriteTrafficPath("/api/cctv/sources")).toBe("/api/cctv/sources");
  // prefix order matters: /api/tomtom/ must not half-match
  expect(rewriteTrafficPath("/api/tomtomX")).toBe("/api/tomtomX");
});

test("requestRoads POSTs form-encoded QL to the rewritten hub route", async () => {
  const calls: Array<{ path: string; init?: RequestInit }> = [];
  const apiFetch: ApiFetch = async (path, init) => {
    calls.push({ path: String(path), init });
    // hub proxies Overpass verbatim; engine wrapper's json() normalizes
    return jsonResponse({ elements: [] });
  };
  const src = trafficSource(apiFetch);
  const res = await src.requestRoads({ south: 37.5, west: -122.5, north: 38.5, east: -121.5 });
  expect(calls).toHaveLength(1);
  expect(calls[0].path).toBe("/api/v1/gev/overpass");
  expect(calls[0].init?.method).toBe("POST");
  const body = String(calls[0].init?.body);
  expect(body.startsWith("data=")).toBe(true);
  expect(decodeURIComponent(body.slice(5))).toContain("[out:json]");
  expect(decodeURIComponent(body.slice(5))).toContain('way["highway"~');
  expect(res.ok).toBe(true);
  const data = await res.json();
  expect(Array.isArray(data.roads)).toBe(true);
});

test("requestRoads keeps the engine's bbox TypeError (no fetch on misuse)", async () => {
  const apiFetch: ApiFetch = vi.fn(async () => jsonResponse({}));
  const src = trafficSource(apiFetch);
  await expect(
    src.requestRoads({ south: 0, west: 0, north: 11, east: 1 }),
  ).rejects.toThrow(TypeError);
  expect(apiFetch).not.toHaveBeenCalled();
});

test("getStatus reads hasKey from the rewritten hub route", async () => {
  const calls: string[] = [];
  const apiFetch: ApiFetch = async (path) => {
    calls.push(String(path));
    return jsonResponse({ hasKey: true });
  };
  const src = trafficSource(apiFetch);
  const out = await src.getStatus();
  expect(calls[0]).toBe("/api/v1/gev/tomtom/status");
  expect(out.hasKey).toBe(true);
});

test("getStatus rejects a malformed status body (engine contract)", async () => {
  const src = trafficSource(async () => jsonResponse({ nope: 1 }));
  await expect(src.getStatus()).rejects.toThrow(/Malformed traffic status/);
});

test("flow tile requests hit the rewritten .pbf route; all-failed rejects, partial tolerates", async () => {
  const calls: string[] = [];
  // every tile 503 → fetchFlowForBounds must reject (flowSource.js:78-83)
  const failing = trafficSource(async () => new Response("x", { status: 503 }));
  await expect(
    failing.fetchFlowForBounds({ south: 37.0, west: -122.6, north: 37.1, east: -122.5 }),
  ).rejects.toThrow();

  const apiFetch: ApiFetch = async (path) => {
    calls.push(String(path));
    return new Response(new ArrayBuffer(0), { status: 200 });
  };
  const src = trafficSource(apiFetch);
  const out = await src.fetchFlowForBounds(
    { south: 37.0, west: -122.6, north: 37.1, east: -122.5 },
    { zoom: 8 },
  );
  expect(Array.isArray(out)).toBe(true); // empty MVT buffers decode to []
  expect(calls.length).toBeGreaterThan(0);
  for (const c of calls)
    expect(c).toMatch(/^\/api\/v1\/gev\/tomtom\/flow\/8\/\d+\/\d+\.pbf$/);
});

test("flow session stats + cache reset methods exist (SOURCE_METHODS contract)", () => {
  const src = trafficSource(async () => jsonResponse({}));
  expect(src.getFlowSessionStats()).toEqual({ tilesFetched: 0 });
  expect(() => src.resetFlowTileCache()).not.toThrow();
});

test("factory wires the real traffic source (stub replaced)", () => {
  const s = createIntelHubLayerSources({ apiFetch: async () => jsonResponse({}) });
  const t = s.traffic as Record<string, unknown>;
  for (const m of ["requestRoads", "getStatus", "fetchFlowForBounds", "getFlowSessionStats", "resetFlowTileCache"])
    expect(typeof t[m]).toBe("function");
});
