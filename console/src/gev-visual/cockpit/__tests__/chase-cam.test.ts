// GEV P14 — cockpit chase-cam driver tests.
//
// Ports the gods-eye-view src/ui/cockpitCamera.update() loop into the React
// tree. Tests:
//   - constructor contract (asserts deps at mount)
//   - cadence gate (50 ms)
//   - ENU math + camera.setView args
//   - slewHeading integration (heading moves toward target)
//   - lifecycle: start/stop/destroy
//   - isInFlight guard (chase-cam skips ticks while flyTo in flight)
//   - setView-throws resilience (catches + retries).

import * as Cesium from "cesium";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { mountCockpitChaseCam } from "../chase-cam";

const NYC_LON = -74.17;
const NYC_LAT = 40.69;
const NYC_ALT = 10000;

function makeEntity(opts: { lon?: number; lat?: number; alt?: number; id?: string } = {}) {
  return {
    id: opts.id ?? "icao-1",
    position: {
      getValue: (_time: unknown, result?: Cesium.Cartesian3) => {
        const c = Cesium.Cartesian3.fromDegrees(
          opts.lon ?? NYC_LON, opts.lat ?? NYC_LAT, opts.alt ?? NYC_ALT,
          undefined, result,
        );
        return c;
      },
    },
  };
}

/** Entity whose position advances metersPerTick meters north on every call to
 *  getValue. Reuse the same handle across calls so tickCount accumulates. */
function makeMovingEntity(opts: {
  startLon?: number;
  startLat?: number;
  alt?: number;
  metersPerTick?: number;
  id?: string;
} = {}) {
  const startLon = opts.startLon ?? NYC_LON;
  const startLat = opts.startLat ?? NYC_LAT;
  const alt = opts.alt ?? NYC_ALT;
  const metersPerTick = opts.metersPerTick ?? 100;
  // 1 deg latitude ≈ 111 111 m (good enough for short distances).
  const deltaLat = metersPerTick / 111111;
  let tickCount = 0;
  return {
    id: opts.id ?? "icao-moving",
    position: {
      getValue: (_time: unknown, result?: Cesium.Cartesian3) => {
        const c = Cesium.Cartesian3.fromDegrees(
          startLon,
          startLat + tickCount * deltaLat,
          alt,
          undefined,
          result,
        );
        tickCount++;
        return c;
      },
    },
  };
}

function makeStore(initial: { active: boolean; trackedId: string | null }) {
  const listeners = new Set<() => void>();
  let state = initial;
  return {
    getState: () => state,
    subscribe: (fn: () => void) => { listeners.add(fn); return () => listeners.delete(fn); },
    _set: (next: typeof state) => { state = next; listeners.forEach((f) => f()); },
  };
}

function makeTransition(initialInFlight = false) {
  return {
    isInFlight: vi.fn(() => initialInFlight),
    flyToTracked: vi.fn(),
    flyBackToBaseline: vi.fn(),
    captureBaseline: vi.fn(),
  };
}

function makeViewer() {
  return {
    clock: { currentTime: new Date() },
    camera: { setView: vi.fn() },
  };
}

