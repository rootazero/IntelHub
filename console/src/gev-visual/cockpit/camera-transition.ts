// GEV P12 T6 — cockpit camera enter/exit transition (spec §6.E2).
//
// On cockpit enter we fly the Cesium camera to a lifted, south-offset,
// down-tilted vantage above the tracked aircraft; on exit we fly back to the
// pose captured the instant before the enter. The vendor follow controller
// keeps driving the tracked entity while we are in the cockpit, so our flyTo
// is deliberately a short delta layered on top of a settled frame rather than
// a full camera takeover (spec R4).
//
// Cesium's Camera.flyTo replaces any in-progress flight, so a fresh request
// implicitly aborts the old one (spec R6); the only thing we owe the caller is
// to swallow the resulting abort rejection instead of surfacing it.
import * as Cesium from "cesium";

export interface CockpitCameraTarget {
  longitude: number;
  latitude: number;
  altitude: number;
}

export interface CockpitCameraFlyOptions {
  duration?: number;
}

/** Camera pose captured before entering the cockpit. */
interface CockpitCameraBaseline {
  position: Cesium.Cartesian3;
  heading: number;
  pitch: number;
  roll: number;
}

interface CockpitCameraOrientation {
  heading: number;
  pitch: number;
  roll: number;
}

interface CockpitCameraFlyRequest {
  destination: Cesium.Cartesian3;
  orientation: CockpitCameraOrientation;
  duration: number;
  easingFunction: typeof Cesium.EasingFunction.QUADRATIC_IN_OUT;
}

/** The subset of Cesium.Camera this adapter drives. */
interface CockpitCameraLike {
  flyTo(request: Partial<CockpitCameraFlyRequest>): Promise<boolean> | boolean;
  readonly position: Cesium.Cartesian3;
  readonly heading: number;
  readonly pitch: number;
  readonly roll: number;
}

export interface CockpitCameraTransitionDeps {
  viewer: {
    scene?: { canvas?: unknown };
    camera: CockpitCameraLike;
  };
}

export interface CockpitCameraTransition {
  /** Fly to the tracked aircraft overhead. Resolves when the flight ends/aborts. */
  flyToTracked(
    target: CockpitCameraTarget,
    opts?: CockpitCameraFlyOptions,
  ): Promise<void>;
  /** Fly back to the pre-cockpit camera pose. No-op without a baseline. */
  flyBackToBaseline(opts?: CockpitCameraFlyOptions): Promise<void>;
  /** Manually capture the current camera pose as the new baseline. */
  captureBaseline(): void;
  destroy(): void;
}

const DEFAULT_DURATION_S = 0.7;
/** 0.5° south of the aircraft → a forward-looking chase-cam tilt. */
const TRACK_LAT_OFFSET_DEG = -0.5;
/** Lift above the aircraft so terrain never clips the horizon. */
const TRACK_ALT_OFFSET_M = 1500;
const TRACK_PITCH_RAD = Cesium.Math.toRadians(-20);

export function mountCockpitCameraTransition(
  deps: CockpitCameraTransitionDeps,
): CockpitCameraTransition {
  // Constructor contract (P3 lesson): assert the seam at mount so a lenient
  // mock cannot hide a wrong object handed in later.
  if (!deps?.viewer?.camera?.flyTo) {
    throw new TypeError(
      "mountCockpitCameraTransition: viewer.camera.flyTo missing",
    );
  }

  const camera = deps.viewer.camera;
  let baseline: CockpitCameraBaseline | null = null;
  let activeFlyPromise: Promise<void> | null = null;
  let destroyed = false;

  function capture(): CockpitCameraBaseline {
    return {
      position: camera.position.clone(),
      heading: camera.heading,
      pitch: camera.pitch,
      roll: camera.roll,
    };
  }

  function fly(request: Omit<CockpitCameraFlyRequest, "easingFunction">): Promise<void> {
    if (destroyed) return Promise.resolve();
    activeFlyPromise = (async () => {
      try {
        const result = camera.flyTo({
          ...request,
          easingFunction: Cesium.EasingFunction.QUADRATIC_IN_OUT,
        });
        if (result instanceof Promise) await result;
      } catch {
        // A superseded flight aborts with a Cesium RuntimeError — expected.
      }
    })();
    return activeFlyPromise;
  }

  return {
    flyToTracked(target, opts = {}) {
      baseline = capture();
      return fly({
        destination: Cesium.Cartesian3.fromDegrees(
          target.longitude,
          target.latitude + TRACK_LAT_OFFSET_DEG,
          target.altitude + TRACK_ALT_OFFSET_M,
        ),
        orientation: { heading: 0, pitch: TRACK_PITCH_RAD, roll: 0 },
        duration: opts.duration ?? DEFAULT_DURATION_S,
      });
    },

    flyBackToBaseline(opts = {}) {
      if (!baseline) return Promise.resolve();
      const pose = baseline;
      return fly({
        destination: pose.position,
        orientation: {
          heading: pose.heading,
          pitch: pose.pitch,
          roll: pose.roll,
        },
        duration: opts.duration ?? DEFAULT_DURATION_S,
      });
    },

    captureBaseline() {
      baseline = capture();
    },

    destroy() {
      destroyed = true;
      baseline = null;
      activeFlyPromise = null;
    },
  };
}
