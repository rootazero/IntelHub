// GEV P14 T2 — cockpit camera enter/exit transition (spec §6.E2).
//
// On cockpit enter we fly the Cesium camera to a chase vantage: 7 m forward
// and 2.6 m above the tracked aircraft (COCKPIT_FORWARD_OFFSET_M / COCKPIT_UP_OFFSET_M)
// pitched down 4° (COCKPIT_VIEW_PITCH_DEG) in the ENU forward direction; on exit
// we fly back to the pose captured the instant before the enter.
//
// The vendor follow controller keeps driving the tracked entity while we are in
// the cockpit, so our flyTo is deliberately a short delta layered on top of a
// settled frame rather than a full camera takeover (spec R4).
//
// Cesium's Camera.flyTo replaces any in-progress flight, so a fresh request
// implicitly aborts the old one (spec R6); the only thing we owe the caller is
// to swallow the resulting abort rejection instead of surfacing it.
import * as Cesium from "cesium";
import {
  COCKPIT_FORWARD_OFFSET_M,
  COCKPIT_UP_OFFSET_M,
  COCKPIT_VIEW_PITCH_DEG,
} from "gev-engine/src/ui/cockpitPresentation.js";

export interface CockpitCameraTarget {
  longitude: number;
  latitude: number;
  altitude: number;
}

export interface CockpitCameraFlyOptions {
  duration?: number;
  /** Override heading in degrees (default 0° = north). */
  headingDeg?: number;
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
  /** Returns true while any flyTo animation is in progress. */
  isInFlight(): boolean;
  destroy(): void;
}

const DEFAULT_DURATION_S = 0.7;
const CHASE_ENTRY_DURATION_S = 0.4;
/** Heading the camera points in if no `getHeading` is provided. */
const DEFAULT_HEADING_DEG = 0;

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
  let inFlightPromise: Promise<void> | null = null;

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
      const headingRad = Cesium.Math.toRadians(
        opts.headingDeg ?? DEFAULT_HEADING_DEG,
      );
      const pitchRad = Cesium.Math.toRadians(COCKPIT_VIEW_PITCH_DEG);
      const anchor = Cesium.Cartesian3.fromDegrees(
        target.longitude, target.latitude, target.altitude,
      );
      const enu = Cesium.Transforms.eastNorthUpToFixedFrame(anchor);
      const scratchForward = new Cesium.Cartesian3(
        Math.sin(headingRad) * Math.cos(pitchRad),
        Math.cos(headingRad) * Math.cos(pitchRad),
        Math.sin(pitchRad),
      );
      const scratchUp = new Cesium.Cartesian3(0, 0, 1);
      const forward = Cesium.Matrix4.multiplyByPointAsVector(
        enu, scratchForward, new Cesium.Cartesian3(),
      );
      const up = Cesium.Matrix4.multiplyByPointAsVector(
        enu, scratchUp, new Cesium.Cartesian3(),
      );
      const destination = Cesium.Cartesian3.clone(anchor);
      Cesium.Cartesian3.add(
        destination,
        Cesium.Cartesian3.multiplyByScalar(
          forward, COCKPIT_FORWARD_OFFSET_M, new Cesium.Cartesian3(),
        ),
        destination,
      );
      Cesium.Cartesian3.add(
        destination,
        Cesium.Cartesian3.multiplyByScalar(
          up, COCKPIT_UP_OFFSET_M, new Cesium.Cartesian3(),
        ),
        destination,
      );
      const promise = deps.viewer.camera.flyTo({
        destination,
        orientation: { direction: forward, up },
        duration: CHASE_ENTRY_DURATION_S,
        easingFunction: Cesium.EasingFunction.QUADRATIC_IN_OUT,
      });
      inFlightPromise = Promise.resolve(promise).then(
        () => { inFlightPromise = null; },
        () => { inFlightPromise = null; },
      );
      return inFlightPromise;
    },

    flyBackToBaseline(opts = {}) {
      if (!baseline) return Promise.resolve();
      const pose = baseline;
      const promise = deps.viewer.camera.flyTo({
        destination: pose.position,
        orientation: { heading: pose.heading, pitch: pose.pitch, roll: pose.roll },
        duration: opts.duration ?? DEFAULT_DURATION_S,
        easingFunction: Cesium.EasingFunction.QUADRATIC_IN_OUT,
      });
      inFlightPromise = Promise.resolve(promise).then(
        () => { inFlightPromise = null; },
        () => { inFlightPromise = null; },
      );
      return inFlightPromise;
    },

    isInFlight(): boolean {
      return inFlightPromise !== null;
    },

    captureBaseline() {
      baseline = capture();
    },

    destroy() {
      destroyed = true;
      baseline = null;
      activeFlyPromise = null;
      inFlightPromise = null;
    },
  };
}
