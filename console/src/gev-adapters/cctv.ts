// Real (GEV P3 T13) CCTV layer source: IntelHub hub-backed camera catalog
// + frame/media proxy.
//
// Replaces the T3 `stubs.cctv` entry. Unlike installations/traffic, this
// adapter does NOT reuse the engine's `createCctvSource` factory: that
// factory builds frame/media URLs from the module-level FRAME_ENDPOINT /
// MEDIA_ENDPOINT constants (source.js:10-29 + sourcePolicy.js), and those
// URLs are consumed directly by `new Image()` / `<video>` (projection.js)
// — they never pass through the injected fetchImpl, so a fetch wrapper
// cannot rewrite them. All four methods are therefore implemented here,
// mirroring the vendor semantics:
//
// - getCatalog  → GET /api/v1/gev/cctv/sources  (hub gev_cctv.rs; engine
//   hardcodes /api/cctv/sources, source.js:46-48) — `{sources}` must be an
//   array or 'Malformed camera sources snapshot' (source.js:38-44)
// - getHealth   → GET /api/v1/gev/cctv/health — `{cameras}` array or
//   'Malformed camera health snapshot'
// - getFrameUrl → /api/v1/gev/cctv/frame/{id}?<same query contract as
//   source.js:10-26 frameUrlFor> (label/city/lat/lon/heading/fov/pitch/ts;
//   ts = floor(now/refreshMs) cache cadence, ACTIVE_FRAME_REFRESH_MS=10000)
// - getMediaUrl → /api/v1/gev/cctv/media/{id}?ts=<15s grid>
//
// Frames/media are served same-origin by the hub proxy (T12), which is
// what the engine's WebGL texture projection path requires (cross-origin
// video/canvas would taint). The query params are render hints for the
// vendor's placeholder-frame renderer; the hub ignores them.

import type { ApiFetch } from "./http";

const FRAME_ENDPOINT = "/api/v1/gev/cctv/frame";
const MEDIA_ENDPOINT = "/api/v1/gev/cctv/media";
const SOURCE_ENDPOINT = "/api/v1/gev/cctv/sources";
const HEALTH_ENDPOINT = "/api/v1/gev/cctv/health";
/** Mirror of vendor sourcePolicy.js ACTIVE_FRAME_REFRESH_MS. */
const ACTIVE_FRAME_REFRESH_MS = 10000;

export interface CctvCamera {
  id: string;
  name?: string;
  city?: string;
  lat?: number;
  lon?: number;
  headingDeg?: number;
  fovDeg?: number;
  pitchDeg?: number;
  [key: string]: unknown;
}

export interface CctvSource {
  getCatalog(options?: { signal?: AbortSignal }): Promise<{ sources: unknown[] }>;
  getHealth(options?: { signal?: AbortSignal }): Promise<{ cameras: unknown[] }>;
  getFrameUrl(camera: CctvCamera, refreshMs?: number): string;
  getMediaUrl(camera: CctvCamera): string;
}

function safeNumber(value: unknown, fallback: number): number {
  const n = Number(value);
  return Number.isFinite(n) ? n : fallback;
}

/** Mirrors vendor source.js frameUrlFor: same param names/rounding. */
export function frameUrlFor(camera: CctvCamera, refreshMs = ACTIVE_FRAME_REFRESH_MS): string {
  const cadenceMs = Math.max(1000, safeNumber(refreshMs, ACTIVE_FRAME_REFRESH_MS));
  const tick = Math.floor(Date.now() / cadenceMs);
  const params = new URLSearchParams({
    label: String(camera.name ?? camera.id),
    city: String(camera.city ?? ""),
    lat: safeNumber(camera.lat, 0).toFixed(6),
    lon: safeNumber(camera.lon, 0).toFixed(6),
    heading: String(Math.round(safeNumber(camera.headingDeg, 0))),
    fov: String(Math.round(safeNumber(camera.fovDeg, 74))),
    pitch: String(Math.round(safeNumber(camera.pitchDeg, -10))),
    ts: String(tick),
  });
  return `${FRAME_ENDPOINT}/${encodeURIComponent(camera.id)}?${params.toString()}`;
}

/** Mirrors vendor source.js mediaUrlFor (15s cache grid). */
export function mediaUrlFor(camera: CctvCamera): string {
  return `${MEDIA_ENDPOINT}/${encodeURIComponent(camera.id)}?ts=${Math.floor(Date.now() / 15000)}`;
}

export const cctvSource = (apiFetch: ApiFetch): CctvSource => {
  async function read(path: string, key: string, { signal }: { signal?: AbortSignal } = {}) {
    signal?.throwIfAborted();
    const response = await apiFetch(path, { cache: "no-store", signal });
    if (!response.ok) throw new Error("Camera source HTTP " + response.status);
    const payload = await response.json();
    signal?.throwIfAborted();
    if (!Array.isArray(payload?.[key])) throw new Error("Malformed camera " + key + " snapshot");
    return payload;
  }
  return {
    getCatalog: (options) => read(SOURCE_ENDPOINT, "sources", options),
    getHealth: (options) => read(HEALTH_ENDPOINT, "cameras", options),
    getFrameUrl: frameUrlFor,
    getMediaUrl: mediaUrlFor,
  };
};
