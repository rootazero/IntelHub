// Real (GEV P3 T4) military-installations layer source: IntelHub's harvested
// OSM military catalog.
//
// Replaces the T3 `stubs.installations` entry. Unlike the vessels adapter
// (./vessels.ts, a full TS port), this one REUSES the engine's own source
// factory — `gev-engine/src/layers/installations/source.js`
// `createInstallationSource` — because it owns three contract behaviors that
// must not drift: the bbox TypeError validation (source.js:19-30), the error
// envelope mapping (`{error, reason}` → thrown Error with `failureReason`,
// source.js:46-52), and the client-side normalize +
// saturation inference (spread of `normalizeMilitaryInstallations` +
// `installationResponseSaturated`, contracts.md §4).
//
// The ONLY delta IntelHub needs is the endpoint path: the engine hardcodes
// `/api/military-installations`, while the hub serves the identical envelope
// from `/api/v1/gev/installations` (hub-core gev_installations.rs, which
// recombines the T3 PG harvest back into Overpass elements). So this adapter
// is a fetchImpl wrapper: a same-origin path rewrite on top of the injected
// authenticated apiFetch (./http.ts). The vendor module is pure (its only
// import is the normalize data module — no bundled static data, no I/O).
//
// searchNearby (Google Places) is deliberately NOT proxied in P3:
// contracts.md §4 marks it the optional degradation path ("失败仅降级提示，
// 不影响主路径"), the engine fires it only on explicit user request
// (ingestion.js:72 googleSearchRequested gate), and an empty `{ places: [] }`
// degrades silently — no chip, no error (the chip needs a REJECTION,
// ingestion.js:129-131). Input validation mirrors the engine source
// (source.js:66-73) so contract misuse still throws TypeError.

import { createInstallationSource } from "gev-engine/src/layers/installations/source.js";
import type { ApiFetch } from "./http";

/** Engine-hardcoded endpoint (source.js:36) → IntelHub hub REST route. */
const ENGINE_PATH = "/api/military-installations";
const HUB_PATH = "/api/v1/gev/installations";

export interface InstallationViewport {
  south: number;
  west: number;
  north: number;
  east: number;
}

export interface NearbyQuery {
  latitude: number;
  longitude: number;
  radiusM: number;
}

/** What the engine's ingestion destructures (ingestion.js:55-69): the
 * normalize spread `{records, droppedCount}` plus `status` + `saturated`. */
export interface MappedSitesResult {
  records: unknown[];
  droppedCount: number;
  status?: string;
  saturated: boolean;
}

export interface InstallationsSource {
  getMappedSites(
    box: InstallationViewport,
    options?: { exact?: boolean; signal?: AbortSignal },
  ): Promise<MappedSitesResult>;
  searchNearby(
    center: NearbyQuery,
    options?: { signal?: AbortSignal },
  ): Promise<{ places: unknown[] }>;
}

export const installationsSource = (apiFetch: ApiFetch): InstallationsSource => {
  // Path rewrite, not URL reconstruction: the engine builds the query string
  // (`south/west/north/east` toFixed(5) + optional `exact=1`), which must
  // survive untouched — only the route prefix changes.
  const engine = createInstallationSource({
    fetchImpl: (path: string, init?: RequestInit) =>
      apiFetch(
        path.startsWith(ENGINE_PATH)
          ? HUB_PATH + path.slice(ENGINE_PATH.length)
          : path,
        init,
      ),
  });

  return {
    getMappedSites: (box, options) => engine.getMappedSites(box, options),

    async searchNearby(
      { latitude, longitude, radiusM }: NearbyQuery,
      { signal }: { signal?: AbortSignal } = {},
    ) {
      // Mirror of source.js:66-73 — contract misuse is a TypeError even
      // though the request itself is declined.
      if (
        ![latitude, longitude, radiusM].every(Number.isFinite) ||
        Math.abs(latitude) > 90 ||
        Math.abs(longitude) > 180 ||
        radiusM < 1000 ||
        radiusM > 50000
      )
        throw new TypeError("Invalid nearby installation search");
      signal?.throwIfAborted();
      // P3: Google Places proxy not implemented — see file header.
      return { places: [] };
    },
  };
};
