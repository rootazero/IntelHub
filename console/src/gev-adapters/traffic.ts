// Real (GEV P3 T7) traffic layer source: IntelHub hub-proxied Overpass roads
// + TomTom flow tiles.
//
// Replaces the T3 `stubs.traffic` entry. Like ./installations.ts, this adapter
// REUSES the engine's own source factory —
// `gev-engine/src/layers/traffic/source.js` `createTrafficSource` — because it
// owns contract behaviors that must not drift: the Overpass QL construction
// and bbox TypeError validation (source.js:9-42), the `{ok,status,headers,
// json()}` response wrapper with client-side `normalizeOverpassRoads`
// (source.js:44-67), the `{hasKey}` status parsing (source.js:69-79), and the
// flow tile pipeline (flowSource.js: tile math, per-tile Promise.allSettled
// partial-failure tolerance, 120s/64-entry decode cache, MVT decode via
// flowDecode.js expecting layer "Traffic flow" with traffic_level/road_closure
// — contracts.md §2).
//
// The ONLY delta IntelHub needs is the endpoint prefix: the engine hardcodes
// `/api/overpass` and `/api/tomtom/...` (source.js:50, :73;
// flowSource.js:61), while the hub serves the identical contract from
// `/api/v1/gev/overpass` and `/api/v1/gev/tomtom/...` (hub-core
// gev_traffic.rs, commit 296c94c). So this adapter is a fetchImpl prefix
// rewrite on top of the injected authenticated apiFetch (./http.ts). The
// vendor modules are pure (flowSource.js imports only tomtomTiles.js +
// flowDecode.js — no bundled static data, no direct I/O).
//
// Quota note: the TomTom free tier is 200K tile requests/month. The engine's
// own decode cache (120s TTL) plus the hub's 120s Redis cache (T6) are the
// two-tier protection; the layer ships OFF by default (rail grouping) so tiles
// only flow on explicit user enable.

import { createTrafficSource } from "gev-engine/src/layers/traffic/source.js";
import type { ApiFetch } from "./http";

/** Engine-hardcoded prefixes → IntelHub hub REST routes. */
const PREFIX_MAP: ReadonlyArray<readonly [string, string]> = [
  ["/api/overpass", "/api/v1/gev/overpass"],
  ["/api/tomtom/", "/api/v1/gev/tomtom/"],
];

export interface TrafficRoadsResponse {
  ok: boolean;
  status: number;
  headers: Headers;
  json: () => Promise<{ roads: unknown[] }>;
}

export interface TrafficSource {
  requestRoads(
    box: { south: number; west: number; north: number; east: number },
    options?: { majorOnly?: boolean; timeoutSec?: number; signal?: AbortSignal },
  ): Promise<TrafficRoadsResponse>;
  getStatus(options?: { signal?: AbortSignal }): Promise<{ hasKey: boolean }>;
  fetchFlowForBounds(
    bounds: { south: number; west: number; north: number; east: number },
    options?: { signal?: AbortSignal; zoom?: number },
  ): Promise<unknown[]>;
  getFlowSessionStats(): { tilesFetched: number };
  resetFlowTileCache(): void;
}

/** Exported for tests: pure prefix rewrite, no other mutation of the URL.
 * The engine's flow URLs carry a `.pbf` suffix (flowSource.js:61); the hub
 * route drops it — axum forbids `{y}.pbf` in one segment (315 boot panic
 * 2026-09-17), and the suffix is decorative for the proxy anyway. */
export function rewriteTrafficPath(path: string): string {
  for (const [from, to] of PREFIX_MAP)
    if (path.startsWith(from)) {
      const rest = path.slice(from.length);
      return to + (from === "/api/tomtom/" ? rest.replace(/\.pbf$/, "") : rest);
    }
  return path;
}

export const trafficSource = (apiFetch: ApiFetch): TrafficSource => {
  const engine = createTrafficSource({
    fetchImpl: (path: string, init?: RequestInit) =>
      apiFetch(rewriteTrafficPath(path), init),
  });
  // createTrafficSource spreads createFlowTileSource (source.js:23-26), so the
  // returned object already satisfies the full five-method contract
  // (stubs.test.ts REQUIRED.traffic). Pass it through untouched.
  return engine as unknown as TrafficSource;
};
