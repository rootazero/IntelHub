// GEV P14 — cockpit chase-cam driver.
//
// Ports the gods-eye-view src/ui/cockpitCamera.update() loop (lines 50-352)
// into the React tree. Why: GEV P9/P12 wired only the cockpit overlay +
// enter/exit flyTo; no per-frame camera driver. The user sees a frozen plane
// model after enter. The vendor proves a 50 ms rAF loop with slewHeading +
// ENU composition + camera.setView keeps the camera glued to the aircraft.
//
// Lifecycle:
//   mountCockpitChaseCam(deps) → ChaseCamHandle
//     .start()  — begin rAF, subscribe cockpitStore
//     .stop()   — pause rAF, keep subscription
//     .destroy() — cancel rAF, unsubscribe, drop refs
//
// In-flight cooperation: chase-cam **must not** call viewer.camera.setView
// while camera-transition is mid-flyTo (Cesium aborts the flight on setView).
// mountCockpitCameraTransition exposes isInFlight(); chase-cam consults it
// before every setView and skips the tick if true.

import * as Cesium from "cesium";
import {
  slewHeading,
  cockpitAnchorCorrectionStep,
  cockpitUiUpdateDue,
} from "gev-engine/src/cockpitMath.js";
import {
  COCKPIT_FORWARD_OFFSET_M,
  COCKPIT_UP_OFFSET_M,
  COCKPIT_HEADING_SLEW_DPS,
  COCKPIT_VIEW_PITCH_DEG,
  COCKPIT_CAMERA_UPDATE_MS,
} from "gev-engine/src/ui/cockpitPresentation.js";
import type { CockpitStore } from "./cockpit-store";
import { MOUSE_LOOK_ZERO_OFFSET as ZERO_OFFSET } from "./mouse-look";

/** Subset of Cesium.Entity we touch. */
interface ChaseEntity {
  id?: string;
  position: { getValue(time: Date, result?: Cesium.Cartesian3): Cesium.Cartesian3 | undefined };
}

/** Subset of Cesium.Viewer.camera we drive. */
interface ChaseCamera {
  setView(opts: {
    destination: Cesium.Cartesian3;
    orientation: { direction: Cesium.Cartesian3; up: Cesium.Cartesian3 };
  }): void;
}

interface ChaseViewer {
  clock?: { currentTime: Date };
  camera: ChaseCamera;
}

/** Subset of mountCockpitCameraTransition we depend on. */
interface ChaseTransition {
  isInFlight(): boolean;
}

export interface ChaseCamDeps {
  viewer: ChaseViewer;
  store: CockpitStore;
  transition: ChaseTransition;
  getTrackedEntity: () => ChaseEntity | null;
  getHeading: () => number | null;
  /**
   * GEV P15 T4 addition — optional mouse-look handle. When omitted,
   * chase-cam uses MOUSE_LOOK_ZERO_OFFSET (backward-compat with P14):
   * camera pose = anchor + forward*7 + up*2.6, exactly. When provided,
   * chase-cam reads `getFrameOffset()` once per cadence tick and composes
   * the offset onto heading (additive), pitch (additive), and range
   * (additive). Drag rides ON TOP of track slew — it is not a
   * replacement for the aircraft's nose direction.
   */
  mouseLook?: {
    getFrameOffset(): {
      headingDeltaRad: number;
      pitchDeltaRad: number;
      rangeOffsetM: number;
    };
  };
}

export interface ChaseCamHandle {
  start(): void;
  stop(): void;
  destroy(): void;
}

const PITCH_RAD = Cesium.Math.toRadians(COCKPIT_VIEW_PITCH_DEG);
const UP_OFFSET = COCKPIT_UP_OFFSET_M;
const CADENCE_MS = COCKPIT_CAMERA_UPDATE_MS;
const SLEW_DPS = COCKPIT_HEADING_SLEW_DPS;

