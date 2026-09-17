// Explicit vitest imports (repo convention, cf. ./stubs.test.ts): vitest.config.ts
// sets globals:true for the runtime, but `tsc -b` does not load vitest/globals
// types, so bare `test`/`expect` break `npm run build`.
import { expect, test } from "vitest";
import {
  KNOTS_TO_MPS,
  toEnvelope,
  toGevRecord,
} from "../aircraft-map";
import type { AdsbPoint } from "../aircraft-map";

// Field-for-field what hub-core monitor/sources/adsb.rs `snapshot_envelope`
// emits for one row (`seen` is NOT in the JSON — the hub publishes `age_s`,
// the age of the observation in whole seconds).
const POINT: AdsbPoint = {
  hex: "A1B2C3",
  flight: "  UAL123  ",
  lat: 31.2,
  lon: 121.4,
  alt_m: 11278,
  gs: 450,
  track: 92.5,
  squawk: "2000",
  mil: false,
  age_s: 7,
};

const TS = "2026-09-17T12:00:00.000Z";
const TS_MS = Date.parse(TS);
const NOW_MS = TS_MS + 30_000;

test("KNOTS_TO_MPS is the engine's knot→m/s factor", () => {
  // Mirrors gev-engine/src/sources/live/aircraft.js:63 (speedKts * 0.514444).
  expect(KNOTS_TO_MPS).toBe(0.514444);
});

test("toGevRecord converts knots to m/s and lowercases the hex id", () => {
  const r = toGevRecord(POINT, TS_MS);
  expect(r.id).toBe("a1b2c3");
  expect(r.reference).toBe("A1B2C3");
  // 450 kt → 231.4998 m/s
  expect(r.speedMps).toBeCloseTo(231.5, 2);
  expect(r.latitude).toBe(31.2);
  expect(r.longitude).toBe(121.4);
  expect(r.courseDeg).toBe(92.5);
  expect(r.baroAltitudeM).toBe(11278);
  // adsb.lol rows carry no ground bit; the engine degrades on alt<=0 itself.
  expect(r.onGround).toBe(false);
  expect(r.positionTimeMs).toBe(TS_MS - 7000);
  expect(r.contactTimeMs).toBe(TS_MS);
});

test("toGevRecord trims the callsign and blanks to null", () => {
  expect(toGevRecord(POINT, TS_MS).callsign).toBe("UAL123");
  expect(toGevRecord({ ...POINT, flight: "   " }, TS_MS).callsign).toBeNull();
  // The hub emits a JSON null for a row with no callsign.
  expect(
    toGevRecord({ ...POINT, flight: null as unknown as string }, TS_MS).callsign,
  ).toBeNull();
});

test("toGevRecord keeps unknown kinematics null, never 0", () => {
  // Engine parity: aircraft.js:63/65 map a missing value to null, and the
  // records layer's sticky merge treats null as "hold last known value".
  // `null * 0.514444 === 0` would instead read as a parked aircraft.
  const r = toGevRecord(
    { ...POINT, gs: null, track: null, alt_m: null, age_s: null },
    TS_MS,
  );
  expect(r.speedMps).toBeNull();
  expect(r.courseDeg).toBeNull();
  expect(r.baroAltitudeM).toBeNull();
  expect(r.positionTimeMs).toBeNull();
  expect(r.contactTimeMs).toBe(TS_MS);
});

test("toGevRecord without a snapshot epoch reports no fix times", () => {
  const r = toGevRecord(POINT, null);
  expect(r.positionTimeMs).toBeNull();
  expect(r.contactTimeMs).toBeNull();
});

test("toEnvelope frames a fresh snapshot as current/200", () => {
  const records = [toGevRecord(POINT, TS_MS)];
  const env = toEnvelope(
    records,
    { ts: TS, coverage: "hotspots+mil+squawk" },
    "intelhub-adsb",
    NOW_MS,
  );
  expect(env).toEqual({
    records,
    complete: true,
    rejectedCount: 0,
    source: "intelhub-adsb",
    coverage: "hotspots+mil+squawk",
    observedAtMs: TS_MS,
    ageMs: 30_000,
    stale: false,
    freshness: "current",
    status: 200,
  });
});

test("toEnvelope frames a stale snapshot as stale/503 and drops unknown coverage", () => {
  const env = toEnvelope([], { ts: TS, stale: true }, "intelhub-adsb-mil", NOW_MS);
  expect(env.freshness).toBe("stale");
  expect(env.stale).toBe(true);
  expect(env.status).toBe(503);
  expect(env.coverage).toBeNull();
  expect(env.ageMs).toBe(30_000);
});

test("toEnvelope without a timestamp reports unknown freshness, null times", () => {
  // The hub's degraded body is exactly `{"stale":true,"aircraft":[]}` — no ts.
  // Date.parse(undefined) is NaN, which must never reach the engine as a time.
  const staleEnv = toEnvelope([], { stale: true } as never, "intelhub-adsb", NOW_MS);
  expect(staleEnv.observedAtMs).toBeNull();
  expect(staleEnv.ageMs).toBeNull();
  // `freshness === "unknown"` is the engine's own stale signal
  // (flights/ingestion.js:41, military/ingestion.js:40).
  expect(staleEnv.freshness).toBe("stale");
  expect(staleEnv.status).toBe(503);

  const tsless = toEnvelope([], {}, "intelhub-adsb", NOW_MS);
  expect(tsless.observedAtMs).toBeNull();
  expect(tsless.ageMs).toBeNull();
  expect(tsless.freshness).toBe("unknown");
  expect(tsless.status).toBe(200);
});

test("toEnvelope clamps a future snapshot epoch to a zero age", () => {
  // Engine parity: aircraft.js:95 `Math.max(0, now - observedAtMs)`.
  const env = toEnvelope([], { ts: TS }, "intelhub-adsb", TS_MS - 5_000);
  expect(env.ageMs).toBe(0);
  expect(env.freshness).toBe("current");
});

test("toEnvelope degrades an aged snapshot even without the hub stale flag", () => {
  // Ruling 8 / T6 I-2: freshness and status derive from the same stale
  // boolean, and age alone past the engine's own 120 s threshold
  // (aircraft.js:92) must degrade the envelope — a hub that serves an old
  // snapshot without setting `stale:true` must not read as fresh.
  const env = toEnvelope([], { ts: TS }, "intelhub-adsb", TS_MS + 130_000);
  expect(env.ageMs).toBe(130_000);
  expect(env.stale).toBe(true);
  expect(env.freshness).toBe("stale");
  expect(env.status).toBe(503);
  // The threshold is strict, exactly like the engine's `ageMs > 120000`.
  const atLimit = toEnvelope([], { ts: TS }, "intelhub-adsb", TS_MS + 120_000);
  expect(atLimit.stale).toBe(false);
  expect(atLimit.freshness).toBe("current");
});
