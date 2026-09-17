// Minimal request-service surface for the vendored GEV engine's application
// protocol.
//
// Step-1 alignment (gev-engine/src/app/operations.js:10-17 + sources/
// featureSource.js): the engine validates, at scene construction,
//   terrain.getHeights / regional.getBrief / weather.getConditions /
//   summary.summarize                       — all must be functions
// and then runs requests.features through requireFeatureSource(), which
// requires NINE methods (getAdministrativeAreas, getAreaGeometry,
// getNeighborhoodAreas, getStreetAreas, getStreetLines, getFootprints,
// getEnclosingAreas, getMonuments, getFocusFootprints). The brief's
// `features: { geocode, routing, llm }` shape predates that engine surface
// and would throw "Missing feature operation" — so the disabled-features
// object below implements the nine real methods instead.
//
// Every method resolves null ("retryable failure" in the engine's own
// vocabulary, cf. sources/overpassFeatures.js query contract) or an empty
// collection, so the engine's degradation paths engage exactly as they do for
// a real upstream outage: layers render empty, nothing throws. The optional
// IntelHub endpoints (boundaries/regional/weather/summary) land in P4/P5 and
// will reuse the injected apiFetch; the apiFetch parameter is therefore part
// of the stable signature now.

import type { ApiFetch } from "../gev-adapters/http";

/** Feature source with every method present but resolving "no data". */
const disabledFeatures = () => {
  // null = retryable failure, the engine's own degraded-result contract —
  // consumers (annotations resolver, search recovery) already handle it.
  // Signatures stay arg-taking so callers pass coordinates/options exactly as
  // they would to the engine's real feature source.
  const unavailable = async (..._args: unknown[]) => null;
  return {
    getAdministrativeAreas: unavailable,
    getAreaGeometry: unavailable,
    getNeighborhoodAreas: unavailable,
    getStreetAreas: unavailable,
    getStreetLines: unavailable,
    getFootprints: unavailable,
    getEnclosingAreas: unavailable,
    getMonuments: unavailable,
    getFocusFootprints: unavailable,
  };
};

/**
 * Build the engine's request-services object, IntelHub edition.
 *
 * @param apiFetch Authenticated hub transport. Unused by the current
 *   all-disabled surface, but kept in the signature so P4 (Open-Meteo
 *   weather) and P5 (boundaries/regional) can wire real endpoints without a
 *   breaking change to `createIntelHubGlobe`.
 */
export function createIntelHubRequestServices(apiFetch: ApiFetch) {
  void apiFetch; // reserved for P4/P5 endpoints — see header
  return {
    // National boundaries via Overpass — P5.
    boundaries: { query: async (..._args: unknown[]) => null },
    // Terrain sampling stays disabled: the engine's terrainHeights falls back
    // to bundled-geoid math when getHeights errors (services/terrainHeights.js
    // header). Returning [] for a non-empty chunk throws its documented
    // "length mismatch" error, which IS that fallback trigger.
    terrain: { getHeights: async (..._args: unknown[]) => [] as unknown[] },
    // Regional brief — P5.
    regional: { getBrief: async (..._args: unknown[]) => null },
    // Weather — P4 (Open-Meteo).
    weather: { getConditions: async (..._args: unknown[]) => null },
    // LLM HUD summary — engine HUD is not constructed in the IntelHub
    // bootstrap (no engine controls/tools phase), so nothing calls this yet.
    summary: { summarize: async (..._args: unknown[]) => null },
    // Own the features slot so operations.js never falls back to the engine's
    // createOverpassFeatureSource (which would POST to /api/overpass).
    features: disabledFeatures(),
  };
}

export type IntelHubRequestServices = ReturnType<
  typeof createIntelHubRequestServices
>;
