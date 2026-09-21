// GEV P15 T3 — mouse-look tests (spec §4.2).
//
// Tests the input → closure-scoped offset → snap-back animator path.
// Uses a hand-rolled Cesium camera double with ScreenSpaceEventHandler
// mocked via setInputAction callbacks we invoke manually.

import { describe, expect, it, vi, beforeEach } from "vitest";
import * as Cesium from "cesium";
import { mountCockpitMouseLook } from "../mouse-look";
import {
  COCKPIT_FORWARD_OFFSET_M,
  COCKPIT_MOUSE_LOOK_PITCH_CLAMP_MAX_RAD,
  COCKPIT_MOUSE_LOOK_PITCH_CLAMP_MIN_RAD,
  COCKPIT_MOUSE_LOOK_SNAPBACK_THRESHOLD_RAD,
  COCKPIT_MOUSE_WHEEL_RANGE_MAX_M,
  COCKPIT_MOUSE_WHEEL_RANGE_MIN_M,
  COCKPIT_MOUSE_WHEEL_RANGE_RATE_M_PER_DELTA,
} from "gev-engine/src/ui/cockpitPresentation.js";

interface MouseEvent {
  position?: { x: number; y: number };
  endPosition?: { x: number; y: number };
  deltaY?: number;
  deltaMode?: number;
  ctrlKey?: boolean;
}

function makeViewer() {
  const handlers = new Map<number, (event: MouseEvent) => void>();
  const screenSpaceEventHandler = {
    setInputAction: vi.fn((cb: (e: MouseEvent) => void, type: number) => {
      handlers.set(type, cb);
    }),
    destroy: vi.fn(() => {
      handlers.clear();
    }),
  };
  return {
    handlers,
    screenSpaceEventHandler,
    viewer: {
      clock: { currentTime: Cesium.JulianDate.now() },
      trackedEntity: { id: "test-aircraft" },
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
        // Real jsdom canvas so Cesium.ScreenSpaceEventHandler's constructor
        // (which calls element.addEventListener) actually binds — the brief's
        // mocked canvas would mask the constructor contract per P3 lesson.
        // clientWidth/clientHeight are jsdom read-only getters; width/height
        // are writable. Use Object.defineProperty for the read-only ones.
        canvas: (() => {
          const el = document.createElement("canvas");
          el.width = 100;
          el.height = 100;
          Object.defineProperty(el, "clientWidth", { value: 100 });
          Object.defineProperty(el, "clientHeight", { value: 100 });
          return el;
        })(),
        pickPositionSupported: false,
        pickPosition: vi.fn(() => null),
        globe: { pick: vi.fn(() => null) },
        screenSpaceCameraController: { enableInputs: false },
        preUpdate: { addEventListener: vi.fn(() => () => {}) },
        requestRender: vi.fn(),
        // getScreenSpaceEventHandler is the Cesium API; we override it
        // per-test to return our handler-tracking double.
        getScreenSpaceEventHandler: vi.fn(() => screenSpaceEventHandler),
      },
      isDestroyed: () => false,
    } as any,
  };
}

function makeStore(initialActive = true) {
  let active = initialActive;
  const listeners: Array<() => void> = [];
  return {
    store: {
      getState: () => ({ active }),
      subscribe: (l: () => void) => {
        listeners.push(l);
        return () => {
          const i = listeners.indexOf(l);
          if (i >= 0) listeners.splice(i, 1);
        };
      },
      setActive: (v: boolean) => {
        active = v;
        listeners.forEach((l) => l());
      },
    },
  };
}

// Cesium ScreenSpaceEventType values used by mouse-look
const RIGHT_DOWN = 7; // ScreenSpaceEventType.RIGHT_DOWN
const RIGHT_UP = 9; // ScreenSpaceEventType.RIGHT_UP
const MOUSE_MOVE = 5; // ScreenSpaceEventType.MOUSE_MOVE
const WHEEL = 14; // ScreenSpaceEventType.WHEEL

