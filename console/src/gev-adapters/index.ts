// Factory for the 15 GEV layer sources IntelHub supplies to the vendored
// engine. The engine's `createApplicationCatalog` (constructCatalog.js) throws
// `Invalid catalog source: <name>` unless every SOURCE_METHODS entry is a
// function, so this object is the single wiring point.
//
// Wave-1 real implementations (T4-T6) replace the flights/military/satellites/
// earthquakes entries, consuming `deps.apiFetch`. Every other entry stays a
// contract stub until its own wave (T3 goal: layers enable, render empty, never
// throw). The stub exports for the four wave-1 layers are kept in ./stubs.ts
// because the contract guard (T12) still introspects them.

import { earthquakesSource } from "./earthquakes";
import { flightsSource } from "./flights";
import { militarySource } from "./military";
import { satellitesSource } from "./satellites";
import { installationsSource } from "./installations";
import { trafficSource } from "./traffic";
import { vesselsSource } from "./vessels";
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
    flights: flightsSource(apiFetch), // T6 → real adsb.lol snapshot source
    military: militarySource(apiFetch), // T6 → real military snapshot source
    satellites: satellitesSource(apiFetch), // T5 → real CelesTrak group source
    earthquakes: earthquakesSource(apiFetch), // T4 → real USGS snapshot source
    vessels: vesselsSource(apiFetch), // T2 (GEV P3) → real AIS live snapshot source
    firms: stubs.firms,
    cables: stubs.cables,
    alpr: stubs.alpr,
    launches: stubs.launches,
    cctv: stubs.cctv,
    radio: stubs.radio,
    traffic: trafficSource(apiFetch), // T7 (GEV P3) → real hub-proxied Overpass + TomTom flow
    bikeshare: stubs.bikeshare,
    installations: installationsSource(apiFetch), // T4 (GEV P3) → real OSM military catalog source
    // T3 I-2: without this entry the transit layer silently falls back to the
    // engine's own unauthenticated /api/transit source (stubs.ts header).
    transit: stubs.transit,
  };
}
