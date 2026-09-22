// Explicit vitest imports (repo convention, cf. ./stubs.test.ts): vitest.config.ts
// sets globals:true for the runtime, but `tsc -b` does not load vitest/globals
// types, so bare `test`/`expect` break `npm run build`.
import { expect, test, vi } from "vitest";
import type { ApiFetch } from "../http";
import { createIntelHubLayerSources } from "../index";
import { KNOTS_TO_MPS } from "../aircraft-map";
import { flightsSource } from "../flights";

// Field-for-field the hub envelope: `ts` + three rows sampled from the live
// `hub:globe:aircraft` snapshot on production 2026-09-17T00:59Z (hub-core
// monitor/sources/adsb.rs `snapshot_envelope`). The rows are real, including
// the nulls — adsb.lol omits `flight`/`gs`/`track`/`squawk` for rows it still
// positions (live probe: 38 null flights, 23 null gs, 60 null track, 57 null
// squawk out of 520 rows), and the hub emits them as JSON null rather than
// dropping the keys. `seen` is not in the envelope at all.
// Clock-relative epoch: the adapter now degrades AGED snapshots on its own
// (aircraft-map.ts toEnvelope, ruling 8 — ageMs > 120 s ⇒ stale), so a
// fresh-snapshot fixture must be recent at RUN time, not pinned to the probe
// moment. Row content still mirrors the live probe cited above.
const TS_MS = Date.now() - 30_000;
const TS = new Date(TS_MS).toISOString();
const AIRCRAFT = [
  {
    age_s: 243,
    alt_m: 12192,
    flight: "N3CEGE",
    gs: 448.5,
    hex: "04200a",
    lat: 37.764479,
    lon: -97.4796,
    mil: true,
    squawk: "6261",
    track: 256.99,
  },
  {
    age_s: 121,
    alt_m: 11575,
    flight: "MSR954",
    gs: 476.0,
    hex: "01025d",
    lat: 26.759483,
    lon: 50.057884,
    mil: false,
    squawk: "4735",
    track: 270.6,
  },
  {
    age_s: 131,
    alt_m: 11270,
    flight: "TQQ234",
    gs: null,
    hex: "09a06b",
    lat: 24.209631,
    lon: 50.400713,
    mil: false,
    squawk: null,
    track: null,
  },
];
const ENVELOPE = {
  ts: TS,
  count: AIRCRAFT.length,
  coverage: "hotspots+mil+squawk",
  cycle_secs: 210,
  last_tick: "mil",
  aircraft: AIRCRAFT,
};

const jsonResponse = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), { status });

test("maps every hub row into an engine observation + envelope framing", async () => {
  const apiFetch = vi.fn<ApiFetch>(async () => jsonResponse(ENVELOPE));
  const env = await flightsSource(apiFetch).getSnapshot({ bounds: "ignored" });

  expect(apiFetch).toHaveBeenCalledTimes(1);
  expect(apiFetch.mock.calls[0][0]).toBe("/api/v1/globe/aircraft");
  expect(env.records).toHaveLength(3);
  expect(env.source).toBe("intelhub-adsb");
  expect(env.coverage).toBe("hotspots+mil+squawk");
  expect(env.observedAtMs).toBe(TS_MS);
  expect(env.ageMs).toBeGreaterThanOrEqual(0);
  expect(env.stale).toBe(false);
  expect(env.freshness).toBe("current");
  expect(env.status).toBe(200);
  expect(env.complete).toBe(true);
  expect(env.rejectedCount).toBe(0);

  // Record shape = what flights/records.js `receive()` destructures.
  const [first, third] = [env.records[0], env.records[2]];
  expect(first.id).toBe("04200a");
  expect(first.reference).toBe("04200a");
  expect(first.callsign).toBe("N3CEGE");
  expect(first.speedMps).toBeCloseTo(448.5 * KNOTS_TO_MPS, 6);
  expect(first.courseDeg).toBe(256.99);
  expect(first.baroAltitudeM).toBe(12192);
  expect(first.positionTimeMs).toBe(TS_MS - 243_000);
  expect(first.contactTimeMs).toBe(TS_MS);
  expect(first.onGround).toBe(false);
  // Missing upstream values stay null so the engine's sticky merge holds the
  // last known value instead of reading a fabricated 0 kt / 0° north heading.
  expect(third.id).toBe("09a06b");
  expect(third.callsign).toBe("TQQ234");
  expect(third.speedMps).toBeNull();
  expect(third.courseDeg).toBeNull();
  expect(third.baroAltitudeM).toBe(11270);
  expect(third.positionTimeMs).toBe(TS_MS - 131_000);
});

