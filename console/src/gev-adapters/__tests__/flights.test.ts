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
const TS = "2026-09-17T00:59:09.029094120+00:00"; // real hub format: +00:00, 9 fractional digits
const TS_MS = Date.parse(TS);
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
