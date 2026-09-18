// P7 camera orientation adapter — React-shell seam over the vendored
// cameraOrientationControls pure functions. The vendor module NEVER starts
// or stops tracking (it only reads viewer.trackedEntity); tracking ownership
// stays with the layer trackById APIs (see follow-controller, Task 5).
// scopeMask/celestialRing are not imported anywhere here (圆圈遮罩禁令).
import {
  toggleCameraTilt,
  resetCameraNorth,
  readCameraTargetFrame,
  frameIsTilted,
  createCameraOrientationAnimator,
} from "gev-engine/src/ui/cameraOrientationControls.js";

export interface CameraViewerLike {
  camera: { heading: number };
  trackedEntity?: unknown;
}

export interface CameraOrientationHandle {
  /** Flip oblique(-35°) ↔ straight-down(-89°). Returns the NEW档位, or null
   *  when there is no view target (nothing tracked, no ground under center). */
  toggleTilt(): "oblique" | "down" | null;
  resetNorth(): boolean;
  isTilted(): boolean;
  destroy(): void;
}

export function mountCameraOrientation(
  viewer: CameraViewerLike,
  deps: { now?: () => number } = {},
): CameraOrientationHandle {
  if (!viewer?.camera || typeof viewer.camera.heading !== "number") {
    throw new TypeError("mountCameraOrientation: viewer.camera.heading missing");
  }
  const animator = createCameraOrientationAnimator(viewer, deps.now ? { now: deps.now } : {});
  let destroyed = false;

  return {
    toggleTilt() {
      const result = toggleCameraTilt(viewer);
      if (result === false) return null;
      return result.tilted ? "oblique" : "down";
    },
    resetNorth() {
      return resetCameraNorth(viewer);
    },
    isTilted() {
      // Paid read (4-12ms depth buffer) — callers must be user gestures or
      // post-gesture state syncs, NEVER per-frame (engine perf discipline).
      const frame = readCameraTargetFrame(viewer);
      return frame ? frameIsTilted(frame) : false;
    },
    destroy() {
      if (destroyed) return;
      destroyed = true;
      animator.cancel();
    },
  };
}