export function mountCockpitChaseCam(deps: ChaseCamDeps): ChaseCamHandle {
  // Constructor contract: assert the seam at mount.
  if (!deps?.viewer?.camera?.setView) {
    throw new TypeError("mountCockpitChaseCam: deps.viewer.camera.setView is required");
  }
  if (typeof deps?.store?.subscribe !== "function") {
    throw new TypeError("mountCockpitChaseCam: deps.store.subscribe is required");
  }
  if (typeof deps?.transition?.isInFlight !== "function") {
    throw new TypeError("mountCockpitChaseCam: deps.transition.isInFlight is required");
  }
  if (typeof deps?.getTrackedEntity !== "function") {
    throw new TypeError("mountCockpitChaseCam: deps.getTrackedEntity is required");
  }
  if (typeof deps?.getHeading !== "function") {
    throw new TypeError("mountCockpitChaseCam: deps.getHeading is required");
  }

  // Closure-scoped state, mirrors vendor cockpitCamera.js.
  const scratchEnu = new Cesium.Matrix4();
  const scratchForward = new Cesium.Cartesian3();
  const scratchUp = new Cesium.Cartesian3();
  const scratchTarget = new Cesium.Cartesian3();
  const scratchCamera = new Cesium.Cartesian3();
  const scratchOffset = new Cesium.Cartesian3();
  const cockpitAnchor = new Cesium.Cartesian3();
  // Previous tick's resolved position — used to derive the entity's motion
  // delta and advance the anchor inertially. Without this, the bounded
  // cockpitAnchorCorrectionStep is capped at 0.75 m/s and a 200 m/s aircraft
  // leaves the anchor 100+ m behind after 1 s.
  const prevTarget = new Cesium.Cartesian3();
  let prevTargetValid = false;
  let heading: number | null = null;
  let lastFrameMs = 0;
  let lastCameraUpdateMs = 0;
  let cockpitAnchorValid = false;
  let running = false;
  let destroyed = false;
  let rafId: number | null = null;

  function step(): void {
    if (destroyed) return;
    rafId = requestAnimationFrame(step);
    if (!running) return;

    // 1. Skip if not active
    if (!deps.store.getState().active) return;

    // 2. Cadence gate
    const nowMs = performance.now();
    if (!cockpitUiUpdateDue(nowMs, lastCameraUpdateMs, CADENCE_MS)) return;
    lastCameraUpdateMs = nowMs;

    // 3. Resolve tracked entity
    const entity = deps.getTrackedEntity();
    if (!entity) return;

    // 4. Resolve current position
    const clockTime = deps.viewer.clock?.currentTime ?? new Date();
    const target = entity.position.getValue(clockTime, scratchTarget);
    if (!target) return;

    // 5. Read mouse-look offset (atomic snapshot per tick) and compose
    // heading/pitch/range. P15 T4: when mouseLook is omitted, this resolves
    // to ZERO_OFFSET — drag rides on top of track slew additively.
    const dtSec = Math.min(0.1, Math.max(0, (nowMs - lastFrameMs) / 1000));
    lastFrameMs = nowMs;
    const targetHeading = deps.getHeading();
    const offset = deps.mouseLook?.getFrameOffset() ?? ZERO_OFFSET;
    // Compose: drag offset rides ON TOP of track slew (additive). Operator
    // drag is an offset on top of the aircraft's nose direction, not a
    // replacement. UNIT NOTE: getHeading() returns DEGREES (per P14 — see
    // existing test 'destination is in chase envelope ... heading 90'), and
    // slewHeading is fed degrees. mouseLook's headingDeltaRad is in RADIANS
    // (per T3 contract). Convert rad → deg before adding.
    const composedTargetHeading =
      (Number.isFinite(targetHeading) ? (targetHeading as number) : 0) +
      Cesium.Math.toDegrees(offset.headingDeltaRad);
    if (Number.isFinite(targetHeading)) {
      heading = slewHeading(
        heading ?? targetHeading ?? 0,
        composedTargetHeading,
        SLEW_DPS * dtSec,
      );
    }
    const composedPitchRad = PITCH_RAD + offset.pitchDeltaRad;
    const composedRange = COCKPIT_FORWARD_OFFSET_M + offset.rangeOffsetM;

    // 6. Update cockpit anchor (smoothed inertial position)
    if (!cockpitAnchorValid) {
      Cesium.Cartesian3.clone(target, cockpitAnchor);
      cockpitAnchorValid = true;
    } else {
      // 6a. Inertial advance: anchor moves by the entity's reported motion
      // delta so we don't depend on cockpitAnchorCorrectionStep's bounded
      // rate (0.75 m/s floor when speed input is 0). For a 200 m/s aircraft
      // a correction-only path leaves the anchor ~133 m behind after 1 s;
      // tracking the entity's reported position keeps the anchor glued.
      if (prevTargetValid) {
        const motion = Cesium.Cartesian3.subtract(target, prevTarget, scratchOffset);
        Cesium.Cartesian3.add(cockpitAnchor, motion, cockpitAnchor);
      }
      // 6b. Bounded correction step — converges on any residual position
      // drift that wasn't captured by the inertial advance (e.g. a feed
      // re-anchor that didn't propagate through motion). Vendor behavior:
      // min(distance, distance*(1-e^{-1.25 dt}), max(0.75, speed*0.22)*dt).
      const distanceM = Cesium.Cartesian3.distance(cockpitAnchor, target);
      const correctionM = cockpitAnchorCorrectionStep(distanceM, 0, dtSec);
      if (correctionM > 0 && distanceM > 0) {
        const dir = Cesium.Cartesian3.subtract(target, cockpitAnchor, scratchOffset);
        Cesium.Cartesian3.normalize(dir, dir);
        Cesium.Cartesian3.multiplyByScalar(dir, correctionM, dir);
        Cesium.Cartesian3.add(cockpitAnchor, dir, cockpitAnchor);
      }
    }
    // Save the resolved target for the next tick's velocity derivation.
    Cesium.Cartesian3.clone(target, prevTarget);
    prevTargetValid = true;

    // 7. Build ENU frame at anchor
    Cesium.Transforms.eastNorthUpToFixedFrame(cockpitAnchor, undefined, scratchEnu);

    // 8. Compute forward direction in local ENU. P15 T4: composedPitchRad
    // bakes in mouseLook's pitchDeltaRad (additive on top of COCKPIT_VIEW_PITCH_DEG).
    const hRad = Cesium.Math.toRadians(heading ?? 0);
    scratchForward.x = Math.sin(hRad) * Math.cos(composedPitchRad);
    scratchForward.y = Math.cos(hRad) * Math.cos(composedPitchRad);
    scratchForward.z = Math.sin(composedPitchRad);
    Cesium.Matrix4.multiplyByPointAsVector(scratchEnu, scratchForward, scratchForward);
    Cesium.Cartesian3.normalize(scratchForward, scratchForward);

    // 9. Up is ENU +Z
    scratchUp.x = 0; scratchUp.y = 0; scratchUp.z = 1;
    Cesium.Matrix4.multiplyByPointAsVector(scratchEnu, scratchUp, scratchUp);
    Cesium.Cartesian3.normalize(scratchUp, scratchUp);

    // 10. Compose camera position. P15 T4: composedRange bakes in
    // mouseLook's rangeOffsetM (additive on top of COCKPIT_FORWARD_OFFSET_M).
    Cesium.Cartesian3.clone(cockpitAnchor, scratchCamera);
    Cesium.Cartesian3.multiplyByScalar(scratchForward, composedRange, scratchOffset);
    Cesium.Cartesian3.add(scratchCamera, scratchOffset, scratchCamera);
    Cesium.Cartesian3.multiplyByScalar(scratchUp, UP_OFFSET, scratchOffset);
    Cesium.Cartesian3.add(scratchCamera, scratchOffset, scratchCamera);

    // 11. Skip if flyTo is in flight (Cesium would abort the flight on setView)
    if (deps.transition.isInFlight()) return;

    // 12. Apply — swallow + retry next tick if Cesium throws (defensive)
    try {
      deps.viewer.camera.setView({
        destination: scratchCamera,
        orientation: { direction: scratchForward, up: scratchUp },
      });
    } catch {
      // Swallowed; next tick will retry. We don't want one transient Cesium
      // hiccup to disable the chase loop permanently.
    }
  }

  const unsubscribe = deps.store.subscribe(() => {
    // Could re-evaluate anchor validity on trackedId change. For P14 we keep
    // anchor sticky until next enter — simpler and matches GE behavior.
  });

  return {
    start(): void {
      if (destroyed || running) return;
      running = true;
      lastFrameMs = performance.now();
      lastCameraUpdateMs = 0;
      // Reset prevTarget so a fresh enter re-anchors cleanly. Without this,
      // the first tick after a new enter would try to advance the anchor by
      // (newTarget - oldTarget) which can span the whole globe if the user
      // switched aircraft.
      prevTargetValid = false;
      if (rafId === null) rafId = requestAnimationFrame(step);
    },
    stop(): void {
      running = false;
    },
    destroy(): void {
      if (destroyed) return;
      destroyed = true;
      running = false;
      if (rafId !== null) {
        cancelAnimationFrame(rafId);
        rafId = null;
      }
      unsubscribe();
      cockpitAnchorValid = false;
      prevTargetValid = false;
      heading = null;
    },
  };
}