import { beforeEach, describe, expect, test, vi } from "vitest";

// Fake vendor module BEFORE import of the adapter: the adapter consumes the
// vendored pure functions, and the fake replicates their REAL contract
// (toggleCameraTilt flips between OBLIQUE -35° and STRAIGHT_DOWN -89° pitch
// states on a target frame read from the viewer; returns false when there is
// no view target). Lenient-mock guard: fake viewer must carry camera.heading.
vi.mock("gev-engine/src/ui/cameraOrientationControls.js", () => {
  let tilted = false;
  return {
    // Module-level `tilted` is shared across tests; reset it so the suite is
    // order-independent (brief explicitly allows adding __reset()).
    __reset: () => {
      tilted = false;
    },
    OBLIQUE_PITCH: -35 * (Math.PI / 180),
    STRAIGHT_DOWN_PITCH: -89 * (Math.PI / 180),
    toggleCameraTilt: (viewer: any) => {
      if (!viewer?.camera || typeof viewer.camera.heading !== "number") {
        throw new TypeError("toggleCameraTilt: viewer.camera.heading missing");
      }
      if (viewer.noTarget) return false;
      tilted = !tilted;
      return { tilted, pitch: tilted ? -35 * (Math.PI / 180) : -89 * (Math.PI / 180) };
    },
    resetCameraNorth: (viewer: any) => {
      if (!viewer?.camera) throw new TypeError("resetCameraNorth: viewer.camera missing");
      return true;
    },
    readCameraTargetFrame: (viewer: any) =>
      viewer.noTarget
        ? null
        : { target: {}, range: 1000, heading: 0, pitch: tilted ? -0.61 : -1.55 },
    setCameraTargetFrame: () => true,
    frameIsTilted: (frame: any) => frame.pitch > -60 * (Math.PI / 180),
    createCameraOrientationAnimator: () => ({
      animate: vi.fn(),
      cancel: vi.fn(),
      get destination() { return null; },
    }),
    pickViewTarget: () => null,
  };
});

import { mountCameraOrientation } from "../camera-orientation";
import * as vendor from "gev-engine/src/ui/cameraOrientationControls.js";

beforeEach(() => {
  (vendor as any).__reset();
});

function fakeViewer(noTarget = false) {
  return { camera: { heading: 0.3 }, trackedEntity: undefined, noTarget };
}

describe("mountCameraOrientation", () => {
  test("constructor contract: rejects viewer without camera.heading", () => {
    expect(() => mountCameraOrientation({} as any)).toThrow(TypeError);
  });

  test("toggleTilt flips oblique → down → oblique and reports the new档位", () => {
    const h = mountCameraOrientation(fakeViewer() as any);
    expect(h.toggleTilt()).toBe("oblique");
    expect(h.isTilted()).toBe(true);
    expect(h.toggleTilt()).toBe("down");
    expect(h.isTilted()).toBe(false);
    h.destroy();
  });

  test("toggleTilt returns null when there is no view target", () => {
    const h = mountCameraOrientation(fakeViewer(true) as any);
    expect(h.toggleTilt()).toBeNull();
    h.destroy();
  });

  test("resetNorth delegates and reports success", () => {
    const h = mountCameraOrientation(fakeViewer() as any);
    expect(h.resetNorth()).toBe(true);
    h.destroy();
  });

  test("destroy is idempotent and cancels the animator", () => {
    const h = mountCameraOrientation(fakeViewer() as any);
    expect(() => { h.destroy(); h.destroy(); }).not.toThrow();
  });
});