test("forwards the abort signal to the transport", async () => {
  const apiFetch = vi.fn<ApiFetch>(async () => jsonResponse(ENVELOPE));
  const controller = new AbortController();
  await flightsSource(apiFetch).getSnapshot(undefined, {
    signal: controller.signal,
  });
  expect(apiFetch.mock.calls[0]).toEqual([
    "/api/v1/globe/aircraft",
    { signal: controller.signal },
  ]);
});

test("non-ok response throws with the status", async () => {
  const apiFetch = vi.fn<ApiFetch>(async () =>
    jsonResponse({ error: "unauthorized" }, 401),
  );
  await expect(flightsSource(apiFetch).getSnapshot()).rejects.toThrow(
    "IntelHub flights HTTP 401",
  );
});

test("the hub's degraded {stale:true,aircraft:[]} body frames as stale/503", async () => {
  const apiFetch = vi.fn<ApiFetch>(async () =>
    jsonResponse({ stale: true, aircraft: [] }),
  );
  const env = await flightsSource(apiFetch).getSnapshot();
  expect(env.records).toEqual([]);
  expect(env.stale).toBe(true);
  expect(env.freshness).toBe("stale");
  expect(env.status).toBe(503);
  // No `ts` in the degraded body: an unknown time must not become epoch 0/NaN.
  expect(env.observedAtMs).toBeNull();
  expect(env.ageMs).toBeNull();
});

test("a non-array aircraft field degrades to no records", async () => {
  for (const body of [{ ts: TS, aircraft: null }, { ts: TS }, { error: "boom" }]) {
    const apiFetch = vi.fn<ApiFetch>(async () => jsonResponse(body));
    const env = await flightsSource(apiFetch).getSnapshot();
    expect(env.records).toEqual([]);
    expect(env.observedAtMs).toBe(
      (body as { ts?: string }).ts === TS ? TS_MS : null,
    );
  }
});

test("index.ts registers the real source, not the stub", async () => {
  const apiFetch = vi.fn<ApiFetch>(async () => jsonResponse(ENVELOPE));
  const sources = createIntelHubLayerSources({ apiFetch });
  const flights = sources.flights as {
    label: string;
    getSnapshot(): Promise<{ records: unknown[]; source: string }>;
  };
  expect(flights.label).toBe("adsb.lol via IntelHub");
  const env = await flights.getSnapshot();
  expect(env.source).toBe("intelhub-adsb");
  expect(env.records).toHaveLength(3);
  expect(apiFetch).toHaveBeenCalledWith("/api/v1/globe/aircraft", {
    signal: undefined,
  });
});

// GEV P21 T1 (2026-09-23): when the hub envelope omits `ts` (production
// incident: T1 adsb.lol collector offline, `merge_globe_snapshots` seeded
// `out` from `json!({})` without restoring a timestamp), the adapter must
// still surface records with valid lat/lon so aircraft remain visible — but
// the freshness signal must clearly mark the snapshot degraded. This pins
// the graceful-degradation contract; without it, the vendor's
// `_positionHistory` collapses to a single 1970-epoch fix and aircraft freeze
// in place between polls. The Rust `merge_synthesizes_ts_when_adsb_dead`
// test is the upstream guard; this test is the downstream guard.
test("degrades to unknown freshness when hub envelope omits ts", async () => {
  const degraded = {
    count: 1,
    coverage: "adsbx",
    aircraft: [
      {
        age_s: 5,
        alt_m: 10000,
        flight: "TEST1",
        gs: 250.0,
        hex: "abc123",
        lat: 40.0,
        lon: -74.0,
        mil: false,
        squawk: null,
        track: 90.0,
      },
    ],
    // intentionally NO `ts` key
  };
  const apiFetch = vi.fn<ApiFetch>(async () => jsonResponse(degraded));
  const env = await flightsSource(apiFetch).getSnapshot();

  expect(env.records).toHaveLength(1);
  expect(env.observedAtMs).toBeNull();
  expect(env.ageMs).toBeNull();
  // "unknown" is the engine's degraded signal (flights/ingestion.js:41) — the
  // vendor's backoff path keys off it. Falling back to "stale" instead would
  // be a regression because the snapshot IS being served, just without an
  // upstream timestamp.
  expect(env.freshness).toBe("unknown");
  expect(env.stale).toBe(true);
  // Without a snapshot epoch, per-record positionTimeMs cannot be derived
  // even though `age_s` is finite — the formula is `observedAtMs − age_s·1000`
  // and the first term is null. This is exactly the state that froze aircraft
  // on the globe page in production.
  expect(env.records[0].positionTimeMs).toBeNull();
  expect(env.records[0].contactTimeMs).toBeNull();
  // Lat/lon still surface so the billboard renders; the vendor's own
  // freshness × focus × limb-haze composition marks the icon as degraded.
  expect(env.records[0].latitude).toBe(40.0);
  expect(env.records[0].longitude).toBe(-74.0);
});
