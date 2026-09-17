// Explicit vitest imports (repo convention, cf. gev-boot/__tests__/define.test.ts):
// vitest.config.ts sets globals:true for the runtime, but `tsc -b` does not load
// vitest/globals types, so bare `test`/`expect` break `npm run build`.
import { expect, test, vi } from "vitest";
import type { ApiFetch } from "../http";
import { createIntelHubLayerSources } from "../index";

// Mirrors SOURCE_METHODS in console/gev-engine/src/app/constructCatalog.js:26-47.
// constructCatalog throws `Invalid catalog source: <name>` if any layer is
// missing a method, so the factory must satisfy every entry for the engine to
// boot with the stubs in place.
const REQUIRED: Record<string, string[]> = {
  flights: ["getSnapshot"],
  military: ["getSnapshot"],
  vessels: ["getSnapshot"],
  cctv: ["getCatalog", "getHealth", "getFrameUrl", "getMediaUrl"],
  radio: ["getDirectory", "recordClick"],
  traffic: [
    "requestRoads",
    "getStatus",
    "fetchFlowForBounds",
    "getFlowSessionStats",
    "resetFlowTileCache",
  ],
  bikeshare: ["getStations"],
  installations: ["getMappedSites", "searchNearby"],
  satellites: ["readGroup"],
  launches: ["getLaunches", "getActiveTle"],
  alpr: ["fetch"],
  firms: ["getSnapshot"],
  earthquakes: ["getSnapshot"],
  cables: ["fetch"],
  // Not in SOURCE_METHODS (the engine layer validates it itself,
  // transit/index.js:34-41), but the catalog consumes sources.transit
  // (constructCatalog.js:127) — the stub must be registered here so the
  // engine's own unauthenticated /api/transit fallback never engages (T3 I-2).
  transit: ["requestSnapshot", "getHistory"],
};

test("sources cover every SOURCE_METHODS entry", () => {
  const s = createIntelHubLayerSources({
    apiFetch: async () => new Response("{}"),
  });
  for (const [layer, methods] of Object.entries(REQUIRED))
    for (const m of methods)
      expect(typeof (s as any)[layer]?.[m], `${layer}.${m}`).toBe("function");
});

// T3 M-4: the factory is the single wiring point, so these spy assertions go
// through createIntelHubLayerSources (not the per-source factories) — they
// prove the real wave-1 sources close over the injected transport. If a future
// refactor drops the apiFetch closure ("transport not wired"), the spy never
// fires and this test fails.
test("factory-wired real sources call the injected apiFetch", async () => {
  const apiFetch = vi.fn<ApiFetch>(async (path) => {
    const body =
      path === "/api/v1/gev/earthquakes"
        ? []
        : path.startsWith("/api/v1/globe/aircraft")
          ? { ts: "2026-09-17T12:00:00.000Z", aircraft: [] }
          : ""; // satellites readGroup → res.text()
    return new Response(JSON.stringify(body), { status: 200 });
  });
  const s = createIntelHubLayerSources({ apiFetch });
  await (s as any).flights.getSnapshot();
  await (s as any).earthquakes.getSnapshot();
  await (s as any).satellites.readGroup("visual");
  const paths = apiFetch.mock.calls.map((call) => call[0]);
  expect(paths).toContain("/api/v1/globe/aircraft");
  expect(paths).toContain("/api/v1/gev/earthquakes");
  expect(paths).toContain("/api/v1/gev/celestrak/visual");
});

test("stub getSnapshot resolves to empty-degraded envelope", async () => {
  const s = createIntelHubLayerSources({
    apiFetch: async () => new Response("{}"),
  });
  const env = await (s as any).vessels.getSnapshot({}, {});
  expect(env.records).toEqual([]);
  // Engine vocabulary (T3 review I-1): freshness "unknown" is the degraded
  // signal the ingestion checks, and `status` is an HTTP-ish code, not an enum.
  expect(env.freshness).toBe("unknown");
  expect(env.observedAtMs).toBeNull();
  expect(env.ageMs).toBeNull();
  expect(env.status).toBe(503);
});

// T3's stated goal is "layer enables, renders empty, never throws". The engine
// guards most calls with its own try/catch, but the stub methods themselves
// must always resolve — an unhandled rejection here would surface as a layer
// that fails to enable rather than one that simply shows nothing.
//
// Only STUB layers are checked here: the wave-1 real sources deliberately throw
// on contract misuse (earthquakes → HTTP error, satellites → unknown group
// TypeError, flights/military → HTTP error, see their own test files) — that is
// their specified behavior, not a stub-resilience regression.
test("every stub method resolves without throwing", async () => {
  const s = createIntelHubLayerSources({
    apiFetch: async () => new Response("{}"),
  });
  // P3 T4+T7: installations/traffic are real now too — they deliberately throw
  // the engine's bbox TypeError on misuse (their own test files cover it).
  const REAL = new Set(["earthquakes", "satellites", "flights", "military", "installations", "traffic"]);
  for (const [layer, methods] of Object.entries(REQUIRED)) {
    if (REAL.has(layer)) continue;
    for (const m of methods) {
      try {
        await (s as any)[layer][m]();
      } catch (error) {
        throw new Error(`${layer}.${m} rejected: ${String(error)}`);
      }
    }
  }
});