describe("mountCockpitMouseLook", () => {
  describe("constructor contract", () => {
    it("throws TypeError if viewer.scene.canvas is missing", () => {
      const { store } = makeStore();
      expect(() =>
        mountCockpitMouseLook({
          viewer: { scene: {}, camera: {} } as unknown as Parameters<typeof mountCockpitMouseLook>[0]["viewer"],
          store: store as unknown as Parameters<typeof mountCockpitMouseLook>[0]["store"],
        }),
      ).toThrow(TypeError);
    });

    it("throws TypeError if store.getState is missing", () => {
      const { viewer } = makeViewer();
      expect(() =>
        mountCockpitMouseLook({
          viewer: viewer as any,
          store: { subscribe: () => () => {} } as any,
        }),
      ).toThrow(TypeError);
    });

    it("throws TypeError if store.subscribe is missing", () => {
      const { viewer } = makeViewer();
      expect(() =>
        mountCockpitMouseLook({
          viewer: viewer as any,
          store: { getState: () => ({ active: true }) } as any,
        }),
      ).toThrow(TypeError);
    });
  });

  describe("default state", () => {
    it("getFrameOffset returns (0, 0, 0) before any input", () => {
      const { viewer } = makeViewer();
      const { store } = makeStore();
      const handle = mountCockpitMouseLook({
        viewer: viewer as any,
        store: store as any,
      });
      expect(handle.getFrameOffset()).toEqual({
        headingDeltaRad: 0,
        pitchDeltaRad: 0,
        rangeOffsetM: 0,
      });
      handle.destroy();
    });
  });

  describe("right-drag → offset update", () => {
    let v: ReturnType<typeof makeViewer>;
    let s: ReturnType<typeof makeStore>;
    let handle: ReturnType<typeof mountCockpitMouseLook>;

    beforeEach(() => {
      v = makeViewer();
      s = makeStore(true);
      handle = mountCockpitMouseLook({
        viewer: v.viewer as any,
        store: s.store as any,
      });
    });

    it("RIGHT_DOWN + MOUSE_MOVE updates headingDeltaRad by dx * yaw rate", () => {
      v.handlers.get(RIGHT_DOWN)!({
        position: { x: 100, y: 100 },
      });
      v.handlers.get(MOUSE_MOVE)!({
        position: { x: 100, y: 100 },
        endPosition: { x: 200, y: 100 }, // 100px right
      });
      // 100px * 0.0035 rad/px = 0.35 rad
      expect(handle.getFrameOffset().headingDeltaRad).toBeCloseTo(0.35, 5);
      expect(handle.getFrameOffset().pitchDeltaRad).toBeCloseTo(0, 5);
      handle.destroy();
    });

    it("MOUSE_MOVE updates pitchDeltaRad by dy * pitch rate", () => {
      v.handlers.get(RIGHT_DOWN)!({ position: { x: 100, y: 100 } });
      v.handlers.get(MOUSE_MOVE)!({
        position: { x: 100, y: 100 },
        endPosition: { x: 100, y: 200 }, // 100px down
      });
      // 100px * 0.0035 rad/px = 0.35 rad
      expect(handle.getFrameOffset().pitchDeltaRad).toBeCloseTo(0.35, 5);
      handle.destroy();
    });

    it("pitch is clamped to [MIN, MAX] on MOUSE_MOVE", () => {
      v.handlers.get(RIGHT_DOWN)!({ position: { x: 100, y: 100 } });
      // Drag way past clamp min
      v.handlers.get(MOUSE_MOVE)!({
        position: { x: 100, y: 100 },
        endPosition: { x: 100, y: -10000 },
      });
      expect(handle.getFrameOffset().pitchDeltaRad).toBeCloseTo(
        COCKPIT_MOUSE_LOOK_PITCH_CLAMP_MIN_RAD,
        5,
      );
      // Drag way past clamp max
      v.handlers.get(MOUSE_MOVE)!({
        position: { x: 100, y: 100 },
        endPosition: { x: 100, y: 10000 },
      });
      expect(handle.getFrameOffset().pitchDeltaRad).toBeCloseTo(
        COCKPIT_MOUSE_LOOK_PITCH_CLAMP_MAX_RAD,
        5,
      );
      handle.destroy();
    });

    it("heading does NOT clamp (free 360° spin)", () => {
      v.handlers.get(RIGHT_DOWN)!({ position: { x: 100, y: 100 } });
      // 10000px right = 35 rad, way past 2π
      v.handlers.get(MOUSE_MOVE)!({
        position: { x: 100, y: 100 },
        endPosition: { x: 10100, y: 100 },
      });
      expect(handle.getFrameOffset().headingDeltaRad).toBeGreaterThan(2 * Math.PI);
      handle.destroy();
    });

    it("MOUSE_MOVE without RIGHT_DOWN does NOT update offset", () => {
      v.handlers.get(MOUSE_MOVE)!({
        position: { x: 100, y: 100 },
        endPosition: { x: 200, y: 200 },
      });
      expect(handle.getFrameOffset().headingDeltaRad).toBe(0);
      expect(handle.getFrameOffset().pitchDeltaRad).toBe(0);
      handle.destroy();
    });
  });

  describe("wheel → offset update", () => {
    let v: ReturnType<typeof makeViewer>;
    let s: ReturnType<typeof makeStore>;
    let handle: ReturnType<typeof mountCockpitMouseLook>;

    beforeEach(() => {
      v = makeViewer();
      s = makeStore(true);
      handle = mountCockpitMouseLook({
        viewer: v.viewer as any,
        store: s.store as any,
      });
    });

    it("wheel-up (deltaY=-100) DECREASES rangeOffsetM (zoom IN)", () => {
      v.handlers.get(WHEEL)!({ deltaY: -100, deltaMode: 0 });
      expect(handle.getFrameOffset().rangeOffsetM).toBeCloseTo(
        -100 * COCKPIT_MOUSE_WHEEL_RANGE_RATE_M_PER_DELTA,
        5,
      );
      handle.destroy();
    });

    it("wheel-down (deltaY=+100) INCREASES rangeOffsetM (zoom OUT)", () => {
      v.handlers.get(WHEEL)!({ deltaY: 100, deltaMode: 0 });
      expect(handle.getFrameOffset().rangeOffsetM).toBeCloseTo(
        100 * COCKPIT_MOUSE_WHEEL_RANGE_RATE_M_PER_DELTA,
        5,
      );
      handle.destroy();
    });

    it("wheel with deltaY=0 is a no-op (no spurious zoom)", () => {
      v.handlers.get(WHEEL)!({ deltaY: 0, deltaMode: 0 });
      expect(handle.getFrameOffset().rangeOffsetM).toBe(0);
      handle.destroy();
    });

    it("wheel clamps rangeOffset to [MIN - FORWARD, MAX - FORWARD] in offset space", () => {
      // Way past max — 1000 notches × 100 px × 25 m/px = 2.5M meters
      v.handlers.get(WHEEL)!({ deltaY: 1_000_000, deltaMode: 0 });
      const maxOffset =
        COCKPIT_MOUSE_WHEEL_RANGE_MAX_M - COCKPIT_FORWARD_OFFSET_M;
      expect(handle.getFrameOffset().rangeOffsetM).toBeCloseTo(maxOffset, 5);

      // Reset and try way past min
      handle.destroy();
      v = makeViewer();
      s = makeStore(true);
      handle = mountCockpitMouseLook({
        viewer: v.viewer as any,
        store: s.store as any,
      });
      v.handlers.get(WHEEL)!({ deltaY: -1_000_000, deltaMode: 0 });
      const minOffset =
        COCKPIT_MOUSE_WHEEL_RANGE_MIN_M - COCKPIT_FORWARD_OFFSET_M;
      expect(handle.getFrameOffset().rangeOffsetM).toBeCloseTo(minOffset, 5);
      handle.destroy();
    });

    it("wheel works during right-drag (axis independence)", () => {
      v.handlers.get(RIGHT_DOWN)!({ position: { x: 100, y: 100 } });
      v.handlers.get(MOUSE_MOVE)!({
        position: { x: 100, y: 100 },
        endPosition: { x: 200, y: 200 },
      });
      v.handlers.get(WHEEL)!({ deltaY: -100, deltaMode: 0 });
      const offset = handle.getFrameOffset();
      expect(offset.headingDeltaRad).toBeGreaterThan(0);
      expect(offset.pitchDeltaRad).toBeGreaterThan(0);
      expect(offset.rangeOffsetM).toBeLessThan(0);
      handle.destroy();
    });

    it("wheel with deltaMode=1 (LINE) applies ×3 normalization (Linux Firefox)", () => {
      v.handlers.get(WHEEL)!({ deltaY: 1, deltaMode: 1 });
      expect(handle.getFrameOffset().rangeOffsetM).toBeCloseTo(
        3 * COCKPIT_MOUSE_WHEEL_RANGE_RATE_M_PER_DELTA,
        5,
      );
      handle.destroy();
    });

    it("wheel with deltaMode=2 (PAGE) applies ×50 normalization", () => {
      v.handlers.get(WHEEL)!({ deltaY: 1, deltaMode: 2 });
      expect(handle.getFrameOffset().rangeOffsetM).toBeCloseTo(
        50 * COCKPIT_MOUSE_WHEEL_RANGE_RATE_M_PER_DELTA,
        5,
      );
      handle.destroy();
    });

    it("trackpad pinch (ctrlKey=true) has SAME sign as mouse wheel", () => {
      v.handlers.get(WHEEL)!({ deltaY: -100, deltaMode: 0, ctrlKey: true });
      expect(handle.getFrameOffset().rangeOffsetM).toBeCloseTo(
        -100 * COCKPIT_MOUSE_WHEEL_RANGE_RATE_M_PER_DELTA,
        5,
      );
      handle.destroy();
    });
  });

  describe("snap-back on RIGHT_UP", () => {
    let v: ReturnType<typeof makeViewer>;
    let s: ReturnType<typeof makeStore>;
    let handle: ReturnType<typeof mountCockpitMouseLook>;

    beforeEach(() => {
      v = makeViewer();
      s = makeStore(true);
      handle = mountCockpitMouseLook({
        viewer: v.viewer as any,
        store: s.store as any,
      });
    });

    it("RIGHT_UP with drag above threshold kicks the animator (lookAt called)", () => {
      v.handlers.get(RIGHT_DOWN)!({ position: { x: 100, y: 100 } });
      v.handlers.get(MOUSE_MOVE)!({
        position: { x: 100, y: 100 },
        endPosition: { x: 200, y: 200 }, // 100,100 delta → 0.495 rad magnitude
      });
      // Magnitude = sqrt(0.35² + 0.35²) = 0.495 rad > 0.01 threshold
      v.viewer.camera.lookAt.mockClear();
      v.handlers.get(RIGHT_UP)!({ position: { x: 200, y: 200 } });
      expect(v.viewer.camera.lookAt).toHaveBeenCalled();
      handle.destroy();
    });

    it("RIGHT_UP with drag below threshold does NOT kick animator (instant snap)", () => {
      v.handlers.get(RIGHT_DOWN)!({ position: { x: 100, y: 100 } });
      v.handlers.get(MOUSE_MOVE)!({
        position: { x: 100, y: 100 },
        endPosition: { x: 100.1, y: 100.1 }, // 0.1px → 0.00035 rad < threshold
      });
      v.viewer.camera.lookAt.mockClear();
      v.handlers.get(RIGHT_UP)!({ position: { x: 100.1, y: 100.1 } });
      expect(v.viewer.camera.lookAt).not.toHaveBeenCalled();
      // Offsets should be reset to zero immediately
      expect(handle.getFrameOffset().headingDeltaRad).toBe(0);
      expect(handle.getFrameOffset().pitchDeltaRad).toBe(0);
      handle.destroy();
    });

    it("snapBack() public method eases to zero offsets via animator", () => {
      v.handlers.get(RIGHT_DOWN)!({ position: { x: 100, y: 100 } });
      v.handlers.get(MOUSE_MOVE)!({
        position: { x: 100, y: 100 },
        endPosition: { x: 200, y: 200 },
      });
      expect(handle.getFrameOffset().headingDeltaRad).toBeGreaterThan(0);
      v.viewer.camera.lookAt.mockClear();
      handle.snapBack();
      // Animator was kicked → lookAt called (it animates the camera back to origin)
      expect(v.viewer.camera.lookAt).toHaveBeenCalled();
      handle.destroy();
    });
  });

  describe("destroy lifecycle", () => {
    it("destroy unsubscribes all 4 handlers; subsequent input is a no-op", () => {
      const v = makeViewer();
      const s = makeStore(true);
      const handle = mountCockpitMouseLook({
        viewer: v.viewer as any,
        store: s.store as any,
      });
      handle.destroy();
      // screenSpaceEventHandler.destroy should have been called
      expect(v.screenSpaceEventHandler.destroy).toHaveBeenCalled();
      // Subsequent fire is a no-op (handler map cleared)
      v.handlers.get(WHEEL)?.({ deltaY: 100, deltaMode: 0 });
      expect(handle.getFrameOffset().rangeOffsetM).toBe(0);
    });

    it("destroy mid-drag resets isDragging; subsequent RIGHT_UP is ignored", () => {
      const v = makeViewer();
      const s = makeStore(true);
      const handle = mountCockpitMouseLook({
        viewer: v.viewer as any,
        store: s.store as any,
      });
      v.handlers.get(RIGHT_DOWN)!({ position: { x: 100, y: 100 } });
      v.handlers.get(MOUSE_MOVE)!({
        position: { x: 100, y: 100 },
        endPosition: { x: 200, y: 200 },
      });
      handle.destroy();
      // Even if RIGHT_UP somehow fires (stale handler), it doesn't re-kick animator
      v.viewer.camera.lookAt.mockClear();
      v.handlers.get(RIGHT_UP)?.({ position: { x: 200, y: 200 } });
      expect(v.viewer.camera.lookAt).not.toHaveBeenCalled();
    });
  });

  describe("store.active gating", () => {
    it("does NOT register handlers when store.active is initially false", () => {
      const v = makeViewer();
      const s = makeStore(false);
      mountCockpitMouseLook({
        viewer: v.viewer as any,
        store: s.store as any,
      });
      expect(v.screenSpaceEventHandler.setInputAction).not.toHaveBeenCalled();
    });

    it("registers handlers when store.active becomes true", () => {
      const v = makeViewer();
      const s = makeStore(false);
      mountCockpitMouseLook({
        viewer: v.viewer as any,
        store: s.store as any,
      });
      expect(v.screenSpaceEventHandler.setInputAction).not.toHaveBeenCalled();
      s.store.setActive(true);
      expect(v.screenSpaceEventHandler.setInputAction).toHaveBeenCalled();
    });
  });
});
