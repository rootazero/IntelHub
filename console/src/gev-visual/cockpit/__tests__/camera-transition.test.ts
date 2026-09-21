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

  it("flyToTracked flies to a lifted, south-offset, down-tilted pose for 0.7s", async () => {
    const viewer = makeViewer();
    const t = mountCockpitCameraTransition({ viewer: viewer as never });

    await t.flyToTracked(NYC);

    expect(viewer.camera.flyTo).toHaveBeenCalledTimes(1);
    const req = firstRequest(viewer);
    expect(req.duration).toBe(0.7);
    expect(req.easingFunction).toBe(Cesium.EasingFunction.QUADRATIC_IN_OUT);
    expect(req.orientation).toEqual({
      heading: 0,
      pitch: Cesium.Math.toRadians(-20),
      roll: 0,
    });
    const expected = Cesium.Cartesian3.fromDegrees(
      NYC.longitude,
      NYC.latitude - 0.5,
      NYC.altitude + 1500,
    );
    expect(Cesium.Cartesian3.equals(req.destination, expected)).toBe(true);
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
    const secondReq = viewer.camera.flyTo.mock.calls[1][0] as {
      destination: Cesium.Cartesian3;
    };
    const expected = Cesium.Cartesian3.fromDegrees(-122.4, 37.1, 11500);
    expect(Cesium.Cartesian3.equals(secondReq.destination, expected)).toBe(true);
  });
});
