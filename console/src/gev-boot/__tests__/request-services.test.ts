// Explicit vitest imports (repo convention, cf. gev-adapters/__tests__/stubs.test.ts):
// vitest.config.ts sets globals:true for the runtime, but `tsc -b` does not load
// vitest/globals types, so bare `test`/`expect` break `npm run build`.
import { expect, test } from "vitest";
import type { ApiFetch } from "../../gev-adapters/http";
import { createIntelHubRequestServices } from "../request-services";

const apiFetch: ApiFetch = async () => new Response("{}");

test("every engine-validated field exists and is callable", () => {
  // gev-engine/src/app/operations.js:10-17 validates these four by name at
  // scene construction; a missing one throws before the viewer ever boots.
  const r = createIntelHubRequestServices(apiFetch);
  expect(typeof r.boundaries.query).toBe("function");
  expect(typeof r.terrain.getHeights).toBe("function");
  expect(typeof r.regional.getBrief).toBe("function");
  expect(typeof r.weather.getConditions).toBe("function");
  expect(typeof r.summary.summarize).toBe("function");
});

test("features passes the engine's requireFeatureSource gate", () => {
  // sources/featureSource.js validates NINE methods (the brief's
  // {geocode,routing,llm} shape predates that surface and would throw
  // "Missing feature operation"). This list mirrors FEATURE_SOURCE_METHODS.
  const { features } = createIntelHubRequestServices(apiFetch);
  for (const m of [
    "getAdministrativeAreas",
    "getAreaGeometry",
    "getNeighborhoodAreas",
    "getStreetAreas",
    "getStreetLines",
    "getFootprints",
    "getEnclosingAreas",
    "getMonuments",
    "getFocusFootprints",
  ])
    expect(typeof (features as any)[m], m).toBe("function");
});

test("every method resolves to a degraded value, never throws", async () => {
  const r = createIntelHubRequestServices(apiFetch);
  // null = "retryable failure" in the engine's own vocabulary — the degraded
  // path its consumers already handle.
  await expect(r.boundaries.query("rel...")).resolves.toBeNull();
  await expect(r.regional.getBrief(30.3, -97.7)).resolves.toBeNull();
  await expect(r.weather.getConditions(30.3, -97.7)).resolves.toBeNull();
  await expect(r.summary.summarize({})).resolves.toBeNull();
  // terrain: [] triggers the engine's documented "length mismatch" error →
  // geoid fallback (services/terrainHeights.js).
  await expect(r.terrain.getHeights([{ lat: 1, lon: 2 }])).resolves.toEqual([]);
  for (const m of [
    "getAdministrativeAreas",
    "getAreaGeometry",
    "getNeighborhoodAreas",
    "getStreetAreas",
    "getStreetLines",
    "getFootprints",
    "getEnclosingAreas",
    "getMonuments",
    "getFocusFootprints",
  ])
    await expect((r.features as any)[m]({ lat: 30.3, lon: -97.7 })).resolves.toBeNull();
});
