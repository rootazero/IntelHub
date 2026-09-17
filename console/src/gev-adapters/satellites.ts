// Real (wave-1) satellites layer source: CelesTrak TLE groups served by IntelHub.
//
// Replaces the T3 `stubs.satellites` entry. The transport is the injected
// authenticated `apiFetch` (see ./http.ts) hitting the hub REST endpoint
// `GET /api/v1/gev/celestrak/{group}`, which serves three-line TLE text
// (name / line1 / line2) for the GEV six core groups out of the PG `satellites`
// catalog (hub-core monitor/sources/celestrak.rs, 6h cadence) and proxies
// `starlink` on demand (Redis-cached 6h at `hub:globe:tle:starlink`):
//
//   stations | visual | gps-ops | glo-ops | galileo | geo   → PG catalog
//   starlink                                                → hub proxy
//
// The return shape `{ ok, status, text }` is exactly what the engine's
// satellites ingestion destructures (gev-engine/src/layers/satellites/
// ingestion.js:31) — it maps non-ok to an empty group without throwing, so the
// adapter must NOT throw on HTTP errors either (only on contract misuse, i.e.
// an unknown group, which is a programming error, not a transport state).

import type { ApiFetch } from "./http";
import type { ReadGroupResult } from "./types";

/** Groups the endpoint serves. Mirrors hub-core api.rs `GEV_TLE_GROUPS` + starlink. */
const GROUPS = new Set([
  "stations",
  "visual",
  "gps-ops",
  "glo-ops",
  "galileo",
  "geo",
  "starlink",
]);

export interface SatellitesSource {
  label: string;
  readGroup(group: string, options?: { signal?: AbortSignal }): Promise<ReadGroupResult>;
}

export const satellitesSource = (apiFetch: ApiFetch): SatellitesSource => ({
  label: "CelesTrak via IntelHub",
  async readGroup(group: string, { signal }: { signal?: AbortSignal } = {}) {
    if (!GROUPS.has(group)) throw new TypeError("Unknown satellite group");
    const res = await apiFetch(`/api/v1/gev/celestrak/${group}`, { signal });
    return { ok: res.ok, status: res.status, text: res.ok ? await res.text() : "" };
  },
});
