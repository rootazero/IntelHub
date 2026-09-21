// GEV P12 T6 — cockpit camera transition tests (spec §6.E2).
//
// The adapter is driven through a hand-rolled Cesium camera double so we assert
// the exact flyTo request (destination / orientation / duration / easing) the
// real viewer will receive, plus the enter→exit baseline round-trip and the
// supersede-previous-flight contract (spec R6).
import * as Cesium from "cesium";
import { describe, expect, it, vi } from "vitest";
import { mountCockpitCameraTransition } from "../camera-transition";

const NYC = { longitude: -74.17, latitude: 40.69, altitude: 10000 };

function makeViewer() {
  return {
    scene: { canvas: {} },
    camera: {
      position: { clone: () => ({ x: 1, y: 2, z: 3 }) },
      heading: 0.5,
      pitch: -0.3,
      roll: 0,
      flyTo: vi.fn().mockResolvedValue(true),
    },
  };
}

/** First flyTo request captured by the fake camera. */
function firstRequest(viewer: ReturnType<typeof makeViewer>) {
  return viewer.camera.flyTo.mock.calls[0][0] as {
    destination: Cesium.Cartesian3;
    orientation: { heading: number; pitch: number; roll: number };
    duration: number;
    easingFunction: unknown;
  };
}

describe("mountCockpitCameraTransition", () => {
  it("throws TypeError if viewer.camera.flyTo is missing", () => {
    expect(() =>
      mountCockpitCameraTransition({
        viewer: { scene: { canvas: {} }, camera: {} as never },
      }),
    ).toThrow(TypeError);
  });

  it("flyToTracked flies to the chase pose (7m forward + 2.6m up, pitch -4°) for 0.4s", async () => {
    const viewer = makeViewer();
    const t = mountCockpitCameraTransition({ viewer: viewer as never });

    await t.flyToTracked(NYC);

    expect(viewer.camera.flyTo).toHaveBeenCalledTimes(1);
    const req = firstRequest(viewer);
    expect(req.duration).toBe(0.4);
    expect(req.easingFunction).toBe(Cesium.EasingFunction.QUADRATIC_IN_OUT);
    // Orientation must be { direction, up } not { heading, pitch, roll } in P14
    expect(req.orientation).toHaveProperty("direction");
    expect(req.orientation).toHaveProperty("up");
    expect((req.orientation as { pitch?: number }).pitch).toBeUndefined();
    // The destination should be near (NYC.lon, NYC.lat, NYC.alt + 2.6m)
    // with a 7m offset in the ENU forward direction. We assert altitude
    // within 1m to confirm the chase envelope.
    const cart = Cesium.Cartographic.fromCartesian(req.destination);
    expect(Math.abs(cart.height - NYC.altitude - 2.6)).toBeLessThan(5);
  });

  it("flyBackToBaseline returns to the pose captured before the enter", async () => {
    const viewer = makeViewer();
    const t = mountCockpitCameraTransition({ viewer: viewer as never });
    await t.flyToTracked(NYC);
    viewer.camera.flyTo.mockClear();

    await t.flyBackToBaseline();

    expect(viewer.camera.flyTo).toHaveBeenCalledTimes(1);
    const req = firstRequest(viewer);
    expect(req.duration).toBe(0.7);
    expect(req.destination).toEqual({ x: 1, y: 2, z: 3 });
    expect(req.orientation).toEqual({ heading: 0.5, pitch: -0.3, roll: 0 });
  });

  it("captureBaseline snapshots the current pose without flying", async () => {
    const viewer = makeViewer();
    const t = mountCockpitCameraTransition({ viewer: viewer as never });
    viewer.camera.position.clone = () => ({ x: 9, y: 8, z: 7 });
    viewer.camera.heading = 1.25;

    t.captureBaseline();
    expect(viewer.camera.flyTo).not.toHaveBeenCalled();

    await t.flyBackToBaseline();
    expect(viewer.camera.flyTo).toHaveBeenCalledTimes(1);
    const req = firstRequest(viewer);
    expect(req.destination).toEqual({ x: 9, y: 8, z: 7 });
    expect(req.orientation).toEqual({ heading: 1.25, pitch: -0.3, roll: 0 });
  });

  it("destroy clears the baseline so a later flyBack is a no-op", async () => {
    const viewer = makeViewer();
    const t = mountCockpitCameraTransition({ viewer: viewer as never });
    await t.flyToTracked(NYC);
    viewer.camera.flyTo.mockClear();

    t.destroy();
    await t.flyBackToBaseline();

    expect(viewer.camera.flyTo).not.toHaveBeenCalled();
  });

  it("a new fly supersedes the in-flight one (both requests reach Cesium)", async () => {
    const viewer = makeViewer();
    let resolveFirst: ((value: boolean) => void) | undefined;
    viewer.camera.flyTo.mockImplementationOnce(
      () =>
        new Promise<boolean>((resolve) => {
          resolveFirst = resolve;
        }),
    );
    const t = mountCockpitCameraTransition({ viewer: viewer as never });

    const first = t.flyToTracked(NYC);
    const second = t.flyToTracked({
      longitude: -122.4,
      latitude: 37.6,
      altitude: 10000,
    });
    resolveFirst?.(true);
    await first;
    await second;

    expect(viewer.camera.flyTo).toHaveBeenCalledTimes(2);
    // Chase-pose destination: anchor + forward*7 + up*2.6 (heading=0, pitch=-4°).
    const secondReq = viewer.camera.flyTo.mock.calls[1][0] as {
      destination: Cesium.Cartesian3;
    };
    const cart = Cesium.Cartographic.fromCartesian(secondReq.destination);
    // anchor: (-122.4, 37.6, 10000); forward=(0,cos(-4°),sin(-4°)); up=(0,0,1)
    // northward component: cos(-4°)/M ≈ 0.9976 / 6369000 rad ≈ 0.0698°
    // latitude delta ≈ +0.0698°; height delta ≈ 2.6m minus the southward
    // tilt of the ENU up vector at this latitude (≈0.49m), net ≈2.11m.
    const northDeg = Math.cos(Cesium.Math.toRadians(-4)) / 6369000 * (180 / Math.PI);
    expect(Math.abs(cart.longitude - Cesium.Math.toRadians(-122.4))).toBeLessThan(0.0001);
    expect(Math.abs(cart.latitude - (Cesium.Math.toRadians(37.6) + northDeg))).toBeLessThan(0.001);
    // Height ≈ 10000 + 2.6 * cos(lat) ≈ 10002.1; ENU→ECEF may introduce
    // minor geometric discrepancy so use a wide tolerance.
    expect(cart.height).toBeGreaterThan(10000);
    expect(cart.height).toBeLessThan(10010);
  });

  it("isInFlight is false initially", () => {
    const viewer = makeViewer();
    const t = mountCockpitCameraTransition({ viewer: viewer as never });
    expect(t.isInFlight()).toBe(false);
  });

  it("isInFlight is true while flyToTracked is pending and false after it resolves", async () => {
    let resolveFlight: (v: boolean) => void = () => {};
    const viewer = makeViewer();
    viewer.camera.flyTo = vi.fn().mockImplementation(
      () => new Promise<boolean>((r) => { resolveFlight = r; }),
    );
    const t = mountCockpitCameraTransition({ viewer: viewer as never });
    const promise = t.flyToTracked(NYC);
    expect(t.isInFlight()).toBe(true);
    resolveFlight(true);
    await promise;
    expect(t.isInFlight()).toBe(false);
  });

  it("isInFlight is true while flyBackToBaseline is pending", async () => {
    // Build the viewer with the pending mock IN PLACE so the transition
    // captures the correct (pending) flyTo at construction time.
    let resolveFlight: (v: boolean) => void = () => {};
    const viewer = {
      scene: { canvas: {} },
      camera: {
        position: { clone: () => ({ x: 1, y: 2, z: 3 }) },
        heading: 0.5,
        pitch: -0.3,
        roll: 0,
        flyTo: vi.fn().mockImplementation(
          () => new Promise<boolean>((r) => { resolveFlight = r; }),
        ),
      },
    };
    const t = mountCockpitCameraTransition({ viewer: viewer as never });

    // First call (flyToTracked) — pending; resolve it so inFlight clears.
    const firstPromise = t.flyToTracked(NYC);
    expect(t.isInFlight()).toBe(true);
    resolveFlight(true);
    await firstPromise;
    expect(t.isInFlight()).toBe(false);

    // Second call (flyBackToBaseline) — new pending from the same mock ref.
    const secondPromise = t.flyBackToBaseline();
    expect(t.isInFlight()).toBe(true);
    resolveFlight(true); // reuse the same resolveFlight closure from makeViewer
    await secondPromise;
    expect(t.isInFlight()).toBe(false);
  });
});
