// Factory for the 14 GEV layer sources IntelHub supplies to the vendored
// engine. The engine's `createApplicationCatalog` (constructCatalog.js) throws
// `Invalid catalog source: <name>` unless every SOURCE_METHODS entry is a
// function, so this object is the single wiring point.
//
// Wave-1 real implementations land in T4-T6 and replace the four entries
// marked below, consuming `deps.apiFetch`. Every other entry stays a contract
// stub until its own wave (T3 goal: layers enable, render empty, never throw).

import { earthquakesSource } from "./earthquakes";
import * as stubs from "./stubs";
import type { ApiFetch } from "./http";

export type LayerSources = Record<string, object>;

export function createIntelHubLayerSources(deps: {
  apiFetch: ApiFetch;
}): LayerSources {
  // `deps` (notably `apiFetch`) is the transport the wave-1 implementations
  // close over when they replace the stubs below. The stubs themselves are pure
  // data objects and perform no I/O, so they never touch it.
  const { apiFetch } = deps;

  return {
    flights: stubs.flights, // T6 → real OpenSky snapshot source
    military: stubs.military, // T6 → real military snapshot source
    satellites: stubs.satellites, // T5 → real CelesTrak group source
    earthquakes: earthquakesSource(apiFetch), // T4 → real USGS snapshot source
    vessels: stubs.vessels,
    firms: stubs.firms,
    cables: stubs.cables,
    alpr: stubs.alpr,
    launches: stubs.launches,
    cctv: stubs.cctv,
    radio: stubs.radio,
    traffic: stubs.traffic,
    bikeshare: stubs.bikeshare,
    installations: stubs.installations,
  };
}
