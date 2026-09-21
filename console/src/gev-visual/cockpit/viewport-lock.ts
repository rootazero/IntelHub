// GEV P12 T7 — cockpit viewport lock (spec §6.E5).
//
// While the cockpit is active the operator drives the aircraft, not the map.
// Every Cesium camera input (rotate / zoom / pinch / pan) must stop reaching
// the globe so an accidental drag on the HUD cannot pitch the view away from
// the tracked aircraft, and click / double-click / right-click selection must
// not re-target the cockpit. The lock is deliberately shallow:
//   - `screenSpaceCameraController.enableInputs = false` kills camera motion
//     (drag rotate, wheel zoom, pinch, middle-drag) in one switch;
//   - a dedicated ScreenSpaceEventHandler swallows LEFT_CLICK /
//     LEFT_DOUBLE_CLICK / RIGHT_CLICK, which are selection events owned by
//     OTHER handlers (the vendor's own pick handler would still fire);
//   - the canvas cursor is hidden so the pointer reads as a cockpit control.
// unlock() restores every one of those, and destroy() auto-unlocks so a HUD
// teardown cannot strand the globe in a dead-input state.
//
// Constructor contract (P3 lesson): assert the seam at mount — a lenient mock
// that omits screenSpaceCameraController or the canvas would pass a test and
// then silently no-op (or crash) against a real viewer.
import * as Cesium from "cesium";

export interface CockpitViewportLockViewer {
  scene: {
    screenSpaceCameraController: { enableInputs: boolean };
  };
  /** Cesium's real canvas (HTMLCanvasElement) — the ScreenSpaceEventHandler
   *  constructor contract requires it, and it is what carries the cursor. */
  cesiumWidget: { canvas: HTMLCanvasElement };
}

export interface CockpitViewportLock {
  /** Lock all globe input (camera motion + click selection) and hide cursor. */
  lock(): void;
  /** Restore input and the cursor. Idempotent when already unlocked. */
  unlock(): void;
  /** Currently locked? */
  isLocked(): boolean;
  /** Idempotent teardown — auto-unlocks a still-locked viewport. */
  destroy(): void;
}

export function mountCockpitViewportLock(
  viewer: CockpitViewportLockViewer,
): CockpitViewportLock {
  if (!viewer?.scene?.screenSpaceCameraController) {
    throw new TypeError(
      "mountCockpitViewportLock: viewer.scene.screenSpaceCameraController missing",
    );
  }
  if (!viewer?.cesiumWidget?.canvas) {
    throw new TypeError(
      "mountCockpitViewportLock: viewer.cesiumWidget.canvas missing",
    );
  }

  const controller = viewer.scene.screenSpaceCameraController;
  const canvas = viewer.cesiumWidget.canvas;
  let locked = false;
  let destroyed = false;
  // Cursor is inline-styled where present; capture whatever the live canvas
  // had so unlock() restores it instead of blindly clearing an app cursor.
  let previousCursor: string | null = null;
  let screenSpaceEventHandler: Cesium.ScreenSpaceEventHandler | null = null;

  function blockSelection(): void {
    screenSpaceEventHandler = new Cesium.ScreenSpaceEventHandler(canvas);
    const noop = () => {};
    screenSpaceEventHandler.setInputAction(
      noop,
      Cesium.ScreenSpaceEventType.LEFT_CLICK,
    );
    screenSpaceEventHandler.setInputAction(
      noop,
      Cesium.ScreenSpaceEventType.LEFT_DOUBLE_CLICK,
    );
    screenSpaceEventHandler.setInputAction(
      noop,
      Cesium.ScreenSpaceEventType.RIGHT_CLICK,
    );
    // WHEEL / MIDDLE_DRAG are camera inputs, already dead via enableInputs.
  }

  function releaseSelection(): void {
    if (screenSpaceEventHandler) {
      screenSpaceEventHandler.destroy();
      screenSpaceEventHandler = null;
    }
  }

  return {
    lock() {
      if (destroyed || locked) return;
      previousCursor = canvas.style.cursor;
      controller.enableInputs = false;
      canvas.style.cursor = "none";
      blockSelection();
      locked = true;
    },
    unlock() {
      if (!locked) return;
      controller.enableInputs = true;
      releaseSelection();
      canvas.style.cursor = previousCursor ?? "";
      previousCursor = null;
      locked = false;
    },
    isLocked: () => locked,
    destroy() {
      if (destroyed) return;
      destroyed = true;
      if (locked) this.unlock();
    },
  };
}
