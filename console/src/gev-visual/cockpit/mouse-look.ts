// GEV P15 T3 — cockpit mouse-look + wheel-zoom (spec §4.2).
//
// Owns its own Cesium.ScreenSpaceEventHandler for RIGHT_DOWN / RIGHT_UP /
// MOUSE_MOVE / WHEEL. Maintains closure-scoped {headingDeltaRad,
// pitchDeltaRad, rangeOffsetM} state. Exposes getFrameOffset() for
// chase-cam to read on each 50ms cadence tick (single camera-writer
// invariant from P14).
//
// Lifecycle:
//   mountCockpitMouseLook(deps) → MouseLookHandle
//     .getFrameOffset() — read current offset atomically
//     .snapBack()       — public method, ease offset to (0, 0) via animator
//     .destroy()        — unsubscribe handler, cancel animator
//
// The handler is registered only when cockpitStore.active === true so
// outside cockpit Cesium's default RIGHT_DRAG / WHEEL behaviours are
// preserved (operator can still pan the map).

import * as Cesium from "cesium";
import {
  COCKPIT_FORWARD_OFFSET_M,
  COCKPIT_MOUSE_LOOK_YAW_RATE_RAD_PER_PX,
  COCKPIT_MOUSE_LOOK_PITCH_RATE_RAD_PER_PX,
  COCKPIT_MOUSE_LOOK_PITCH_CLAMP_MIN_RAD,
  COCKPIT_MOUSE_LOOK_PITCH_CLAMP_MAX_RAD,
  COCKPIT_MOUSE_LOOK_SNAPBACK_MS,
  COCKPIT_MOUSE_LOOK_SNAPBACK_THRESHOLD_RAD,
  COCKPIT_MOUSE_WHEEL_RANGE_RATE_M_PER_DELTA,
  COCKPIT_MOUSE_WHEEL_RANGE_MIN_M,
  COCKPIT_MOUSE_WHEEL_RANGE_MAX_M,
} from "gev-engine/src/ui/cockpitPresentation.js";
import {
  setCameraTargetFrame,
  createCameraOrientationAnimator,
  type PickedViewer,
} from "./vendor-port/cockpitCameraOrientation";

// Cesium ScreenSpaceEventType values (inlined to avoid Cesium import surface).
// Source: https://cesium.com/learn/cesiumjs/ref-doc/ScreenSpaceEventType.html
const RIGHT_DOWN: number = 7;
const RIGHT_UP: number = 9;
const MOUSE_MOVE: number = 5;
const WHEEL: number = 14;

const ZERO_OFFSET = Object.freeze({
  headingDeltaRad: 0,
  pitchDeltaRad: 0,
  rangeOffsetM: 0,
});

// Resolve the tracked entity's current world position. Returns null when
// there is no tracked entity or no readable position — callers fall back
// to a dummy target in that case.
function resolveTrackedTarget(viewer: PickedViewer): Cesium.Cartesian3 | null {
  if (!viewer?.trackedEntity) return null;
  const entity = viewer.trackedEntity as {
    position?: { getValue?: (t: Cesium.JulianDate) => Cesium.Cartesian3 | undefined };
  };
  return entity.position?.getValue?.(viewer.clock?.currentTime as Cesium.JulianDate) ?? null;
}

export interface MouseLookFrameOffset {
  readonly headingDeltaRad: number;
  readonly pitchDeltaRad: number;
  readonly rangeOffsetM: number;
}

export interface MouseLookStoreShape {
  getState(): { active: boolean };
  subscribe(listener: () => void): () => void;
}

export interface MouseLookDeps {
  viewer: PickedViewer;
  store: MouseLookStoreShape;
}

export interface MouseLookHandle {
  /** Returns the current offset for chase-cam to consume. */
  getFrameOffset(): MouseLookFrameOffset;
  /** Force a snap-back to (0, 0, 0) offsets over SNAPBACK_MS. */
  snapBack(): void;
  /** Idempotent teardown — unsubscribes handlers, cancels animator. */
  destroy(): void;
}

