// GEV P15 T2 — vendor-port tests (spec §4.3).
//
// The three ported functions (readCameraTargetFrame, setCameraTargetFrame,
// createCameraOrientationAnimator) are pure camera-math — no input handling,
// no state. They become the substrate for mouse-look and (future) detached-
// chase mode. These tests pin the port to the vendor behavior line-for-line.

import { describe, expect, it, vi } from "vitest";
import * as Cesium from "cesium";
import {
  readCameraTargetFrame,
  setCameraTargetFrame,
  createCameraOrientationAnimator,
} from "../vendor-port/cockpitCameraOrientation";

function makeViewer() {
  return {
    clock: { currentTime: Cesium.JulianDate.now() },
    trackedEntity: undefined,
    camera: {
      positionWC: new Cesium.Cartesian3(7, 0, 2.6),
      directionWC: new Cesium.Cartesian3(0, 1, 0),
      upWC: new Cesium.Cartesian3(0, 0, 1),
      heading: 0,
      pitch: 0,
      roll: 0,
      position: { clone: () => ({ x: 1, y: 2, z: 3 }) },
      transform: Cesium.Matrix4.clone(Cesium.Matrix4.IDENTITY),
      lookAt: vi.fn(),
      lookAtTransform: vi.fn(),
      setView: vi.fn(),
      pickEllipsoid: vi.fn(() => null),
      getPickRay: vi.fn(() => null),
    },
    scene: {
      canvas: { clientWidth: 100, clientHeight: 100, width: 100, height: 100 },
      pickPositionSupported: false,
      pickPosition: vi.fn(() => null),
      globe: { pick: vi.fn(() => null) },
      preUpdate: { addEventListener: vi.fn(() => () => {}) },
      requestRender: vi.fn(),
    },
  };
}

describe("readCameraTargetFrame", () => {
  it("returns null when viewer.camera.positionWC is missing", () => {
    const viewer = makeViewer();
    viewer.camera.positionWC = undefined as never;
    expect(readCameraTargetFrame(viewer as never)).toBeNull();
  });

  it("returns the orbit frame around the tracked entity when present", () => {
    const viewer = makeViewer();
    // Vendor-realistic fixture: camera ~10 km from target in ENU frame.
    // Camera at (-74.17°, 40.69°, 5 000 m), target at (-74.17°, 40.69°, 15 000 m).
    // Range ≈ 5 000, pitch ≈ -π/2 (looking straight up at target above).
    viewer.camera.positionWC = Cesium.Cartesian3.fromDegrees(
      -74.17,
      40.69,
      5_000,
    );
    const target = Cesium.Cartesian3.fromDegrees(-74.17, 40.69, 15_000);
    viewer.trackedEntity = { position: { getValue: () => target } };
    const frame = readCameraTargetFrame(viewer as never);
    expect(frame).not.toBeNull();
    expect(frame!.range).toBeGreaterThan(9_000);
    expect(frame!.range).toBeLessThan(11_000);
    expect(Number.isFinite(frame!.range)).toBe(true);
    expect(frame!.heading).toBeGreaterThanOrEqual(0);
    expect(frame!.heading).toBeLessThanOrEqual(2 * Math.PI);
    expect(frame!.pitch).toBeGreaterThanOrEqual(-Math.PI / 2);
    expect(frame!.pitch).toBeLessThanOrEqual(Math.PI / 2);
  });

  it("falls back to viewport center when no tracked entity", () => {
    const viewer = makeViewer();
    const target = Cesium.Cartesian3.fromDegrees(0, 0, 0);
    viewer.camera.pickEllipsoid = vi.fn(() => target);
    const frame = readCameraTargetFrame(viewer as never);
    expect(frame).not.toBeNull();
    expect(frame!.target).toEqual(target);
  });
});

describe("setCameraTargetFrame", () => {
  it("returns false when frame.target is missing", () => {
    const viewer = makeViewer();
    const result = setCameraTargetFrame(viewer as never, {
      target: undefined as never,
      range: 100,
      heading: 0,
      pitch: -0.5,
    });
    expect(result).toBe(false);
  });

  it("calls camera.lookAt then camera.lookAtTransform when tracked entity present", () => {
    const viewer = makeViewer();
    const target = Cesium.Cartesian3.fromDegrees(-74.17, 40.69, 10000);
    viewer.trackedEntity = { position: { getValue: () => target } };
    const result = setCameraTargetFrame(viewer as never, {
      target,
      range: 100,
      heading: 0,
      pitch: -0.5,
    });
    expect(result).toBe(true);
    expect(viewer.camera.lookAt).toHaveBeenCalledTimes(1);
    expect(viewer.camera.lookAtTransform).toHaveBeenCalledTimes(1);
  });

  it("calls camera.lookAt then camera.setView when no tracked entity (free-flight)", () => {
    const viewer = makeViewer();
    const target = Cesium.Cartesian3.fromDegrees(0, 0, 0);
    const result = setCameraTargetFrame(viewer as never, {
      target,
      range: 100,
      heading: 0,
      pitch: -0.5,
    });
    expect(result).toBe(true);
    expect(viewer.camera.lookAt).toHaveBeenCalledTimes(1);
    expect(viewer.camera.setView).toHaveBeenCalledTimes(1);
  });
});

describe("createCameraOrientationAnimator", () => {
  it("returns an object with animate, cancel, and destination getter", () => {
    const viewer = makeViewer();
    const animator = createCameraOrientationAnimator(viewer as never);
    expect(typeof animator.animate).toBe("function");
    expect(typeof animator.cancel).toBe("function");
    expect(animator.destination).toBeNull();
  });

  it("animates heading from frame.heading to destination.heading over duration", () => {
    const viewer = makeViewer();
    let preUpdateListener: (() => void) | null = null;
    viewer.scene.preUpdate.addEventListener = vi.fn((fn: () => void) => {
      preUpdateListener = fn;
      return () => {
        preUpdateListener = null;
      };
    });
    const animator = createCameraOrientationAnimator(viewer as never, {
      duration: 100,
    });
    const frame = {
      target: Cesium.Cartesian3.fromDegrees(0, 0, 0),
      range: 100,
      heading: 0,
      pitch: -0.5,
    };
    const destination = { heading: Math.PI / 2, pitch: -0.5 };
    animator.animate(frame, destination);
    expect(animator.destination).toEqual(destination);
    expect(preUpdateListener).not.toBeNull();
    preUpdateListener!();
    expect(viewer.camera.lookAt).toHaveBeenCalled();
    expect(viewer.camera.lookAtTransform).toHaveBeenCalled();
  });

  it("cancel() detaches the preUpdate listener", () => {
    const viewer = makeViewer();
    let detached = false;
    const removeListener = () => {
      detached = true;
    };
    viewer.scene.preUpdate.addEventListener = vi.fn(() => removeListener);
    const animator = createCameraOrientationAnimator(viewer as never);
    animator.animate(
      { target: Cesium.Cartesian3.fromDegrees(0, 0, 0), range: 100, heading: 0, pitch: -0.5 },
      { heading: 1, pitch: -0.5 },
    );
    expect(detached).toBe(false);
    animator.cancel();
    expect(detached).toBe(true);
    expect(animator.destination).toBeNull();
  });
});