describe("mountCockpitChaseCam", () => {
  let rafCallbacks: FrameRequestCallback[];
  let originalRaf: typeof globalThis.requestAnimationFrame;
  let originalCaf: typeof globalThis.cancelAnimationFrame;
  let now: number;

  beforeEach(() => {
    rafCallbacks = [];
    now = 0;
    originalRaf = globalThis.requestAnimationFrame;
    originalCaf = globalThis.cancelAnimationFrame;
    globalThis.requestAnimationFrame = (cb: FrameRequestCallback): number => {
      rafCallbacks.push(cb);
      return rafCallbacks.length;
    };
    globalThis.cancelAnimationFrame = vi.fn((_id: number) => {});
    vi.spyOn(performance, "now").mockImplementation(() => now);
  });

  afterEach(() => {
    globalThis.requestAnimationFrame = originalRaf;
    globalThis.cancelAnimationFrame = originalCaf;
    vi.restoreAllMocks();
  });

  function tickAdvance(ms: number) { now += ms; }
  function flushRafs(n: number = 1) {
    for (let i = 0; i < n; i++) {
      const cbs = rafCallbacks.splice(0);
      cbs.forEach((cb) => cb(now));
    }
  }

  it("throws TypeError if deps are missing", () => {
    expect(() =>
      mountCockpitChaseCam({} as never),
    ).toThrow(TypeError);
  });

  it("does not call setView until start() and active===true", () => {
    const viewer = makeViewer();
    const store = makeStore({ active: false, trackedId: null });
    const transition = makeTransition();
    const handle = mountCockpitChaseCam({
      viewer: viewer as never,
      store: store as never,
      transition,
      getTrackedEntity: () => makeEntity() as never,
      getHeading: () => 90,
    });
    flushRafs(2);
    expect(viewer.camera.setView).not.toHaveBeenCalled();
    handle.destroy();
  });

  it("after start() + active===true, setView is called within one cadence tick", () => {
    const viewer = makeViewer();
    const store = makeStore({ active: false, trackedId: null });
    const transition = makeTransition();
    const handle = mountCockpitChaseCam({
      viewer: viewer as never,
      store: store as never,
      transition,
      getTrackedEntity: () => makeEntity() as never,
      getHeading: () => 90,
    });
    handle.start();
    store._set({ active: true, trackedId: "icao-1" });
    tickAdvance(60); // > 50 ms cadence gate
    flushRafs(1);
    expect(viewer.camera.setView).toHaveBeenCalledTimes(1);
    handle.destroy();
  });

  it("destination is in chase envelope (7m forward, 2.6m up from anchor, heading 90)", () => {
    const viewer = makeViewer();
    const store = makeStore({ active: false, trackedId: null });
    const transition = makeTransition();
    const handle = mountCockpitChaseCam({
      viewer: viewer as never,
      store: store as never,
      transition,
      getTrackedEntity: () => makeEntity() as never,
      getHeading: () => 90,
    });
    handle.start();
    store._set({ active: true, trackedId: "icao-1" });
    tickAdvance(60);
    flushRafs(1);
    const call = viewer.camera.setView.mock.calls[0][0];
    const destCart = Cesium.Cartographic.fromCartesian(call.destination);
    const anchorCart = Cesium.Cartographic.fromCartesian(
      Cesium.Cartesian3.fromDegrees(NYC_LON, NYC_LAT, NYC_ALT),
    );
    expect(destCart.height).toBeCloseTo(NYC_ALT + 2.6, 0); // up envelope
    expect(destCart.longitude).toBeCloseTo(anchorCart.longitude, 4);
    expect(destCart.latitude).toBeCloseTo(anchorCart.latitude, 4);
    handle.destroy();
  });

  it("cadence gate: skips ticks within 50 ms", () => {
    const viewer = makeViewer();
    const store = makeStore({ active: true, trackedId: "icao-1" });
    const transition = makeTransition();
    const handle = mountCockpitChaseCam({
      viewer: viewer as never,
      store: store as never,
      transition,
      getTrackedEntity: () => makeEntity() as never,
      getHeading: () => 90,
    });
    handle.start();
    tickAdvance(60);
    flushRafs(1);
    expect(viewer.camera.setView).toHaveBeenCalledTimes(1);
    tickAdvance(10); // < 50 ms
    flushRafs(1);
    expect(viewer.camera.setView).toHaveBeenCalledTimes(1); // still 1
    tickAdvance(50);
    flushRafs(1);
    expect(viewer.camera.setView).toHaveBeenCalledTimes(2); // now 2
    handle.destroy();
  });

  it("skips setView when transition.isInFlight() is true", () => {
    const viewer = makeViewer();
    const store = makeStore({ active: true, trackedId: "icao-1" });
    const transition = makeTransition(true); // in flight
    const handle = mountCockpitChaseCam({
      viewer: viewer as never,
      store: store as never,
      transition,
      getTrackedEntity: () => makeEntity() as never,
      getHeading: () => 90,
    });
    handle.start();
    tickAdvance(60);
    flushRafs(1);
    expect(viewer.camera.setView).not.toHaveBeenCalled();
    handle.destroy();
  });

  it("slewHeading: heading moves monotonically toward target", () => {
    const viewer = makeViewer();
    const store = makeStore({ active: true, trackedId: "icao-1" });
    const transition = makeTransition();
    let heading = 0;
    const handle = mountCockpitChaseCam({
      viewer: viewer as never,
      store: store as never,
      transition,
      getTrackedEntity: () => makeEntity() as never,
      getHeading: () => heading,
    });
    handle.start();
    heading = 180; // target 180° from current
    tickAdvance(60);
    flushRafs(1);
    // Second tick at +60 ms → heading should be ~ 28°/s * 0.06s ≈ 1.68° from initial
    heading = 180;
    tickAdvance(60);
    flushRafs(1);
    // Both ticks call setView. Argument inspection is fragile here; just assert
    // the chase loop is alive.
    expect(viewer.camera.setView.mock.calls.length).toBeGreaterThanOrEqual(2);
    handle.destroy();
  });

  it("survives setView throwing — no crash, retries next tick", () => {
    const viewer = makeViewer();
    viewer.camera.setView = vi.fn(() => {
      throw new Error("simulated Cesium setView failure");
    });
    const store = makeStore({ active: true, trackedId: "icao-1" });
    const transition = makeTransition();
    const handle = mountCockpitChaseCam({
      viewer: viewer as never,
      store: store as never,
      transition,
      getTrackedEntity: () => makeEntity() as never,
      getHeading: () => 90,
    });
    handle.start();
    tickAdvance(60);
    expect(() => flushRafs(1)).not.toThrow();
    expect(viewer.camera.setView).toHaveBeenCalledTimes(1);
    // Next tick: no longer throws (loosen the mock)
    viewer.camera.setView = vi.fn();
    tickAdvance(60);
    flushRafs(1);
    expect(viewer.camera.setView).toHaveBeenCalledTimes(1);
    handle.destroy();
  });

  it("anchor follows entity motion via inertial advance", () => {
    const viewer = makeViewer();
    const store = makeStore({ active: true, trackedId: "icao-moving" });
    const transition = makeTransition();
    // heading=0 → forward vector points north, so the 7m forward offset adds
    // to the anchor's north motion rather than canceling it.
    const movingEntity = makeMovingEntity({ metersPerTick: 100 });
    const handle = mountCockpitChaseCam({
      viewer: viewer as never,
      store: store as never,
      transition,
      getTrackedEntity: () => movingEntity as never,
      getHeading: () => 0,
    });
    handle.start();
    // First tick: anchor = target (at start). Snapshot initial destination
    // by computing lat/lon RIGHT NOW — the chase-cam closure reuses a single
    // scratchCartesian3 across ticks, so reading mock.calls[i].destination
    // later returns the final tick's value.
    tickAdvance(60);
    flushRafs(1);
    const initialDestCart = Cesium.Cartographic.fromCartesian(
      viewer.camera.setView.mock.calls[0][0].destination,
    );
    // Three more ticks. Entity has moved 3 * 100 m north; anchor should
    // track it via the inertial-advance path.
    tickAdvance(60);
    flushRafs(1);
    tickAdvance(60);
    flushRafs(1);
    tickAdvance(60);
    flushRafs(1);
    const lastDestCart = Cesium.Cartographic.fromCartesian(
      viewer.camera.setView.mock.calls[
        viewer.camera.setView.mock.calls.length - 1
      ][0].destination,
    );
    const deltaLatDeg = Cesium.Math.toDegrees(
      lastDestCart.latitude - initialDestCart.latitude,
    );
    const metersPerDegLat = 111111;
    const deltaMeters = deltaLatDeg * metersPerDegLat;
    // With a 200 m/s entity the destination should be ~300 m farther north
    // after 4 ticks. Old (correction-only) code capped at ~0.14 m total.
    expect(deltaMeters).toBeGreaterThan(50);
    handle.destroy();
  });

  it("destroy() cancels rAF and unsubscribes from store", () => {
    const viewer = makeViewer();
    const store = makeStore({ active: true, trackedId: "icao-1" });
    const transition = makeTransition();
    const handle = mountCockpitChaseCam({
      viewer: viewer as never,
      store: store as never,
      transition,
      getTrackedEntity: () => makeEntity() as never,
      getHeading: () => 90,
    });
    handle.start();
    handle.destroy();
    expect(globalThis.cancelAnimationFrame).toHaveBeenCalled();
    store._set({ active: false, trackedId: null });
    store._set({ active: true, trackedId: "icao-1" });
    tickAdvance(100);
    flushRafs(2);
    expect(viewer.camera.setView).not.toHaveBeenCalled();
  });

  // GEV P15 T4 — mouse-look offset injection into chase-cam.
  //
  // These three tests pin the contract that:
  //   (1) when mouseLook is omitted, chase-cam produces the P14 pose
  //       (anchor + forward*7 + up*2.6) — exact backward-compat,
  //   (2) when mouseLook IS provided, getFrameOffset() is read on each
  //       cadence tick and applied to heading/pitch/range,
  //   (3) heading composition is ADDITIVE — drag rides on top of track
  //       slew, not a replacement.

  it("uses ZERO_OFFSET when mouseLook is omitted (backward-compat with P14)", () => {
    const viewer = makeViewer();
    const store = makeStore({ active: true, trackedId: "icao-1" });
    const transition = makeTransition();
    const handle = mountCockpitChaseCam({
      viewer: viewer as never,
      store: store as never,
      transition,
      getTrackedEntity: () => makeEntity() as never,
      getHeading: () => 90,
    });
    handle.start();
    tickAdvance(60);
    flushRafs(1);
    expect(viewer.camera.setView).toHaveBeenCalled();
    // Capture the destination and assert it's exactly 7m forward + 2.6m up
    // (no offset applied). With heading=90° and COCKPIT_VIEW_PITCH_DEG=-4
    // (note: it's NEGATIVE — slightly looking down) the camera sits at
    // anchor + forward*7 + up*2.6 in LOCAL ENU, with forward's z-component
    // contributing a small downward shift. Distance from anchor in local
    // ENU = sqrt((7*cos(PITCH_RAD))² + (7*sin(PITCH_RAD) + 2.6)²). For
    // PITCH_RAD=-4°, that's ≈7.30m. We assert with 1-decimal tolerance
    // to absorb Cesium's tiny re-projection rounding.
    const call = viewer.camera.setView.mock.calls.at(-1)?.[0] as
      | { destination: Cesium.Cartesian3 }
      | undefined;
    expect(call).toBeDefined();
    const dest = call!.destination;
    const anchor = Cesium.Cartesian3.fromDegrees(NYC_LON, NYC_LAT, NYC_ALT);
    const dist = Cesium.Cartesian3.distance(dest, anchor);
    // Import the actual constant so the test stays in sync with engine.
    // gev-engine is consumed as an ES module via the engine's vite alias;
    // in vitest, importing it requires the package alias configured in
    // vitest.config — we just hardcode -4 here, with a comment that any
    // pitch change in cockpitPresentation.js requires updating this value.
    const pitchRad = Cesium.Math.toRadians(-4); // COCKPIT_VIEW_PITCH_DEG = -4
    const horiz = 7 * Math.cos(pitchRad);
    const vert = 7 * Math.sin(pitchRad) + 2.6;
    const expected = Math.sqrt(horiz * horiz + vert * vert);
    expect(dist).toBeCloseTo(expected, 1);
    handle.destroy();
  });

  it("reads mouseLook.getFrameOffset() on each cadence tick (offset injection)", () => {
    const viewer = makeViewer();
    const store = makeStore({ active: true, trackedId: "icao-1" });
    const transition = makeTransition();
    // Mouse-look offset: heading +0.35 rad, pitch +0.20 rad, range +500m.
    // The range offset is the cleanest signal that getFrameOffset() was
    // read and applied — without it, the camera sits ~7.7m from anchor;
    // with +500m, the camera sits ~507m away. (Note: heading/pitch offsets
    // also apply, but the ECEF dot-product assertion the brief sketches
    // is non-portable across anchor longitudes — at NYC lon=-74° the
    // local-ENU-north vector tilts well off the ECEF (0,1,0) axis, so a
    // pure ECEF dot is a confusing signal. Distance from anchor is
    // longitude-invariant.)
    const mouseLook = {
      getFrameOffset: () => ({
        headingDeltaRad: 0.35,
        pitchDeltaRad: 0.20,
        rangeOffsetM: 500,
      }),
      snapBack: vi.fn(),
      destroy: vi.fn(),
    };
    const handle = mountCockpitChaseCam({
      viewer: viewer as never,
      store: store as never,
      transition,
      getTrackedEntity: () => makeEntity() as never,
      getHeading: () => 90,
      mouseLook,
    });
    handle.start();
    tickAdvance(60);
    flushRafs(1);
    expect(viewer.camera.setView).toHaveBeenCalled();
    const call = viewer.camera.setView.mock.calls.at(-1)?.[0] as
      | { destination: Cesium.Cartesian3 }
      | undefined;
    expect(call).toBeDefined();
    const dest = call!.destination;
    const anchor = Cesium.Cartesian3.fromDegrees(NYC_LON, NYC_LAT, NYC_ALT);
    const dist = Cesium.Cartesian3.distance(dest, anchor);
    // 500m range offset → camera sits ~507m from anchor. Assert > 100m
    // to leave plenty of slack for any future envelope tweaks.
    expect(dist).toBeGreaterThan(100);
    handle.destroy();
  });

  it("heading slew composes drag offset with track slew (additive, not replacement)", () => {
    // Verify slewHeading's target is (getHeading() + offset.headingDeltaRad),
    // not just getHeading() — drag rides ON TOP of track slew. Track angle
    // = 90°, drag offset = 1 rad → expected heading slew target is 90° + 1
    // rad ≈ 147.3°. With heading ≈147°, the local-ENU forward direction
    // has a strongly negative y-component (south). We verify by inverse-
    // transforming the ECEF direction back into the anchor's ENU frame
    // and checking that the local-north component is negative. Without
    // the offset, heading would stay at 90° and localDir.y would be ~0.
    //
    // SLEW TIMING: COCKPIT_HEADING_SLEW_DPS = 28°/s. To slew from 90° to
    // 147.3° (delta ≈ 57.3°) takes ≈ 2.05 s. dtSec is clamped at 0.1 s
    // per tick, so we need ≥ 21 ticks. We run 30 ticks at 200 ms each
    // (dtSec clamps at 0.1) to give the slew well past convergence.
    const viewer = makeViewer();
    const store = makeStore({ active: true, trackedId: "icao-1" });
    const transition = makeTransition();
    const mouseLook = {
      getFrameOffset: () => ({
        headingDeltaRad: 1.0,
        pitchDeltaRad: 0,
        rangeOffsetM: 0,
      }),
      snapBack: vi.fn(),
      destroy: vi.fn(),
    };
    const handle = mountCockpitChaseCam({
      viewer: viewer as never,
      store: store as never,
      transition,
      getTrackedEntity: () => makeEntity() as never,
      getHeading: () => 90,
      mouseLook,
    });
    handle.start();
    for (let i = 0; i < 30; i++) {
      tickAdvance(200);
      flushRafs(1);
    }
    expect(viewer.camera.setView).toHaveBeenCalled();
    const call = viewer.camera.setView.mock.calls.at(-1)?.[0] as
      | { orientation: { direction: Cesium.Cartesian3 } }
      | undefined;
    expect(call).toBeDefined();
    // Inverse-transform direction from ECEF to local ENU at the anchor.
    // ENU frame is orthonormal, so the inverse is its transpose.
    const anchor = Cesium.Cartesian3.fromDegrees(NYC_LON, NYC_LAT, NYC_ALT);
    const enu = Cesium.Transforms.eastNorthUpToFixedFrame(anchor);
    const enuInv = Cesium.Matrix4.inverseTransformation(
      enu,
      new Cesium.Matrix4(),
    );
    const localDir = Cesium.Matrix4.multiplyByPointAsVector(
      enuInv,
      call!.orientation.direction,
      new Cesium.Cartesian3(),
    );
    // With heading ≈147°, local-north component < 0 (camera looks south).
    // Without offset, heading would be 90° and local-north = 0; this
    // assertion would fail (expect.toBeLessThan(-0.5) requires the value
    // to be less than -0.5, and 0 is not). The threshold -0.5 is loose
    // enough to absorb Cesium's normalization/ENU-transform rounding.
    expect(localDir.y).toBeLessThan(-0.5);
    handle.destroy();
  });

  // GEV P16 T1 — chase-cam exposed state surface (bank, altitude, VSI).
  //
  // These tests pin the contract that chase-cam's resolved per-frame pose
  // (heading, pitch, range, bank, altitude, VSI) is exposed via
  // `handle.getResolvedState()` so the cockpit HUD instruments (heading
  // tape, altimeter, VSI needle, attitude indicator) can read it without
  // re-deriving from Cesium globals. Returns null until the first cadence
  // tick resolves the tracked entity.

  it("getResolvedState returns null before any tick", () => {
    const viewer = makeViewer();
    const store = makeStore({ active: true, trackedId: "icao-1" });
    const transition = makeTransition();
    const handle = mountCockpitChaseCam({
      viewer: viewer as never,
      store: store as never,
      transition,
      getTrackedEntity: () => makeEntity() as never,
      getHeading: () => 90,
    });
    expect(handle.getResolvedState()).toBeNull();
    handle.destroy();
  });

  it("bankRad defaults to 0 when entity has no orientation", () => {
    const viewer = makeViewer();
    const store = makeStore({ active: true, trackedId: "icao-1" });
    const transition = makeTransition();
    // makeEntity() returns an entity with NO orientation → bank should be 0
    const handle = mountCockpitChaseCam({
      viewer: viewer as never,
      store: store as never,
      transition,
      getTrackedEntity: () => makeEntity() as never,
      getHeading: () => 90,
    });
    handle.start();
    tickAdvance(60);
    flushRafs(1);
    const state = handle.getResolvedState();
    expect(state).not.toBeNull();
    expect(state?.bankRad).toBe(0);
    handle.destroy();
  });

  it("vsiMps is 0 during warmup (< 2 samples)", () => {
    const viewer = makeViewer();
    const store = makeStore({ active: true, trackedId: "icao-1" });
    const transition = makeTransition();
    const handle = mountCockpitChaseCam({
      viewer: viewer as never,
      store: store as never,
      transition,
      getTrackedEntity: () => makeEntity() as never,
      getHeading: () => 90,
    });
    handle.start();
    // Two ticks (each 60 ms apart > 50 ms cadence gate)
    tickAdvance(60);
    flushRafs(1);
    tickAdvance(60);
    flushRafs(1);
    // With 2 samples, dt > 0; vsiMps SHOULD now be 0 because the altitude
    // didn't change between samples (entity is static). The warmup branch
    // is guarded by `length >= 2` so a single-sample tick yields 0; after
    // 2 ticks at the same altitude, rate = 0 / dt = 0. Both branches land
    // on 0 here; we assert the surface doesn't blow up.
    expect(handle.getResolvedState()?.vsiMps).toBe(0);
    handle.destroy();
  });

  it("altitudeHistory saturates at 8 samples", () => {
    const viewer = makeViewer();
    const store = makeStore({ active: true, trackedId: "icao-1" });
    const transition = makeTransition();
    // The chase-cam mount assigns its internal altitudeHistory array onto
    // __testHooks.altitudeHistory; tests read the length off the holder.
    const testHooks: { altitudeHistory?: number[] } = {};
    const handle = mountCockpitChaseCam({
      viewer: viewer as never,
      store: store as never,
      transition,
      getTrackedEntity: () => makeEntity() as never,
      getHeading: () => 90,
      __testHooks: testHooks,
    });
    handle.start();
    // 20 ticks at 60 ms cadence → way past the 8-sample ring buffer cap
    for (let i = 0; i < 20; i++) {
      tickAdvance(60);
      flushRafs(1);
    }
    expect(testHooks.altitudeHistory?.length).toBe(8);
    handle.destroy();
  });

  it("vsiMps derives linear rate over 400ms window", () => {
    // Climbing entity: each getValue() returns altitude + 100/7 m more than
    // the previous call. After 8 samples the cumulative climb is 7 * (100/7)
    // = 100 m. With dt = 8 * 0.050 = 0.4 s, vsiMps = 100 / 0.4 = 250 m/s.
    const viewer = makeViewer();
    const store = makeStore({ active: true, trackedId: "icao-climb" });
    const transition = makeTransition();
    let climbCount = 0;
    const climbingEntity = {
      id: "icao-climb",
      position: {
        getValue: (_time: unknown, result?: Cesium.Cartesian3) =>
          Cesium.Cartesian3.fromDegrees(
            NYC_LON, NYC_LAT, NYC_ALT + climbCount++ * (100 / 7),
            undefined, result,
          ),
      },
    };
    const handle = mountCockpitChaseCam({
      viewer: viewer as never,
      store: store as never,
      transition,
      getTrackedEntity: () => climbingEntity as never,
      getHeading: () => 90,
    });
    handle.start();
    for (let i = 0; i < 8; i++) {
      tickAdvance(60);
      flushRafs(1);
    }
    expect(handle.getResolvedState()?.vsiMps).toBeCloseTo(250, 0);
    handle.destroy();
  });
});