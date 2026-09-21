// GEV P12 — aircraft source object (mirror of vendor standalone.js)
//
// Vendor URL contracts (HARDCODED in gev-engine/src/sources/live/standalone.js):
//   Line 86: '/api/opensky-track?icao24=' + encodeURIComponent(reference)
//   Line 99: `/api/adsbdb/${query.kind}/${encodeURIComponent(query.id)}`
// Any path refactor WILL BREAK vendor silently. The path-literal assertions in
// console/src/gev-boot/__tests__/source-contracts.test.ts (T4) pin these
// strings.
//
// Transport is the injected authenticated `apiFetch` (./http.ts makeApiFetch
// injects `Authorization: Bearer ihk_<hex>`). The hub routes here start with
// `/api/`, so auth_middleware requires the bearer token — a plain `fetch`
// would 401 and the vendor would swallow it.

import type { ApiFetch } from "./http";
import type { AdsbEnvelope, GevAircraftRecord, SnapshotEnvelope } from "./types";
import { snapshotEpochMs, toEnvelope, toGevRecord } from "./aircraft-map";

export interface TrackRecord {
  observedAtMs: number;
  latitude: number;
  longitude: number;
  baroAltitudeM: number | null;
  courseDeg: number | null;
  onGround: boolean;
}

export interface AirportInfo {
  code: string;
  name: string;
  lat: number | null;
  lon: number | null;
}

export type EnrichmentPayload =
  | { found: true; typeCode?: string; typeName?: string; registration?: string }
  | {
      found: true;
      airline?: string;
      origin?: AirportInfo;
      destination?: AirportInfo;
    }
  | { found: false };

export interface IntelHubAircraftSource {
  label: string;
  getSnapshot(
    query: SnapshotQuery,
    opts?: { signal?: AbortSignal },
  ): Promise<SnapshotEnvelope<GevAircraftRecord>>;
  getTrack(
    reference: string,
    opts?: { signal?: AbortSignal },
  ): Promise<{ records: TrackRecord[]; complete: false }>;
  getEnrichment(
    query: { kind: "type" | "route"; id: string },
    opts?: { signal?: AbortSignal },
  ): Promise<EnrichmentPayload>;
}

export interface SnapshotQuery {
  latitude?: number;
  longitude?: number;
}

const HEX6_RE = /^[0-9a-f]{6}$/;
// ADS-B callsigns are alphanumeric (e.g. UAL123, BAW456) — letters AND digits.
const CALLSIGN_RE = /^[A-Z0-9]{2,8}$/;

export function createIntelHubAircraftSource({
  apiFetch,
}: {
  apiFetch: ApiFetch;
}): IntelHubAircraftSource {
  // Constructor contract (P3 lesson): adapter methods are silently broken if
  // apiFetch is missing or non-functional. Assert now so test mocks cannot hide.
  if (typeof apiFetch !== "function") {
    throw new TypeError(
      "createIntelHubAircraftSource: apiFetch must be a function",
    );
  }

  return {
    label: "hub.intelhub",

    async getSnapshot(query, { signal } = {}) {
      const params = new URLSearchParams();
      if (
        Number.isFinite(query?.latitude) &&
        Number.isFinite(query?.longitude)
      ) {
        params.set("lat", String(query.latitude));
        params.set("lon", String(query.longitude));
      }
      const url = `/api/v1/globe/aircraft${params.toString() ? "?" + params : ""}`;
      const res = await apiFetch(url, { signal });
      if (!res.ok) {
        throw new Error(`aircraft snapshot HTTP ${res.status}`);
      }
      const env = (await res.json()) as AdsbEnvelope;
      // snapshotEpochMs guards NaN (unparseable `ts`) so an invalid epoch
      // cannot leak into record timestamps; matches the T6 flights source.
      const observedAtMs = snapshotEpochMs(env);
      const records = (Array.isArray(env.aircraft) ? env.aircraft : []).map(
        (p) => toGevRecord(p, observedAtMs),
      );
      return toEnvelope(records, env, "hub.intelhub", Date.now());
    },

    async getTrack(reference, { signal } = {}) {
      const hex = reference.toLowerCase();
      if (!HEX6_RE.test(hex)) {
        // Silent fallback for non-hex (vendor always passes icao24, defensive).
        return { records: [], complete: false as const };
      }
      // 8s timeout mirrors vendor (tracking.js:484 AbortSignal.timeout(8000)).
      const timeoutSignal = AbortSignal.timeout(8000);
      const composedSignal = signal
        ? AbortSignal.any([signal, timeoutSignal])
        : timeoutSignal;
      try {
        const res = await apiFetch(
          `/api/opensky-track?icao24=${encodeURIComponent(hex)}`,
          { signal: composedSignal },
        );
        if (!res.ok) {
          return { records: [], complete: false as const };
        }
        const body = await res.json();
        return { records: normalizeTrackPath(body), complete: false as const };
      } catch {
        // Silent fallback (vendor tracking.js:486-492 catch { return; }).
        return { records: [], complete: false as const };
      }
    },

    async getEnrichment(query, { signal } = {}) {
      if (query.kind !== "type" && query.kind !== "route") {
        throw new Error(
          `unsupported enrichment kind: ${(query as { kind: string }).kind}`,
        );
      }
      const id =
        query.kind === "route" ? query.id.toUpperCase() : query.id.toLowerCase();
      if (query.kind === "type" && !HEX6_RE.test(id)) {
        throw new Error("type enrichment id must be 6-char hex");
      }
      if (query.kind === "route" && !CALLSIGN_RE.test(id)) {
        throw new Error("route enrichment id must be 2-8 char callsign");
      }
      const res = await apiFetch(
        `/api/adsbdb/${query.kind}/${encodeURIComponent(id)}`,
        { signal },
      );
      if (!res.ok) {
        // 4xx (404 etc) → throw, vendor silently skips.
        // 5xx → throw, vendor records cooldown.
        throw new Error(`adsbdb enrichment HTTP ${res.status}`);
      }
      return (await res.json()) as EnrichmentPayload;
    },
  };
}

/**
 * Normalize an OpenSky `/tracks/all` response (array-of-arrays) to
 * `TrackRecord[]`. Mirrors the vendor's `normalizeAircraftTrack`
 * (gev-engine/src/sources/live/aircraft.js:134): a waypoint is admitted when
 * its time offset and coordinates are finite; altitude/track/ground are
 * optional (`null` / `false` when absent).
 *
 * Row shape: `[time, lat, lon, baro_alt, true_track, on_ground]`.
 */
function normalizeTrackPath(payload: unknown): TrackRecord[] {
  const path = (payload as { path?: unknown })?.path;
  if (!Array.isArray(path)) return [];
  const records: TrackRecord[] = [];
  for (const waypoint of path) {
    if (!Array.isArray(waypoint)) continue;
    const [time, lat, lon, baroAlt, track, ground] = waypoint;
    if (!Number.isFinite(time) || !Number.isFinite(lat) || !Number.isFinite(lon))
      continue;
    records.push({
      observedAtMs: (time as number) * 1000,
      latitude: lat as number,
      longitude: lon as number,
      baroAltitudeM: Number.isFinite(baroAlt) ? (baroAlt as number) : null,
      courseDeg: Number.isFinite(track) ? (track as number) : null,
      onGround: ground === true,
    });
  }
  return records;
}