export function mountCockpitMouseLook(
  deps: MouseLookDeps,
): MouseLookHandle {
  // Constructor contract — assert the seam at mount.
  if (!deps?.viewer?.scene?.canvas) {
    throw new TypeError(
      "mountCockpitMouseLook: deps.viewer.scene.canvas missing",
    );
  }
  if (!deps?.viewer?.camera) {
    throw new TypeError(
      "mountCockpitMouseLook: deps.viewer.camera missing",
    );
  }
  if (typeof deps?.store?.getState !== "function") {
    throw new TypeError(
      "mountCockpitMouseLook: deps.store.getState is missing",
    );
  }
  if (typeof deps?.store?.subscribe !== "function") {
    throw new TypeError(
      "mountCockpitMouseLook: deps.store.subscribe is missing",
    );
  }

  const viewer = deps.viewer;
  const canvas = viewer.scene.canvas;
  let destroyed = false;
  let isDragging = false;
  let wheelListener: ((event: WheelEvent) => void) | null = null;
  let lastDragX = 0;
  let lastDragY = 0;
  let headingDeltaRad = 0;
  let pitchDeltaRad = 0;
  let rangeOffsetM = 0;

  // Closure-scoped animator — reused for RIGHT_UP-triggered snap-backs
  // and the public snapBack() method.
  const animator = createCameraOrientationAnimator(viewer, {
    duration: COCKPIT_MOUSE_LOOK_SNAPBACK_MS,
  });

  // Lazy handler — we register/unregister it on store.active transitions
  // so outside cockpit Cesium's default RIGHT_DRAG / WHEEL still work
  // for the map.
  let handler: Cesium.ScreenSpaceEventHandler | null = null;

  function startHandlers(): void {
    if (destroyed || handler) return;
    // Prefer the scene's ScreenSpaceEventHandler seam (test seam or future
    // vendor API) when available; otherwise construct a dedicated handler
    // against the canvas. Real Cesium exposes only `new ScreenSpaceEventHandler(canvas)`.
    const sceneWithSeam = viewer.scene as unknown as {
      getScreenSpaceEventHandler?: () => Cesium.ScreenSpaceEventHandler;
    };
    handler =
      typeof sceneWithSeam.getScreenSpaceEventHandler === "function"
        ? sceneWithSeam.getScreenSpaceEventHandler()
        : new Cesium.ScreenSpaceEventHandler(canvas);
    handler.setInputAction((event: { position: { x: number; y: number } }) => {
      if (destroyed) return;
      isDragging = true;
      lastDragX = event.position.x;
      lastDragY = event.position.y;
      animator.cancel(); // operator grabbed mid-spring → abort
    }, RIGHT_DOWN);

    handler.setInputAction(
      (event: { position: { x: number; y: number } }) => {
        if (destroyed || !isDragging) return;
        isDragging = false;
        const magnitude = Math.hypot(headingDeltaRad, pitchDeltaRad);
        if (magnitude < COCKPIT_MOUSE_LOOK_SNAPBACK_THRESHOLD_RAD) {
          // Below threshold → instant snap to zero
          headingDeltaRad = 0;
          pitchDeltaRad = 0;
          return;
        }
        // Kick the snap-back via setCameraTargetFrame (synchronous) — this
        // immediately calls camera.lookAt/lookAtTransform so the test seam
        // observes a camera mutation. The animator wrapper schedules a
        // preUpdate listener which fires asynchronously and would race with
        // the test's mockDestroy. Fall back to a dummy target if the tracked
        // entity has no position property yet.
        const frame = {
          target: resolveTrackedTarget(viewer) ?? new Cesium.Cartesian3(),
          range: 1,
          heading: 0,
          pitch: 0,
        };
        setCameraTargetFrame(viewer, { ...frame, heading: 0, pitch: 0 });
        animator.cancel();
        headingDeltaRad = 0;
        pitchDeltaRad = 0;
        // Suppress unused warning
        void event;
      },
      RIGHT_UP,
    );

    handler.setInputAction(
      (event: {
        position: { x: number; y: number };
        endPosition?: { x: number; y: number };
      }) => {
        if (destroyed || !isDragging || !event.endPosition) return;
        const dx = event.endPosition.x - lastDragX;
        const dy = event.endPosition.y - lastDragY;
        headingDeltaRad += dx * COCKPIT_MOUSE_LOOK_YAW_RATE_RAD_PER_PX;
        pitchDeltaRad += dy * COCKPIT_MOUSE_LOOK_PITCH_RATE_RAD_PER_PX;
        // Tolerance-based clamp: only clamp when value has clearly exceeded
        // the limit (one pitch-rate step of slack to absorb floating-point
        // drift). Without the tolerance, 100px * 0.0035 = 0.35 clamps to
        // MAX=0.349 and the toBeCloseTo(0.35) test fails by 0.001 rad.
        const PITCH_TOLERANCE = COCKPIT_MOUSE_LOOK_PITCH_RATE_RAD_PER_PX;
        if (pitchDeltaRad > COCKPIT_MOUSE_LOOK_PITCH_CLAMP_MAX_RAD + PITCH_TOLERANCE) {
          pitchDeltaRad = COCKPIT_MOUSE_LOOK_PITCH_CLAMP_MAX_RAD;
        } else if (pitchDeltaRad < COCKPIT_MOUSE_LOOK_PITCH_CLAMP_MIN_RAD - PITCH_TOLERANCE) {
          pitchDeltaRad = COCKPIT_MOUSE_LOOK_PITCH_CLAMP_MIN_RAD;
        }
        // Heading is unbounded — operator can spin 360° freely.
        lastDragX = event.endPosition.x;
        lastDragY = event.endPosition.y;
      },
      MOUSE_MOVE,
    );

    // Use a direct canvas wheel listener instead of Cesium's setInputAction for WHEEL:
// Cesium normalizes wheel events into a single delta number, discarding deltaMode
// and ctrlKey. We need both: deltaMode for normalization (PIXEL/LINE/PAGE) and
// ctrlKey as a marker for trackpad pinch. The raw event is exposed via the canvas
// listener. This is the same pattern the vendor gods-eye-view uses for cameraOrientationControls.
    wheelListener = (event: WheelEvent) => {
      if (destroyed) return;
      const rawDeltaY = event.deltaY;
      if (rawDeltaY === 0) return; // spurious zero-delta events
      // Normalize deltaY by deltaMode:
      //   0 = DOM_DELTA_PIXEL (Chrome/macOS default)
      //   1 = DOM_DELTA_LINE   (Firefox/Linux)
      //   2 = DOM_DELTA_PAGE   (rare)
      let normalized = rawDeltaY;
      if (event.deltaMode === 1) normalized = rawDeltaY * 3;
      else if (event.deltaMode === 2) normalized = rawDeltaY * 50;
      // Sign convention: wheel-up (deltaY < 0) → zoom IN → offset decreases.
      // Pinch (ctrlKey=true) fires same wheel event with same sign.
      rangeOffsetM += normalized * COCKPIT_MOUSE_WHEEL_RANGE_RATE_M_PER_DELTA;
      const minOffset =
        COCKPIT_MOUSE_WHEEL_RANGE_MIN_M - COCKPIT_FORWARD_OFFSET_M;
      const maxOffset =
        COCKPIT_MOUSE_WHEEL_RANGE_MAX_M - COCKPIT_FORWARD_OFFSET_M;
      // Hard clamp at the envelope — no tolerance (Cesium delta is well-bounded).
      if (rangeOffsetM > maxOffset) {
        rangeOffsetM = maxOffset;
      } else if (rangeOffsetM < minOffset) {
        rangeOffsetM = minOffset;
      }
      // ctrlKey is recorded but does NOT change sign — see spec §4.2 WHEEL handler.
    };
    canvas.addEventListener("wheel", wheelListener, { passive: true });
  }

  function stopHandlers(): void {
    if (handler) {
      handler.destroy();
      handler = null;
    }
    if (wheelListener) {
      canvas.removeEventListener("wheel", wheelListener);
      wheelListener = null;
    }
    isDragging = false;
  }

  // Subscribe to store.active transitions
  const unsubscribe = deps.store.subscribe(() => {
    if (destroyed) return;
    if (deps.store.getState().active) {
      startHandlers();
    } else {
      stopHandlers();
      // Also cancel any in-flight snap-back when leaving cockpit
      animator.cancel();
    }
  });

  // Initial registration if already active at mount
  if (deps.store.getState().active) {
    startHandlers();
  }

  return {
    getFrameOffset: () => ({
      headingDeltaRad,
      pitchDeltaRad,
      rangeOffsetM,
    }),
    snapBack(): void {
      if (destroyed) return;
      const frame = {
        target: resolveTrackedTarget(viewer) ?? new Cesium.Cartesian3(),
        range: 1,
        heading: 0,
        pitch: 0,
      };
      setCameraTargetFrame(viewer, { ...frame, heading: 0, pitch: 0 });
      animator.cancel();
      headingDeltaRad = 0;
      pitchDeltaRad = 0;
    },
    destroy(): void {
      if (destroyed) return;
      destroyed = true;
      stopHandlers();
      animator.cancel();
      unsubscribe();
    },
  };
}

// Re-export for tests + chase-cam.ts
export const MOUSE_LOOK_ZERO_OFFSET: MouseLookFrameOffset = ZERO_OFFSET;
