// Explicit vitest imports (repo convention, cf. ./stubs.test.ts): vitest.config.ts
// sets globals:true for the runtime, but `tsc -b` does not load vitest/globals
// types, so bare `test`/`expect` break `npm run build`.
import { expect, test, vi } from "vitest";
import type { ApiFetch } from "../http";
import { createIntelHubLayerSources } from "../index";
import {
  KNOTS_TO_MPS,
  normalizeVesselObservation,
  normalizeVesselTrack,
  toVesselsSnapshot,
  vesselsSource,
} from "../vessels";
import type { AisLiveEnvelope, AisRow } from "../vessels";

const jsonResponse = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), { status });

// Field-for-field a hub `/api/v1/gev/ais-live` envelope (hub-core
// monitor/sources/ais.rs `vessels_envelope`). `speed` is KNOTS and
// `last_position_epoch` is Unix SECONDS — the adapter converts.
const ROW: AisRow = {
  mmsi: "636092423",
  lat: 1.264_56,
  lon: 103.839_76,
  name: "EVER GIVEN",
  imo: "9811000",
  type_specific: "CargoShips",
  destination: "ROTTERDAM",
  speed: 12.4,
  course: 86.7,
  heading: 87,
  last_position_epoch: 1_700_000_000,
};
const NEWEST_ISO = "2023-11-14T22:13:20Z";
const NEWEST_MS = Date.parse(NEWEST_ISO);
const LIVE_ENVELOPE: AisLiveEnvelope = {
  rows: [ROW],
  newestPositionAt: NEWEST_ISO,
  refreshing: false,
  status: "live",
  lastMessageAt: NEWEST_ISO,
  nextAttemptAt: null,
  silentForMs: 500,
  reconnectAttempt: 0,
};

test("normalizeVesselObservation converts knots→m/s and epoch seconds→ms", () => {
  const r = normalizeVesselObservation(ROW);
  expect(r).not.toBeNull();
  expect(r!.id).toBe("636092423");
  expect(r!.reference).toBe("636092423");
  expect(r!.latitude).toBe(1.26456);
  expect(r!.longitude).toBe(103.83976);
  expect(r!.name).toBe("EVER GIVEN");
  expect(r!.imo).toBe("9811000");
  expect(r!.type).toBe("CargoShips");
  expect(r!.destination).toBe("ROTTERDAM");
  // 12.4 kt → 6.3791056 m/s (sources/live/vessels.js:15 factor).
  expect(r!.speedMps).toBeCloseTo(12.4 * KNOTS_TO_MPS, 6);
  expect(r!.courseDeg).toBe(86.7);
  expect(r!.headingDeg).toBe(87);
  expect(r!.observedAtMs).toBe(1_700_000_000_000);
  expect(r!.altitudeDatum).toBe("sea-surface");
});

test("normalizeVesselObservation drops rows without id or valid coordinates", () => {
  expect(normalizeVesselObservation({ ...ROW, mmsi: "" })).toBeNull();
  expect(normalizeVesselObservation({ ...ROW, mmsi: null, input_identifier: "  " })).toBeNull();
  expect(normalizeVesselObservation({ ...ROW, lat: null })).toBeNull();
  expect(normalizeVesselObservation({ ...ROW, lat: 91 })).toBeNull();
  expect(normalizeVesselObservation({ ...ROW, lon: -181 })).toBeNull();
  // Missing optional fields degrade to null/empty strings, not fabricated 0s.
  const sparse = normalizeVesselObservation({ mmsi: "1", lat: 0, lon: 0 });
  expect(sparse).toMatchObject({
    id: "1",
    name: "1",
    imo: "",
    type: "",
    destination: "",
    speedMps: null,
    courseDeg: null,
    headingDeg: null,
    observedAtMs: null,
  });
});

test("normalizeVesselObservation falls back to input_* identity fields", () => {
  const r = normalizeVesselObservation({
    input_identifier: "legacy-7",
    input_name: "OLD SHIP",
    lat: 10,
    lon: 20,
    last_position_UTC: NEWEST_ISO,
  });
  expect(r!.id).toBe("legacy-7");
  expect(r!.name).toBe("OLD SHIP");
  expect(r!.observedAtMs).toBe(NEWEST_MS);
});

test("normalizeVesselTrack maps samples and drops invalid points", () => {
  const records = normalizeVesselTrack([
    { lat: 25.7, lon: -80.1, t: 1_700_000_000 },
    { lat: null, lon: -80.2, t: 1_700_000_030 },
    { lat: 25.9, lon: -80.3, t: null },
    "garbage",
  ]);
  expect(records).toHaveLength(2);
  expect(records[0]).toEqual({
    latitude: 25.7,
    longitude: -80.1,
    observedAtMs: 1_700_000_000_000,
    altitudeDatum: "sea-surface",
  });
  expect(records[1].observedAtMs).toBeNull();
  expect(normalizeVesselTrack(null)).toEqual([]);
});

