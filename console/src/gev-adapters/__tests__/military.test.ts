// Explicit vitest imports (repo convention, cf. ./stubs.test.ts): vitest.config.ts
// sets globals:true for the runtime, but `tsc -b` does not load vitest/globals
// types, so bare `test`/`expect` break `npm run build`.
import { expect, test, vi } from "vitest";
import type { ApiFetch } from "../http";
import { createIntelHubLayerSources } from "../index";
import { militarySource } from "../military";

// Same hub envelope as ./flights.test.ts — one Redis snapshot feeds both layers;
// only the `mil` bit separates them (hub-core adsb.rs reads it from adsb.lol's
// `dbFlags & 1`). Rows are real, sampled from the live production snapshot on
// 2026-09-17T00:59Z (223 of its 520 rows carried `mil: true`).
const TS = "2026-09-17T00:59:09.029094120+00:00";
const TS_MS = Date.parse(TS);
const AIRCRAFT = [
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
    age_s: 277,
    alt_m: 0,
    flight: null,
    gs: 7.8,
    hex: "14fc0d",
    lat: 55.97483,
    lon: 37.436085,
    mil: true,
    squawk: null,
    track: 75.94,
  },
];
const ENVELOPE = {
  ts: TS,
  count: AIRCRAFT.length,
  coverage: "hotspots+mil+squawk",
  cycle_secs: 210,
  last_tick: "squawk7700",
  aircraft: AIRCRAFT,
};

const jsonResponse = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), { status });

test("keeps only mil===true rows and frames them under the mil source id", async () => {
  const apiFetch = vi.fn<ApiFetch>(async () => jsonResponse(ENVELOPE));
  const env = await militarySource(apiFetch).getSnapshot();

  expect(apiFetch).toHaveBeenCalledTimes(1);
  expect(apiFetch.mock.calls[0][0]).toBe("/api/v1/globe/aircraft");
  expect(env.records.map((r) => r.id)).toEqual(["04200a", "14fc0d"]);
  expect(env.source).toBe("intelhub-adsb-mil");
  expect(env.coverage).toBe("hotspots+mil+squawk");
  expect(env.observedAtMs).toBe(TS_MS);
  expect(env.stale).toBe(false);
  expect(env.freshness).toBe("current");
  expect(env.status).toBe(200);
  // Records share the flights observation shape (military/records.js reads the
  // same fields), including the knots→m/s conversion.
  expect(env.records[0].callsign).toBe("N3CEGE");
  expect(env.records[0].speedMps).toBeCloseTo(448.5 * 0.514444, 6);
  expect(env.records[0].positionTimeMs).toBe(TS_MS - 243_000);
  // A mil row with a null callsign must not become an empty-string identity.
  expect(env.records[1].callsign).toBeNull();
});

test("forwards the abort signal to the transport", async () => {
  const apiFetch = vi.fn<ApiFetch>(async () => jsonResponse(ENVELOPE));
  const controller = new AbortController();
  await militarySource(apiFetch).getSnapshot(undefined, {
    signal: controller.signal,
  });
  expect(apiFetch.mock.calls[0]).toEqual([
    "/api/v1/globe/aircraft",
    { signal: controller.signal },
  ]);
});

test("non-ok response throws with the status", async () => {
  const apiFetch = vi.fn<ApiFetch>(async () =>
    jsonResponse({ error: "bad gateway" }, 502),
  );
  await expect(militarySource(apiFetch).getSnapshot()).rejects.toThrow(
    "IntelHub military HTTP 502",
  );
});

test("degraded / empty snapshots yield no military records", async () => {
  for (const body of [
    { stale: true, aircraft: [] },
    { ts: TS, aircraft: [] },
    { ts: TS, aircraft: [{ hex: "A1B2C3", lat: 1, lon: 2, mil: false }] },
  ]) {
    const apiFetch = vi.fn<ApiFetch>(async () => jsonResponse(body));
    const env = await militarySource(apiFetch).getSnapshot();
    expect(env.records).toEqual([]);
  }
  const staleApiFetch = vi.fn<ApiFetch>(async () =>
    jsonResponse({ stale: true, aircraft: [] }),
  );
  const env = await militarySource(staleApiFetch).getSnapshot();
  expect(env.freshness).toBe("stale");
  expect(env.status).toBe(503);
  expect(env.observedAtMs).toBeNull();
});

test("index.ts registers the real source, not the stub", async () => {
  const apiFetch = vi.fn<ApiFetch>(async () => jsonResponse(ENVELOPE));
  const sources = createIntelHubLayerSources({ apiFetch });
  const military = sources.military as {
    label: string;
    getSnapshot(): Promise<{ records: { id: string }[]; source: string }>;
  };
  expect(military.label).toBe("adsb.lol mil via IntelHub");
  const env = await military.getSnapshot();
  expect(env.source).toBe("intelhub-adsb-mil");
  expect(env.records.map((r) => r.id)).toEqual(["04200a", "14fc0d"]);
});
