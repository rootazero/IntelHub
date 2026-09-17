// Explicit vitest imports (repo convention, cf. gev-boot/__tests__/define.test.ts):
// vitest.config.ts sets globals:true for the runtime, but `tsc -b` does not load
// vitest/globals types, so bare `test`/`expect` break `npm run build`.
import { expect, test } from "vitest";
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
};

test("sources cover every SOURCE_METHODS entry", () => {
  const s = createIntelHubLayerSources({
    apiFetch: async () => new Response("{}"),
  });
  for (const [layer, methods] of Object.entries(REQUIRED))
    for (const m of methods)
      expect(typeof (s as any)[layer]?.[m], `${layer}.${m}`).toBe("function");
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
  const REAL = new Set(["earthquakes", "satellites", "flights", "military"]);
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