test("toVesselsSnapshot maps a live envelope field-for-field (Ruling 8 freshness)", () => {
  const nowMs = NEWEST_MS + 30_000;
  const snap = toVesselsSnapshot(LIVE_ENVELOPE, 200, nowMs);
  expect(snap.records).toHaveLength(1);
  expect(snap.source).toBe("AISStream via IntelHub");
  expect(snap.coverage).toBe("received AIS positions");
  expect(snap.complete).toBe(true);
  expect(snap.rejectedCount).toBe(0);
  expect(snap.rawRowCount).toBe(1);
  expect(snap.observedAtMs).toBe(NEWEST_MS);
  expect(snap.ageMs).toBe(30_000);
  expect(snap.stale).toBe(false);
  expect(snap.freshness).toBe("current");
  expect(snap.status).toBe(200);
  expect(snap.transportStatus).toBe("live");
  expect(snap.lastMessageAt).toBe(NEWEST_ISO);
  expect(snap.nextAttemptAt).toBeNull();
  expect(snap.silentForMs).toBe(500);
  expect(snap.reconnectAttempt).toBe(0);
});

test("toVesselsSnapshot: refreshing envelopes frame stale with rejected count", () => {
  const env: AisLiveEnvelope = {
    rows: [ROW, { ...ROW, mmsi: "" }, { ...ROW, mmsi: "636092423" }],
    newestPositionAt: NEWEST_ISO,
    refreshing: true,
    status: "reconnecting",
  };
  const snap = toVesselsSnapshot(env, 200, NEWEST_MS + 60_000);
  expect(snap.records).toHaveLength(1);
  expect(snap.rejectedCount).toBe(2);
  expect(snap.rawRowCount).toBe(3);
  expect(snap.stale).toBe(true);
  expect(snap.freshness).toBe("stale");
  expect(snap.complete).toBe(false);
  expect(snap.transportStatus).toBe("reconnecting");
});

test("getSnapshot fetches ais-live with maxRows and forwards the signal", async () => {
  const apiFetch = vi.fn<ApiFetch>(async () => jsonResponse(LIVE_ENVELOPE));
  const controller = new AbortController();
  const snap = await vesselsSource(apiFetch).getSnapshot(
    { maxRows: 5000 },
    { signal: controller.signal },
  );
  expect(apiFetch).toHaveBeenCalledWith(
    "/api/v1/gev/ais-live?maxRows=5000",
    { signal: controller.signal },
  );
  expect(snap.records).toHaveLength(1);
  expect(snap.status).toBe(200);
  expect(snap.transportStatus).toBe("live");
});

test("getSnapshot: missing-key envelope resolves degraded with transportStatus", async () => {
  const apiFetch = vi.fn<ApiFetch>(async () =>
    jsonResponse({
      rows: [],
      newestPositionAt: null,
      refreshing: true,
      status: "missing-key",
      lastMessageAt: null,
      nextAttemptAt: null,
      silentForMs: null,
      reconnectAttempt: 0,
    }),
  );
  const snap = await vesselsSource(apiFetch).getSnapshot();
  expect(snap.records).toEqual([]);
  expect(snap.status).toBe(200);
  expect(snap.transportStatus).toBe("missing-key");
  expect(snap.stale).toBe(true);
  expect(snap.freshness).toBe("stale");
  expect(snap.observedAtMs).toBeNull();
  expect(snap.ageMs).toBeNull();
});

test("getSnapshot: malformed body or non-ok response degrades to 503 + empty records", async () => {
  for (const [body, status] of [
    [{}, 200],
    [{ error: "proxy html page" }, 502],
    [null, 200],
  ] as const) {
    const apiFetch = vi.fn<ApiFetch>(async () => jsonResponse(body, status));
    const snap = await vesselsSource(apiFetch).getSnapshot();
    expect(snap.records).toEqual([]);
    expect(snap.status).toBe(status === 200 ? 503 : status);
    expect(snap.freshness).toBe("unknown");
    expect(snap.observedAtMs).toBeNull();
    expect(snap.ageMs).toBeNull();
  }
});

test("getTrack fetches the per-MMSI ring and normalizes samples", async () => {
  const apiFetch = vi.fn<ApiFetch>(async () =>
    jsonResponse({ samples: [{ lat: 25.7, lon: -80.1, t: 1_700_000_000 }] }),
  );
  const controller = new AbortController();
  const track = await vesselsSource(apiFetch).getTrack("636092423", {
    signal: controller.signal,
  });
  expect(apiFetch).toHaveBeenCalledWith(
    "/api/v1/gev/ais-live/track?mmsi=636092423",
    { signal: controller.signal },
  );
  expect(track.complete).toBe(false);
  expect(track.records).toHaveLength(1);
  expect(track.records[0].latitude).toBe(25.7);
  expect(track.records[0].observedAtMs).toBe(1_700_000_000_000);
});

test("getTrack: non-ok rejects (engine tracking swallows and keeps live trail)", async () => {
  const apiFetch = vi.fn<ApiFetch>(async () => jsonResponse({ error: "unknown mmsi" }, 404));
  await expect(vesselsSource(apiFetch).getTrack("1")).rejects.toThrow(
    "IntelHub vessels HTTP 404",
  );
});

test("index.ts registers the real source, not the stub", async () => {
  const apiFetch = vi.fn<ApiFetch>(async () => jsonResponse(LIVE_ENVELOPE));
  const sources = createIntelHubLayerSources({ apiFetch });
  const vessels = sources.vessels as {
    label: string;
    getSnapshot(): Promise<{ records: unknown[]; source: string }>;
  };
  expect(vessels.label).toBe("AISStream via IntelHub");
  const env = await vessels.getSnapshot();
  expect(env.source).toBe("AISStream via IntelHub");
  expect(env.records).toHaveLength(1);
});
