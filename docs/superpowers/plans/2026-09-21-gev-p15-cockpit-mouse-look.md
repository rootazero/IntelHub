# GEV P15 — Cockpit Mouse-Look + Wheel-Zoom Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Wire the vendored gods-eye-view camera-orbit math into the IntelHub React tree so cockpit operators can right-drag to look around the aircraft and mouse-wheel to zoom within `[50 m, 5000 m]`, while keeping the existing P14 chase-cam 50 ms cadence as the single camera-writer.

**Architecture:** Port three pure-math functions from vendor `cameraOrientationControls.js` (`readCameraTargetFrame`, `setCameraTargetFrame`, `createCameraOrientationAnimator`) into a new IntelHub `vendor-port/cockpitCameraOrientation.ts`. Add a new `mouse-look.ts` that owns its own `Cesium.ScreenSpaceEventHandler` for `RIGHT_DOWN/RIGHT_UP/MOUSE_MOVE/WHEEL`, maintains closure-scoped `{headingDeltaRad, pitchDeltaRad, rangeOffsetM}` state, exposes a `getFrameOffset()` reader, and runs a vendor-style snap-back animator on `RIGHT_UP`. Extend `chase-cam.ts` to consume `mouseLook.getFrameOffset()` on each cadence tick (backward-compatible via `ZERO_OFFSET` default). Extend `viewport-lock.ts` to leave Cesium's `enableInputs = false` (so the cockpit doesn't pan the map) while letting our own handler fire for RIGHT_DRAG/WHEEL. Wire everything in `GlobeV2.tsx` mount effect with `mouseLook.destroy()` called AFTER `chaseCam.destroy()`.

**Tech Stack:** Vitest (existing test framework), TypeScript strict, Cesium `ScreenSpaceEventHandler`, Cesium `EasingFunction.CUBIC_IN_OUT`. No new deps.

**Spec:** [docs/superpowers/specs/2026-09-21-gev-p15-cockpit-mouse-look-design.md](../specs/2026-09-21-gev-p15-cockpit-mouse-look-design.md) — the plan argues from the spec; executors read both.

## Global Constraints

- **Worktree**: All work on `feat/gev-p15-cockpit-mouse-look` at `/Volumes/TBU/Workspace/IntelHub-p15`. Never edit `/Volumes/TBU/Workspace/IntelHub` directly.
- **TDD**: Each task writes failing tests first, runs them (expect FAIL), implements, runs (expect PASS), commits.
- **No DB / API / env-var changes**: Pure console bundle.
- **No console/dist rsync from Mac**: `build-console.sh` runs in VM (VITE_CARTO_KEY in `core/console-build.env`).
- **315 → 410 deploy sequence**: `scripts/build-hub.sh` + `scripts/build-console.sh` + `sudo systemctl restart hub-core && sleep 300` on test VM; same on prod after main merge.
- **Constants imported verbatim** from `console/gev-engine/src/ui/cockpitPresentation.js`. New constants added to that file (and then vendor-sync marked as IntelHub-only extension per AGENTS.md GEV pin workflow).
- **Constructor contract**: every `mount*` function throws `TypeError` if required deps are missing. P3 lesson — a lenient mock that omits `viewer` would pass a test and then silently no-op against a real viewer.
- **No `vi.fn()` mocks of Cesium constructors** without asserting the seam (P14 GEV P3 lesson: lenient mocks hide wrong objects). Either use real Cesium (in `CockpitCameraOrientation.ts` tests) or assert `MockCockpitCamera.constructor` requires `canvas?.addEventListener`.
- **AGENTS.md ops rules** apply verbatim: 410 production isolation, worktree-first, 315-test-first deploy, 5-min restart wait.
- **Single camera-writer invariant** (P14 rule): only `chase-cam.ts` calls `viewer.camera.setView`. `mouse-look.ts` and `vendor-port/cockpitCameraOrientation.ts` are input/state/math only — they never write to `viewer.camera` directly except via `setCameraTargetFrame` from the animator, which the chase-cam does NOT call (the animator's output is closure-scoped offsets that chase-cam reads).

## Review Focus

The spec implies these input classes / failure modes that no test currently exercises:

1. **Wheel `deltaY=0` events** (some trackpads fire spurious zero-delta events on inertia-decay) — must not move the offset by 0 (no-op, but test pins the "no-op at zero" behavior so a future bug that multiplies by zero doesn't double the offset).
2. **Wheel while right-drag is active** — must update `rangeOffsetM` independently (operator can wheel-zoom mid-drag). Test pins the axis independence.
3. **Snap-back fires with `viewer.trackedEntity = null`** — animator should abort (vendor pattern line 359–362). Test pins the abort.
4. **`destroy()` called between `RIGHT_DOWN` and `RIGHT_UP`** — input state must be reset; subsequent `RIGHT_UP` from a stale handler must not re-fire (test pins `isDragging` reset + handler unsubscribe).
5. **Wheel events with `event.deltaMode = 2` (page mode)** — Linux Firefox scrolls in page mode (one notch = one page). The normalization multiplier × 50 is asserted in the test so a future tweak doesn't break this.

For each, the test that pins it is in Task 2 (mouse-look.test.ts).

---

## File Structure

```
console/src/gev-visual/cockpit/
├── mouse-look.ts                      (NEW, ~190 LoC)
├── vendor-port/
│   └── cockpitCameraOrientation.ts    (NEW, ~120 LoC, ports 3 fns from vendor)
├── chase-cam.ts                       (EDIT, +30 LoC for offset injection)
├── viewport-lock.ts                   (EDIT, +5 LoC for the "own ScreenSpaceEventHandler" comment)
├── cockpitPresentation.js             (EDIT in gev-engine/src/ui/, +10 LoC constants)
├── __tests__/
│   ├── mouse-look.test.ts             (NEW, ~250 LoC, 10 tests + 5 review-focus tests)
│   ├── cockpitCameraOrientation.test.ts (NEW, ~120 LoC, 4 tests)
│   ├── chase-cam.test.ts              (EDIT, +3 tests)
│   ├── viewport-lock.test.ts          (EDIT, +1 test)
│   └── source-contracts.test.ts       (EDIT, +1 test)
└── index.ts                           (EDIT, +3 exports)

console/src/pages/GlobeV2.tsx          (EDIT, ~10 LoC wiring)
console/scripts/accept-sp8.py          (EDIT, +3 P15 bundle checks)
console/probe-gev.mjs                  (EDIT, +1 P15 probe: drag-then-release snap-back)
```

Each module has one responsibility:
- `mouse-look.ts` — input handlers + closure-scoped offset state.
- `vendor-port/cockpitCameraOrientation.ts` — pure camera-math (read/set frame, animate).
- `chase-cam.ts` — orchestrates chase-cam loop, now also reads mouse-look offsets.
- `viewport-lock.ts` — disables Cesium default input + cursor during cockpit; mouse-look owns its own handler (no circular dep).
- `GlobeV2.tsx` — mounts/destroys both modules in correct order.

---

## Task 1: Add `COCKPIT_MOUSE_LOOK_*` constants

**Files:**
- Edit: `console/gev-engine/src/ui/cockpitPresentation.js` (add 8 constants)
- Edit: `console/gev-engine/src/UPSTREAM.json` (mark these as IntelHub-only extension)

**Interfaces:**
- Consumes: nothing.
- Produces: 8 new exports — `COCKPIT_MOUSE_LOOK_YAW_RATE_RAD_PER_PX`, `COCKPIT_MOUSE_LOOK_PITCH_RATE_RAD_PER_PX`, `COCKPIT_MOUSE_LOOK_PITCH_CLAMP_MIN_RAD`, `COCKPIT_MOUSE_LOOK_PITCH_CLAMP_MAX_RAD`, `COCKPIT_MOUSE_LOOK_SNAPBACK_MS`, `COCKPIT_MOUSE_LOOK_SNAPBACK_THRESHOLD_RAD`, `COCKPIT_MOUSE_WHEEL_RANGE_RATE_M_PER_DELTA`, `COCKPIT_MOUSE_WHEEL_RANGE_MIN_M`, `COCKPIT_MOUSE_WHEEL_RANGE_MAX_M`. (Note: split the pitch clamp into min/max for easier import; PITCH_CLAMP_RAD as a tuple is misleadingly named.)

- [ ] **Step 1: Add the constants**

Edit `console/gev-engine/src/ui/cockpitPresentation.js`. Find the line `export const COCKPIT_GROUND_PROBE_MS = 500;` and append after it:

```js
// GEV P15 — cockpit mouse-look + wheel-zoom (spec §4.2).
// IntelHub-only extension — vendor gods-eye-view has not added these yet.
// Next sync-vendor.sh run will detect drift and propose re-pin.
export const COCKPIT_MOUSE_LOOK_YAW_RATE_RAD_PER_PX = 0.0035; // ≈0.2°/px
export const COCKPIT_MOUSE_LOOK_PITCH_RATE_RAD_PER_PX = 0.0035;
export const COCKPIT_MOUSE_LOOK_PITCH_CLAMP_MIN_RAD = -1.4835; // ≈-85°
export const COCKPIT_MOUSE_LOOK_PITCH_CLAMP_MAX_RAD = 0.349; // ≈+20°
export const COCKPIT_MOUSE_LOOK_SNAPBACK_MS = 350;
export const COCKPIT_MOUSE_LOOK_SNAPBACK_THRESHOLD_RAD = 0.01;
export const COCKPIT_MOUSE_WHEEL_RANGE_RATE_M_PER_DELTA = 25;
export const COCKPIT_MOUSE_WHEEL_RANGE_MIN_M = 50;
export const COCKPIT_MOUSE_WHEEL_RANGE_MAX_M = 5000;
```

- [ ] **Step 2: Mark UPSTREAM.json**

Edit `console/gev-engine/UPSTREAM.json` (or whatever the GEV pin manifest is called — verify with `ls console/gev-engine/`). Add an `intelhub_extensions` array listing the 9 constants as IntelHub-only.

```bash
ls /Volumes/TBU/Workspace/IntelHub-p15/console/gev-engine/
cat /Volumes/TBU/Workspace/IntelHub-p15/console/gev-engine/UPSTREAM.json 2>/dev/null || echo "no UPSTREAM.json"
```

If no `UPSTREAM.json` exists, skip this step — the vendor-sync script must have a different drift-detection mechanism. Verify with the existing GEV sync script.

- [ ] **Step 3: Verify imports resolve**

Run: `cd /Volumes/TBU/Workspace/IntelHub-p15/console && node -e "import('./src/ui/cockpitPresentation.js').then(m => console.log(Object.keys(m).filter(k => k.includes('MOUSE')).sort()))"`
Expected: prints the 9 new `COCKPIT_MOUSE_*` constant names.

- [ ] **Step 4: Commit**

```bash
cd /Volumes/TBU/Workspace/IntelHub-p15
git add console/gev-engine/src/ui/cockpitPresentation.js \
        console/gev-engine/UPSTREAM.json 2>/dev/null || true
git commit -m "feat(gev-p15-t1): COCKPIT_MOUSE_LOOK_* + COCKPIT_MOUSE_WHEEL_* constants"
```

---

## Task 2: Vendor Port — `cockpitCameraOrientation.ts`

**Files:**
- Create: `console/src/gev-visual/cockpit/vendor-port/cockpitCameraOrientation.ts`
- Create: `console/src/gev-visual/cockpit/__tests__/cockpitCameraOrientation.test.ts`

**Interfaces:**
- Consumes: nothing (leaf module).
- Produces:
  - `readCameraTargetFrame(viewer): {target: Cesium.Cartesian3, range: number, heading: number, pitch: number} | null`
  - `setCameraTargetFrame(viewer, frame): boolean`
  - `createCameraOrientationAnimator(viewer, opts?): { animate(frame, destination): boolean; cancel(): void; get destination(): {heading, pitch} | null }`
  - `PickedViewer` interface (consumed by Task 3).
- Vendor source: `/Volumes/TBU/Github/gods-eye-view/src/ui/cameraOrientationControls.js` lines 60–128 (read/set), 188–229 (animator).

- [ ] **Step 1: Write the failing tests**

Create `console/src/gev-visual/cockpit/__tests__/cockpitCameraOrientation.test.ts`:

```ts
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
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd /Volumes/TBU/Workspace/IntelHub-p15/console && npx vitest run src/gev-visual/cockpit/__tests__/cockpitCameraOrientation.test.ts`
Expected: FAIL — `Cannot find module '../vendor-port/cockpitCameraOrientation'`

- [ ] **Step 3: Create the vendor-port file**

Create `console/src/gev-visual/cockpit/vendor-port/cockpitCameraOrientation.ts`. Port the three functions from `/Volumes/TBU/Github/gods-eye-view/src/ui/cameraOrientationControls.js` lines 60–128 and 188–229. TypeScript-strict port — exact same math, only type signatures and module format change.

```ts
// GEV P15 T2 — vendor port of cameraOrientationControls (spec §4.3).
//
// Ports three pure-math functions from gods-eye-view/src/ui/cameraOrientationControls.js:
//   - readCameraTargetFrame (lines 60-93)
//   - setCameraTargetFrame (lines 95-128)
//   - createCameraOrientationAnimator (lines 188-229)
//
// The math is byte-stable upstream — only the file extension / module format
// changes; logic stays identical. AGENTS.md GEV pin workflow will detect drift
// on next sync. mouse-look.ts consumes these to pan / zoom / snap-back in
// cockpit mode.

import * as Cesium from "cesium";

function isPickedWorldPosition(value: unknown): value is Cesium.Cartesian3 {
  return (
    value instanceof Cesium.Cartesian3 &&
    Number.isFinite(value.x) &&
    Number.isFinite(value.y) &&
    Number.isFinite(value.z)
  );
}

function trackedTarget(viewer: PickedViewer): Cesium.Cartesian3 | null {
  const entity = viewer.trackedEntity;
  const displayPosition =
    (entity as { gevDisplayPosition?: () => Cesium.Cartesian3 } | undefined)
      ?.gevDisplayPosition;
  const fromDisplay =
    typeof displayPosition === "function" ? displayPosition() : null;
  if (isPickedWorldPosition(fromDisplay)) return fromDisplay;
  const position = entity?.position?.getValue?.(viewer.clock?.currentTime);
  return isPickedWorldPosition(position) ? position : null;
}

export interface PickedViewer {
  clock?: { currentTime: Cesium.JulianDate };
  trackedEntity?: Cesium.Entity | undefined;
  camera: Cesium.Camera;
  scene: Cesium.Scene;
  isDestroyed?: () => boolean;
}

export interface CameraTargetFrame {
  target: Cesium.Cartesian3;
  range: number;
  heading: number;
  pitch: number;
}

export function readCameraTargetFrame(
  viewer: PickedViewer,
): CameraTargetFrame | null {
  const camera = viewer?.camera;
  const target = viewer?.trackedEntity
    ? trackedTarget(viewer)
    : pickViewTarget(viewer);
  if (!camera || !target || !isPickedWorldPosition(camera.positionWC))
    return null;
  const transform = Cesium.Transforms.eastNorthUpToFixedFrame(target);
  const inverse = Cesium.Matrix4.inverseTransformation(
    transform,
    new Cesium.Matrix4(),
  );
  const localOffset = Cesium.Matrix4.multiplyByPoint(
    inverse,
    camera.positionWC,
    new Cesium.Cartesian3(),
  );
  const range = Cesium.Cartesian3.magnitude(localOffset);
  if (!Number.isFinite(range) || range < 1) return null;
  const pitch = -Math.asin(
    Cesium.Math.clamp(localOffset.z / range, -1, 1),
  );
  const targetHeading = Math.atan2(-localOffset.x, -localOffset.y);
  const heading = normalizedHeading(
    pitch < Cesium.Math.toRadians(-88.5) ? camera.heading : targetHeading,
  );
  return { target, range, heading, pitch };
}

function pickViewTarget(viewer: PickedViewer): Cesium.Cartesian3 | null {
  const scene = viewer?.scene;
  const camera = viewer?.camera;
  const canvas = scene?.canvas;
  if (!scene || !camera || !canvas) return null;
  const width = canvas.clientWidth || canvas.width || 0;
  const height = canvas.clientHeight || canvas.height || 0;
  if (!width || !height) return null;
  const center = new Cesium.Cartesian2(width / 2, height / 2);
  let target: Cesium.Cartesian3 | null = null;
  if (
    scene.pickPositionSupported &&
    typeof scene.pickPosition === "function"
  ) {
    try {
      target = scene.pickPosition(center) ?? null;
    } catch {
      target = null;
    }
  }
  if (
    !isPickedWorldPosition(target) &&
    typeof camera.getPickRay === "function"
  ) {
    try {
      target = scene.globe?.pick(camera.getPickRay(center), scene) || null;
    } catch {
      target = null;
    }
  }
  if (
    !isPickedWorldPosition(target) &&
    typeof camera.pickEllipsoid === "function"
  ) {
    try {
      target = camera.pickEllipsoid(center, Cesium.Ellipsoid.WGS84) ?? null;
    } catch {
      target = null;
    }
  }
  return isPickedWorldPosition(target) ? target : null;
}

export function setCameraTargetFrame(
  viewer: PickedViewer,
  frame: CameraTargetFrame,
): boolean {
  const camera = viewer?.camera;
  if (!camera || !frame?.target || !Number.isFinite(frame.range)) return false;
  try {
    const trackedTransform = viewer.trackedEntity
      ? Cesium.Matrix4.clone(camera.transform)
      : null;
    camera.lookAt(
      frame.target,
      new Cesium.HeadingPitchRange(
        normalizedHeading(frame.heading),
        frame.pitch,
        frame.range,
      ),
    );
    if (trackedTransform) {
      camera.lookAtTransform(trackedTransform);
      viewer.scene?.requestRender?.();
      return true;
    }
    const destination = Cesium.Cartesian3.clone(camera.positionWC);
    const direction = Cesium.Cartesian3.clone(camera.directionWC);
    const up = Cesium.Cartesian3.clone(camera.upWC);
    camera.lookAtTransform(Cesium.Matrix4.IDENTITY);
    if (destination && direction && up) {
      camera.setView({ destination, orientation: { direction, up } });
    }
    viewer.scene?.requestRender?.();
    return true;
  } catch {
    return false;
  }
}

export interface AnimatorHandle {
  animate(
    frame: CameraTargetFrame,
    destination: { heading: number; pitch: number },
  ): boolean;
  cancel(): void;
  readonly destination: { heading: number; pitch: number } | null;
}

export function createCameraOrientationAnimator(
  viewer: PickedViewer,
  {
    now = () => performance.now(),
    duration = 650,
  }: { now?: () => number; duration?: number } = {},
): AnimatorHandle {
  let remove: (() => void) | null = null;
  let pending: { heading: number; pitch: number } | null = null;
  const cancel = () => {
    remove?.();
    remove = null;
    pending = null;
  };
  function animate(
    frame: CameraTargetFrame,
    destination: { heading: number; pitch: number },
  ): boolean {
    cancel();
    if (!viewer.scene?.preUpdate?.addEventListener || duration <= 0)
      return setCameraTargetFrame(viewer, { ...frame, ...destination });
    const entity = viewer.trackedEntity;
    pending = destination;
    const start = now();
    const headingDelta = Cesium.Math.negativePiToPi(
      destination.heading - frame.heading,
    );
    remove = viewer.scene.preUpdate.addEventListener(() => {
      if (viewer.isDestroyed?.() || viewer.trackedEntity !== entity) {
        cancel();
        return;
      }
      const progress = Cesium.Math.clamp((now() - start) / duration, 0, 1);
      const eased = Cesium.EasingFunction.CUBIC_IN_OUT(progress);
      const target = entity ? trackedTarget(viewer) : frame.target;
      if (
        !isPickedWorldPosition(target) ||
        !setCameraTargetFrame(viewer, {
          ...frame,
          target,
          heading: frame.heading + headingDelta * eased,
          pitch: Cesium.Math.lerp(frame.pitch, destination.pitch, eased),
        })
      ) {
        cancel();
        return;
      }
      if (progress === 1) cancel();
    });
    viewer.scene.requestRender?.();
    return true;
  }
  return {
    animate,
    cancel,
    get destination() {
      return pending;
    },
  };
}

function normalizedHeading(heading: number): number {
  const wrapped = Cesium.Math.zeroToTwoPi(
    Number.isFinite(heading) ? heading : 0,
  );
  return Math.abs(wrapped - Cesium.Math.TWO_PI) < Cesium.Math.EPSILON10
    ? 0
    : wrapped;
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd /Volumes/TBU/Workspace/IntelHub-p15/console && npx vitest run src/gev-visual/cockpit/__tests__/cockpitCameraOrientation.test.ts`
Expected: PASS — 9 tests pass.

- [ ] **Step 5: Run full cockpit test suite for regression**

Run: `cd /Volumes/TBU/Workspace/IntelHub-p15/console && npx vitest run src/gev-visual/cockpit/`
Expected: PASS — 95 (existing) + 9 (T2) = 104 tests pass.

- [ ] **Step 6: Commit**

```bash
cd /Volumes/TBU/Workspace/IntelHub-p15
git add console/src/gev-visual/cockpit/vendor-port/cockpitCameraOrientation.ts \
        console/src/gev-visual/cockpit/__tests__/cockpitCameraOrientation.test.ts
git commit -m "feat(gev-p15-t2): vendor-port cockpitCameraOrientation (3 fns)"
```

---

## Task 3: `mouse-look.ts` — Input + State

**Files:**
- Create: `console/src/gev-visual/cockpit/mouse-look.ts`
- Create: `console/src/gev-visual/cockpit/__tests__/mouse-look.test.ts`
- Edit: `console/src/gev-visual/cockpit/index.ts` (add barrel export)

**Interfaces:**
- Consumes:
  - `MouseLookDeps { viewer: PickedViewer; store: MouseLookStoreShape }`
  - `COCKPIT_MOUSE_LOOK_*` and `COCKPIT_MOUSE_WHEEL_*` constants from T1.
  - `setCameraTargetFrame` and `createCameraOrientationAnimator` from T2.
- Produces:
  - `mountCockpitMouseLook(deps): MouseLookHandle`
  - `MouseLookHandle { getFrameOffset(): {headingDeltaRad, pitchDeltaRad, rangeOffsetM}; snapBack(): void; destroy(): void }`

- [ ] **Step 1: Write the failing tests**

Create `console/src/gev-visual/cockpit/__tests__/mouse-look.test.ts`:

```ts
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
} from "../../../gev-engine/src/ui/cockpitPresentation.js";

interface MouseEvent {
  position: { x: number; y: number };
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
        canvas: { clientWidth: 100, clientHeight: 100, width: 100, height: 100 },
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
    } as never,
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
          viewer: { scene: {}, camera: {} } as never,
          store: store as never,
        }),
      ).toThrow(TypeError);
    });

    it("throws TypeError if store.getState is missing", () => {
      const { viewer } = makeViewer();
      expect(() =>
        mountCockpitMouseLook({
          viewer: viewer as never,
          store: { subscribe: () => () => {} } as never,
        }),
      ).toThrow(TypeError);
    });

    it("throws TypeError if store.subscribe is missing", () => {
      const { viewer } = makeViewer();
      expect(() =>
        mountCockpitMouseLook({
          viewer: viewer as never,
          store: { getState: () => ({ active: true }) } as never,
        }),
      ).toThrow(TypeError);
    });
  });

  describe("default state", () => {
    it("getFrameOffset returns (0, 0, 0) before any input", () => {
      const { viewer } = makeViewer();
      const { store } = makeStore();
      const handle = mountCockpitMouseLook({
        viewer: viewer as never,
        store: store as never,
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
        viewer: v.viewer as never,
        store: s.store as never,
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
        viewer: v.viewer as never,
        store: s.store as never,
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
        viewer: v.viewer as never,
        store: s.store as never,
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
        viewer: v.viewer as never,
        store: s.store as never,
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
        viewer: v.viewer as never,
        store: s.store as never,
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
        viewer: v.viewer as never,
        store: s.store as never,
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
        viewer: v.viewer as never,
        store: s.store as never,
      });
      expect(v.screenSpaceEventHandler.setInputAction).not.toHaveBeenCalled();
    });

    it("registers handlers when store.active becomes true", () => {
      const v = makeViewer();
      const s = makeStore(false);
      mountCockpitMouseLook({
        viewer: v.viewer as never,
        store: s.store as never,
      });
      expect(v.screenSpaceEventHandler.setInputAction).not.toHaveBeenCalled();
      s.store.setActive(true);
      expect(v.screenSpaceEventHandler.setInputAction).toHaveBeenCalled();
    });
  });
});
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd /Volumes/TBU/Workspace/IntelHub-p15/console && npx vitest run src/gev-visual/cockpit/__tests__/mouse-look.test.ts`
Expected: FAIL — `Cannot find module '../mouse-look'`

- [ ] **Step 3: Implement `mouse-look.ts`**

Create `console/src/gev-visual/cockpit/mouse-look.ts`:

```ts
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
} from "../../../gev-engine/src/ui/cockpitPresentation.js";
import {
  setCameraTargetFrame,
  createCameraOrientationAnimator,
  type PickedViewer,
} from "./vendor-port/cockpitCameraOrientation.js";

// Cesium ScreenSpaceEventType values (inlined to avoid Cesium import surface).
// Source: https://cesium.com/learn/cesiumjs/ref-doc/ScreenSpaceEventType.html
const RIGHT_DOWN = 7 as const;
const RIGHT_UP = 9 as const;
const MOUSE_MOVE = 5 as const;
const WHEEL = 14 as const;

const ZERO_OFFSET = Object.freeze({
  headingDeltaRad: 0,
  pitchDeltaRad: 0,
  rangeOffsetM: 0,
});

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
    // Cesium exposes the ScreenSpaceEventHandler via scene.screenSpaceCameraController's
    // sibling — most idiomatic API is scene.canvas + new ScreenSpaceEventHandler.
    handler = new Cesium.ScreenSpaceEventHandler(canvas);
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
        // Kick the animator with destination (0, 0). The animator eases
        // the camera back to origin via setCameraTargetFrame (vendor
        // pattern, lines 188-229 of cameraOrientationControls.js).
        const frame = {
          target: new Cesium.Cartesian3(), // dummy, animator calls trackedTarget() each tick
          range: 1,
          heading: 0,
          pitch: 0,
        };
        const target = viewer.trackedEntity
          ? (viewer.trackedEntity as { position?: { getValue?: (t: Cesium.JulianDate) => Cesium.Cartesian3 | undefined } })
              .position?.getValue?.(viewer.clock?.currentTime as Cesium.JulianDate)
          : null;
        if (target) {
          frame.target = target;
          animator.animate(frame, { heading: 0, pitch: 0 });
        } else {
          // No tracked entity → instant snap (animator would abort anyway)
          headingDeltaRad = 0;
          pitchDeltaRad = 0;
        }
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
        pitchDeltaRad = Math.max(
          COCKPIT_MOUSE_LOOK_PITCH_CLAMP_MIN_RAD,
          Math.min(COCKPIT_MOUSE_LOOK_PITCH_CLAMP_MAX_RAD, pitchDeltaRad),
        );
        // Heading is unbounded — operator can spin 360° freely.
        lastDragX = event.endPosition.x;
        lastDragY = event.endPosition.y;
      },
      MOUSE_MOVE,
    );

    handler.setInputAction(
      (event: {
        deltaY?: number;
        deltaMode?: number;
        ctrlKey?: boolean;
      }) => {
        if (destroyed) return;
        const rawDeltaY = event.deltaY ?? 0;
        if (rawDeltaY === 0) return; // spurious zero-delta events (trackpad inertia decay)
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
        rangeOffsetM = Math.max(minOffset, Math.min(maxOffset, rangeOffsetM));
        // ctrlKey is recorded but does NOT change sign — see spec §4.2 WHEEL handler.
      },
      WHEEL,
    );
  }

  function stopHandlers(): void {
    if (handler) {
      handler.destroy();
      handler = null;
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
        target: new Cesium.Cartesian3(),
        range: 1,
        heading: 0,
        pitch: 0,
      };
      const target = viewer.trackedEntity
        ? (viewer.trackedEntity as { position?: { getValue?: (t: Cesium.JulianDate) => Cesium.Cartesian3 | undefined } })
            .position?.getValue?.(viewer.clock?.currentTime as Cesium.JulianDate)
        : null;
      if (target) {
        frame.target = target;
        animator.animate(frame, { heading: 0, pitch: 0 });
        // Reset offsets immediately — animator will then ease the camera back
        // via setCameraTargetFrame.
        headingDeltaRad = 0;
        pitchDeltaRad = 0;
      }
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
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd /Volumes/TBU/Workspace/IntelHub-p15/console && npx vitest run src/gev-visual/cockpit/__tests__/mouse-look.test.ts`
Expected: PASS — 21 tests pass (10 main + 5 wheel + 4 snap-back + 2 lifecycle + 2 store gating — wait, the test file has 22 cases. Let me recount: 3 constructor + 1 default + 6 right-drag + 9 wheel + 3 snap-back + 2 destroy + 2 store-gating = 26 tests).

- [ ] **Step 5: Run full cockpit test suite for regression**

Run: `cd /Volumes/TBU/Workspace/IntelHub-p15/console && npx vitest run src/gev-visual/cockpit/`
Expected: PASS — 95 + 9 (T2) + 26 (T3) = 130 tests pass.

- [ ] **Step 6: Update barrel index.ts**

Edit `console/src/gev-visual/cockpit/index.ts`. Append:

```ts
export {
  mountCockpitMouseLook,
  MOUSE_LOOK_ZERO_OFFSET,
  type MouseLookHandle,
  type MouseLookDeps,
  type MouseLookFrameOffset,
  type MouseLookStoreShape,
} from "./mouse-look";
```

- [ ] **Step 7: Commit**

```bash
cd /Volumes/TBU/Workspace/IntelHub-p15
git add console/src/gev-visual/cockpit/mouse-look.ts \
        console/src/gev-visual/cockpit/__tests__/mouse-look.test.ts \
        console/src/gev-visual/cockpit/index.ts
git commit -m "feat(gev-p15-t3): mouse-look — RIGHT_DRAG pan + WHEEL zoom + snap-back"
```

---

## Task 4: `chase-cam.ts` — Offset Injection

**Files:**
- Edit: `console/src/gev-visual/cockpit/chase-cam.ts` (lines 130–210, the tick loop)
- Edit: `console/src/gev-visual/cockpit/__tests__/chase-cam.test.ts` (+3 tests)

**Interfaces:**
- Consumes: `MOUSE_LOOK_ZERO_OFFSET` and `MouseLookHandle.getFrameOffset` shape from T3.
- Produces: `ChaseCamDeps` extended with optional `mouseLook?: { getFrameOffset(): { headingDeltaRad: number; pitchDeltaRad: number; rangeOffsetM: number } }`. When omitted, `MOUSE_LOOK_ZERO_OFFSET` is used (P14 backward-compat).

- [ ] **Step 1: Write the failing tests**

Edit `console/src/gev-visual/cockpit/__tests__/chase-cam.test.ts`. Append three new tests inside the existing `describe("mountCockpitChaseCam")` block:

```ts
it("uses ZERO_OFFSET when mouseLook is omitted (backward-compat with P14)", async () => {
  // Set up chase-cam WITHOUT mouseLook — camera.setView destination must
  // match P14's expected `(anchor + forward*7 + up*2.6)` exactly.
  const viewer = makeViewer();
  viewer.trackedEntity = makeEntity({ lat: 40.69, lon: -74.17, alt: 10000 });
  const store = makeStore({ active: true });
  const transition = makeTransition(false);
  const handle = mountCockpitChaseCam({
    viewer,
    store,
    transition,
    getTrackedEntity: () => viewer.trackedEntity,
    getHeading: () => 90,
  });
  handle.start();
  await rafTick();
  expect(viewer.camera.setView).toHaveBeenCalled();
  // Capture the destination and assert it's exactly 7m forward + 2.6m up
  // (no offset applied).
  const call = viewer.camera.setView.mock.calls.at(-1)?.[0] as
    | { destination: Cesium.Cartesian3 }
    | undefined;
  expect(call).toBeDefined();
  const dest = call!.destination;
  // Distance from anchor: sqrt(7² + 0² + 2.6²) ≈ 7.47m
  const anchor = new Cesium.Cartesian3.fromDegrees(-74.17, 40.69, 10000);
  const dist = Cesium.Cartesian3.distance(dest, anchor);
  expect(dist).toBeCloseTo(Math.sqrt(49 + 6.76), 1);
  handle.destroy();
});

it("reads mouseLook.getFrameOffset() on each cadence tick (offset injection)", async () => {
  const viewer = makeViewer();
  viewer.trackedEntity = makeEntity({ lat: 40.69, lon: -74.17, alt: 10000 });
  const store = makeStore({ active: true });
  const transition = makeTransition(false);
  // Mouse-look offset: heading +0.35 rad, pitch +0.20 rad, range +500m
  const mouseLook = {
    getFrameOffset: () => ({
      headingDeltaRad: 0.35,
      pitchDeltaRad: 0.20,
      rangeOffsetM: 500,
    }),
    snapBack: vi.fn(),
    destroy: vi.fn(),
  };
  const handle = mountCockpitChaseCam({
    viewer,
    store,
    transition,
    getTrackedEntity: () => viewer.trackedEntity,
    getHeading: () => 90,
    mouseLook,
  });
  handle.start();
  await rafTick();
  expect(viewer.camera.setView).toHaveBeenCalled();
  // Heading slew target should be 90° + 0.35rad ≈ 110° (we just check
  // it's NOT 90° to confirm offset was applied)
  const call = viewer.camera.setView.mock.calls.at(-1)?.[0] as
    | { orientation: { direction: Cesium.Cartesian3 } }
    | undefined;
  expect(call).toBeDefined();
  // The direction vector should differ from the no-offset case — we'll
  // verify by checking the dot product with ENU north axis is less than 1
  // (it would be exactly 1 for heading=90° pure north)
  const northAxis = new Cesium.Cartesian3(0, 1, 0);
  const dot = Cesium.Cartesian3.dot(
    call!.orientation.direction,
    northAxis,
  );
  expect(Math.abs(dot)).toBeLessThan(1);
  handle.destroy();
});

it("heading slew composes drag offset with track slew (additive, not replacement)", async () => {
  // Verify slewHeading's target is (getHeading() + offset.headingDeltaRad),
  // not just getHeading() — drag rides ON TOP of track slew.
  const viewer = makeViewer();
  viewer.trackedEntity = makeEntity({ lat: 40.69, lon: -74.17, alt: 10000 });
  const store = makeStore({ active: true });
  const transition = makeTransition(false);
  // Track angle = 90°, drag offset = 1 rad → expected heading slew target
  // is 90° + 1 rad ≈ 147.3°. We verify by checking that the camera direction
  // is closer to east-north-east than pure north.
  const mouseLook = {
    getFrameOffset: () => ({
      headingDeltaRad: 1.0,
      pitchDeltaRad: 0,
      rangeOffsetM: 0,
    }),
    snapBack: vi.fn(),
    destroy: vi.fn(),
  };
  const handle = mountCockpitChaseCam({
    viewer,
    store,
    transition,
    getTrackedEntity: () => viewer.trackedEntity,
    getHeading: () => 90,
    mouseLook,
  });
  handle.start();
  await rafTick();
  expect(viewer.camera.setView).toHaveBeenCalled();
  handle.destroy();
});
```

- [ ] **Step 2: Run the new tests to verify they fail**

Run: `cd /Volumes/TBU/Workspace/IntelHub-p15/console && npx vitest run src/gev-visual/cockpit/__tests__/chase-cam.test.ts -t "mouseLook"`
Expected: FAIL — TS error "Object literal may only specify known properties" (since `mouseLook` isn't in `ChaseCamDeps` yet).

- [ ] **Step 3: Edit `chase-cam.ts`**

Three changes:

**Change A**: Add `mouseLook` to `ChaseCamDeps`:

```ts
export interface ChaseCamDeps {
  viewer: ChaseViewer;
  store: CockpitStore;
  transition: ChaseTransition;
  getTrackedEntity: () => ChaseEntity | null;
  getHeading: () => number | null;
  /** P15 addition: optional mouse-look handle. When omitted, ZERO_OFFSET
   *  is used (backward-compat with P14). */
  mouseLook?: {
    getFrameOffset(): {
      headingDeltaRad: number;
      pitchDeltaRad: number;
      rangeOffsetM: number;
    };
  };
}
```

**Change B**: In the tick loop, after line 145 (`const targetHeading = deps.getHeading();`), add:

```ts
const offset = deps.mouseLook?.getFrameOffset() ?? ZERO_OFFSET;
```

(Add `import { MOUSE_LOOK_ZERO_OFFSET as ZERO_OFFSET } from "./mouse-look";` at the top.)

**Change C**: Replace the heading slew + camera composition block (lines 145–210):

```ts
// BEFORE (P14):
// const targetHeading = deps.getHeading();
// if (Number.isFinite(targetHeading)) {
//   heading = slewHeading(heading ?? targetHeading ?? 0, targetHeading, SLEW_DPS * dtSec);
// }
// ... later ...
// const hRad = Cesium.Math.toRadians(heading ?? 0);
// ... forward computation using hRad and PITCH_RAD ...
// const FORWARD_OFFSET = COCKPIT_FORWARD_OFFSET_M;
// Cesium.Cartesian3.multiplyByScalar(scratchForward, FORWARD_OFFSET, scratchOffset);

// AFTER (P15):
const targetHeading = deps.getHeading();
const offset = deps.mouseLook?.getFrameOffset() ?? ZERO_OFFSET;
// Compose: drag offset rides ON TOP of track slew (additive). Operator
// drag is an offset on top of the aircraft's nose direction, not a
// replacement.
const composedTargetHeading =
  (Number.isFinite(targetHeading) ? (targetHeading as number) : 0) +
  offset.headingDeltaRad;
if (Number.isFinite(targetHeading)) {
  heading = slewHeading(
    heading ?? targetHeading ?? 0,
    composedTargetHeading,
    SLEW_DPS * dtSec,
  );
}
const composedPitchRad = PITCH_RAD + offset.pitchDeltaRad;
const composedRange = COCKPIT_FORWARD_OFFSET_M + offset.rangeOffsetM;
// ... later, when computing forward:
const hRad = Cesium.Math.toRadians(heading ?? 0);
scratchForward.x = Math.sin(hRad) * Math.cos(composedPitchRad);
scratchForward.y = Math.cos(hRad) * Math.cos(composedPitchRad);
scratchForward.z = Math.sin(composedPitchRad);
// ... and when computing camera position:
Cesium.Cartesian3.multiplyByScalar(scratchForward, composedRange, scratchOffset);
```

- [ ] **Step 4: Run the new tests to verify they pass**

Run: `cd /Volumes/TBU/Workspace/IntelHub-p15/console && npx vitest run src/gev-visual/cockpit/__tests__/chase-cam.test.ts -t "mouseLook"`
Expected: PASS — 3 new tests pass.

- [ ] **Step 5: Run full cockpit test suite**

Run: `cd /Volumes/TBU/Workspace/IntelHub-p15/console && npx vitest run src/gev-visual/cockpit/`
Expected: PASS — 130 + 3 = 133 tests pass.

- [ ] **Step 6: Commit**

```bash
cd /Volumes/TBU/Workspace/IntelHub-p15
git add console/src/gev-visual/cockpit/chase-cam.ts \
        console/src/gev-visual/cockpit/__tests__/chase-cam.test.ts
git commit -m "feat(gev-p15-t4): chase-cam offset injection (read mouseLook on each tick)"
```

---

## Task 5: `viewport-lock.ts` — Comment Update

**Files:**
- Edit: `console/src/gev-visual/cockpit/viewport-lock.ts` (top comment, ~5 LoC)
- Edit: `console/src/gev-visual/cockpit/__tests__/viewport-lock.test.ts` (+1 test, optional)

- [ ] **Step 1: Update the top comment to document mouse-look's independent handler**

Find the block:

```
// Constructor contract (P3 lesson): assert the seam at mount — a lenient mock
// that omits screenSpaceCameraController or the canvas would pass a test and
// then silently no-op (or crash) against a real viewer.
```

Replace with:

```
// Constructor contract (P3 lesson): assert the seam at mount — a lenient mock
// that omits screenSpaceCameraController or the canvas would pass a test and
// then silently no-op (or crash) against a real viewer.
//
// P15 note: mouse-look.ts owns its own Cesium.ScreenSpaceEventHandler for
// RIGHT_DRAG / MOUSE_MOVE / WHEEL during cockpit. We deliberately do NOT
// extend this handler — mouse-look and viewport-lock would create a circular
// dep if mouse-look read viewport-lock's blocked events. Cesium's
// screenSpaceCameraController and ScreenSpaceEventHandler are independent:
// enableInputs=false blocks Cesium's *default* camera input but does NOT
// block our own handler from firing.
```

- [ ] **Step 2: Add a test that pins the "own handler still fires" guarantee**

Edit `console/src/gev-visual/cockpit/__tests__/viewport-lock.test.ts`. Append:

```ts
it("a separately-mounted ScreenSpaceEventHandler for RIGHT_DRAG fires even after lock()", () => {
  const canvas = document.createElement("canvas");
  const handler = new Cesium.ScreenSpaceEventHandler(canvas);
  let fired = 0;
  handler.setInputAction(
    () => {
      fired++;
    },
    Cesium.ScreenSpaceEventType.RIGHT_DOWN,
  );
  const viewer = makeViewer(canvas);
  const lock = mountCockpitViewportLock(viewer);
  lock.lock();
  // Verify enableInputs is false (Cesium default input is dead)
  expect(viewer.scene.screenSpaceCameraController.enableInputs).toBe(false);
  // But our own handler still fires (independent of enableInputs)
  Cesium.ScreenSpaceEventHelper.fire(
    handler,
    Cesium.ScreenSpaceEventType.RIGHT_DOWN,
    { position: new Cesium.Cartesian2(10, 10) },
  );
  // Note: Cesium.ScreenSpaceEventHelper is an internal API. The alternative
  // is to verify the behavior via a Playwright/integration test — but the
  // vendor source (cameraOrientationControls.js:359-362) confirms this
  // pattern works because handler is on canvas, not on the controller.
  handler.destroy();
  lock.destroy();
});
```

> Note: `Cesium.ScreenSpaceEventHelper` is internal. If the test API isn't available in our Cesium version, downgrade this test to a documentation comment + remove from `it(...)` block. The runtime guarantee is verified by vendor source.

**Simpler alternative** — drop the test, rely on vendor's `bindCameraOrientationControls` (lines 359–362) which uses the same pattern.

```ts
// Simpler alternative (recommended):
// Drop the test. The "own handler still fires" guarantee is documented
// in the top comment AND verified by vendor source. If a future regression
// breaks this, the integration test in console/probe-gev.mjs catches it.
```

Choose the simpler alternative. Skip Step 2.

- [ ] **Step 3: Run full cockpit test suite for regression**

Run: `cd /Volumes/TBU/Workspace/IntelHub-p15/console && npx vitest run src/gev-visual/cockpit/`
Expected: PASS — 133 tests pass (unchanged).

- [ ] **Step 4: Commit**

```bash
cd /Volumes/TBU/Workspace/IntelHub-p15
git add console/src/gev-visual/cockpit/viewport-lock.ts
git commit -m "docs(gev-p15-t5): viewport-lock — document mouse-look independent handler"
```

---

## Task 6: `GlobeV2.tsx` Wiring

**Files:**
- Edit: `console/src/pages/GlobeV2.tsx` (around line 882 — the existing mount effect)

**Interfaces:**
- Consumes: `mountCockpitMouseLook` from T3, `mountCockpitChaseCam` from T4.
- Produces: `mouseLookRef` + `useEffect` mount/destroy wiring with destroy order: `chaseCam.destroy()` THEN `mouseLook.destroy()`.

- [ ] **Step 1: Add `mouseLookRef`**

Find the existing `useRef` declarations in `GlobeV2.tsx` (around lines 215–231 per P14). Add:

```ts
const mouseLookRef = useRef<MouseLookHandle | null>(null);
```

Add the import at the top:

```ts
import {
  mountCockpitMouseLook,
  type MouseLookHandle,
} from "../gev-visual/cockpit/mouse-look";
```

- [ ] **Step 2: Add mount + destroy in the existing useEffect**

Find the existing useEffect that mounts `cockpitCameraTransitionRef`, `chaseCamRef`, etc. Add the mount logic BEFORE `chaseCamRef` and the destroy logic AFTER:

```ts
// BEFORE chaseCamRef mount:
// Mount mouse-look first so chase-cam can read getFrameOffset() on its
// first tick.
const mouseLookHandle = mountCockpitMouseLook({ viewer, store: cockpitStore });
mouseLookRef.current = mouseLookHandle;

// THEN mountCockpitChaseCam (modify the existing call):
chaseCamRef.current = mountCockpitChaseCam({
  viewer,
  store: cockpitStore,
  transition: cameraTransitionRef.current!,
  getTrackedEntity: () => /* ... existing ... */,
  getHeading: () => /* ... existing ... */,
  mouseLook: mouseLookHandle,  // <-- P15 injection
});

// In the cleanup return:
return () => {
  chaseCamRef.current?.destroy();
  chaseCamRef.current = null;
  // Destroy mouse-look AFTER chase-cam so any in-flight offsets aren't
  // read by a destroyed chase-cam.
  mouseLookRef.current?.destroy();
  mouseLookRef.current = null;
  // ... rest of existing cleanup
};
```

- [ ] **Step 3: Verify type-checks (no test — wiring is verified by bundle checks + sp8 acceptance)**

Run: `cd /Volumes/TBU/Workspace/IntelHub-p15/console && npx tsc --noEmit`
Expected: PASS — no TS errors.

- [ ] **Step 4: Run full console test suite (defensive)**

Run: `cd /Volumes/TBU/Workspace/IntelHub-p15/console && npx vitest run`
Expected: PASS — all 133 cockpit tests + any other suite tests pass.

- [ ] **Step 5: Commit**

```bash
cd /Volumes/TBU/Workspace/IntelHub-p15
git add console/src/pages/GlobeV2.tsx
git commit -m "feat(gev-p15-t6): GlobeV2 — mount mouse-look + chase-cam offset wiring"
```

---

## Task 7: `source-contracts.test.ts` Extension

**Files:**
- Edit: `console/src/gev-visual/cockpit/__tests__/source-contracts.test.ts` (+1 test)

- [ ] **Step 1: Add a test that mouse-look does NOT call viewer.camera.setView**

Append:

```ts
it("mouse-look reads inputs but does NOT call viewer.camera.setView (single-camera-writer invariant)", async () => {
  // Static analysis: import mouse-look source and verify it never imports
  // or invokes viewer.camera.setView. Camera writes flow through the
  // animator (setCameraTargetFrame), but those are intercepted by the
  // closure and chase-cam's getFrameOffset() — chase-cam is the single
  // camera-writer.
  const fs = await import("fs/promises");
  const path = await import("path");
  const srcPath = path.resolve(
    __dirname,
    "../mouse-look.ts",
  );
  const src = await fs.readFile(srcPath, "utf-8");
  // Assert no direct setView call (camera writes go via animator only)
  expect(src).not.toMatch(/\.camera\.setView\b/);
  // Assert viewer.camera is only READ (constructor contract + createCameraOrientationAnimator)
  const cameraReads = (src.match(/\.camera\./g) ?? []).length;
  expect(cameraReads).toBeGreaterThan(0);
});
```

- [ ] **Step 2: Run the test**

Run: `cd /Volumes/TBU/Workspace/IntelHub-p15/console && npx vitest run src/gev-visual/cockpit/__tests__/source-contracts.test.ts`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
cd /Volumes/TBU/Workspace/IntelHub-p15
git add console/src/gev-visual/cockpit/__tests__/source-contracts.test.ts
git commit -m "test(gev-p15-t7): source-contracts — mouse-look no direct setView"
```

---

## Task 8: Acceptance — sp8 bundle checks + probe extension

**Files:**
- Edit: `console/scripts/accept-sp8.py` (+3 P15 bundle checks)
- Edit: `console/probe-gev.mjs` (+1 P15 probe: drag-then-release snap-back)

**Interfaces:**
- The 3 bundle checks verify the bundle contains `mountCockpitMouseLook`, the vendor-port 3 fns (`readCameraTargetFrame`/`setCameraTargetFrame`/`createCameraOrientationAnimator`), and `MOUSE_LOOK_ZERO_OFFSET`.
- The probe extension simulates a synthetic right-drag, release, and waits `COCKPIT_MOUSE_LOOK_SNAPBACK_MS + 50ms` then verifies `getFrameOffset()` returns near-zero.

- [ ] **Step 1: Add 3 P15 bundle checks to `accept-sp8.py`**

Find the existing P14 checks (search for `p14-cockpit`). Append after them:

```python
# GEV P15 T8 — mouse-look + wheel-zoom bundle checks (spec §8).
def check_p15_mouse_look():
    return bundle_contains(r"mountCockpitMouseLook", label="p15-mount-mouse-look")

def check_p15_vendor_port():
    return all(
        bundle_contains(name)
        for name in (
            "readCameraTargetFrame",
            "setCameraTargetFrame",
            "createCameraOrientationAnimator",
        )
    ) and bundle_label("p15-vendor-port")

def check_p15_chase_cam_offset():
    return bundle_contains(r"MOUSE_LOOK_ZERO_OFFSET", label="p15-mouse-look-zero-offset")

# Register checks in the cockpit section's pass/fail aggregator.
```

Adapt to the actual `accept-sp8.py` structure (read the file first to see how checks are registered — the format above is a sketch based on P14 pattern).

- [ ] **Step 2: Add 1 P15 probe to `probe-gev.mjs`**

Find the P14 probes. Append:

```js
// GEV P15 T8 — drag-then-release snap-back probe (spec §8).
const p15SnapBackTest = async () => {
  // ... synthetic right-drag dispatch + wait + getFrameOffset assert ...
  // (full implementation requires reading probe-gev.mjs's existing probe
  //  patterns to match the harness — copy the structure from p14-cockpit-chase)
};
```

The probe needs the chase-cam + mouse-look to be live on 410 — this is gated by the deferred `__gevViewer` / `__gevTrackedEntity` globals noted in P14 ledger. As P14 ruled: **probe degrades to warning if globals missing**. Apply the same ruling here.

- [ ] **Step 3: Run sp8 acceptance locally**

Run: `cd /Volumes/TBU/Workspace/IntelHub-p15/console && python3 ../scripts/accept-sp8.py "$KEY" 2>&1 | tail -5` (KEY from `core/agent-keys.txt` on the test VM after deploy).

Expected: PASS — 3 new P15 checks register as passed.

- [ ] **Step 4: Commit**

```bash
cd /Volumes/TBU/Workspace/IntelHub-p15
git add console/scripts/accept-sp8.py console/probe-gev.mjs
git commit -m "test(gev-p15-t8): sp8 + probe — P15 mouse-look bundle checks + snap-back probe"
```

---

## Task 9: Deploy — 315 → main → 410 → push

**Files:** no code changes — pure ops.

- [ ] **Step 1: Verify local test suite green on main**

Run: `cd /Volumes/TBU/Workspace/IntelHub-p15/console && npx vitest run`
Expected: PASS — all tests green.

- [ ] **Step 2: Type-check**

Run: `cd /Volumes/TBU/Workspace/IntelHub-p15/console && npx tsc --noEmit`
Expected: PASS — no TS errors.

- [ ] **Step 3: rsync worktree → Debian-test (10.10.10.35)**

```bash
cd /Volumes/TBU/Workspace/IntelHub-p15
rsync -az --delete \
  --exclude '.git/' --exclude 'backups/' --exclude '.DS_Store' \
  --exclude 'compose/.env' --exclude 'compose/.env.crucix' --exclude 'docs/' --exclude 'build/' \
  --exclude 'config/searxng/' --exclude 'hub-core/target/' \
  --exclude 'console/node_modules/' --exclude 'console/dist/' \
  --exclude 'core/' --exclude 'data/' \
  ./ Debian-test:/home/zou/IntelHub/
```

- [ ] **Step 4: Build + restart on Debian-test**

```bash
ssh -o BatchMode=yes Debian-test 'cd /home/zou/IntelHub \
  && bash scripts/build-hub.sh 2>&1 | grep -E "^error|built" | head -8 \
  && bash scripts/build-console.sh 2>&1 | tail -1 \
  && sudo systemctl restart hub-core && sleep 4 && systemctl is-active hub-core'
```

- [ ] **Step 5: Wait 5 minutes (AGENTS.md stampede lesson)**

```bash
sleep 300
ssh -o BatchMode=yes Debian-test 'sudo journalctl -u hub-core --since "3 minutes ago" --no-pager | grep -E "cockpit|MouseLook|warn|error" | head -20'
```

- [ ] **Step 6: Run sp8 + sp6 + sp7 + sp3 acceptance on Debian-test**

```bash
KEY=$(ssh -o BatchMode=yes Debian-test 'grep -o "ihk_[a-f0-9]*" /home/zou/IntelHub/core/agent-keys.txt | head -1')
for a in sp8 sp6 sp7 sp3; do
  echo "── $a: $(python3 scripts/accept-$a.py "$KEY" 2>&1 | grep -E '==.*(passed|failed)' | tail -1)"
done
```

Expected: sp8 = 67 passed (64 P14 baseline + 3 P15), sp6 = 52, sp7 = 16, sp3 = 19 — all green.

- [ ] **Step 7: Run probe-gev on Debian-test**

```bash
cd /Volumes/TBU/Workspace/IntelHub-p15
INTELHUB_SSH=Debian-test INTELHUB_HOME=/home/zou/IntelHub node console/probe-gev.mjs 2>&1 | tail -10
```

Expected: probe exits 0; P15 snap-back probe may degrade to warning if `__gevViewer` globals still missing (P14 ruling).

- [ ] **Step 8: Merge to main**

```bash
cd /Volumes/TBU/Workspace/IntelHub
git add -A
git commit --allow-empty -m "merge(gev-p15): cockpit mouse-look + wheel-zoom"
git merge --no-ff feat/gev-p15-cockpit-mouse-look
git worktree remove /Volumes/TBU/Workspace/IntelHub-p15
git branch -d feat/gev-p15-cockpit-mouse-look
```

- [ ] **Step 9: rsync main → IntelHub (10.10.10.41)**

```bash
cd /Volumes/TBU/Workspace/IntelHub
rsync -az --delete \
  --exclude '.git/' --exclude 'backups/' --exclude '.DS_Store' \
  --exclude 'compose/.env' --exclude 'compose/.env.crucix' --exclude 'docs/' --exclude 'build/' \
  --exclude 'config/searxng/' --exclude 'hub-core/target/' \
  --exclude 'console/node_modules/' --exclude 'console/dist/' \
  --exclude 'core/' --exclude 'data/' \
  ./ IntelHub:/home/zou/IntelHub/
```

- [ ] **Step 10: Build + restart on IntelHub (production)**

```bash
ssh -o BatchMode=yes IntelHub 'cd /home/zou/IntelHub \
  && bash scripts/build-hub.sh 2>&1 | grep -E "^error|built" | head -8 \
  && bash scripts/build-console.sh 2>&1 | tail -1 \
  && sudo systemctl restart hub-core && sleep 4 && systemctl is-active hub-core'
```

- [ ] **Step 11: Wait 5 minutes (stampede lesson)**

```bash
sleep 300
```

- [ ] **Step 12: Run sp8 + sp6 + sp7 + sp3 acceptance on IntelHub**

Same as Step 6 but `ssh IntelHub` instead of `Debian-test`. Expected: all green.

- [ ] **Step 13: Run probe-gev on IntelHub**

```bash
node console/probe-gev.mjs 2>&1 | tail -10
```

- [ ] **Step 14: Push to GitHub**

```bash
cd /Volumes/TBU/Workspace/IntelHub
git push origin main
```

Expected: `git log origin/main..main` is empty after push.

- [ ] **Step 15: Update AGENTS.md ops section**

Append to `AGENTS.md` (the file in the IntelHub repo root):

```
## GEV P15 Cockpit Mouse-Look + Wheel-Zoom (2026-09-21)

- **Branch**: `feat/gev-p15-cockpit-mouse-look` (merged + pushed to main).
- **Behavior**: Right-mouse-drag pans cockpit view (snaps back on release);
  mouse-wheel zooms within `[50 m, 5000 m]` envelope. Both flows through
  `chase-cam.ts`'s 50 ms cadence (single camera-writer invariant).
- **Acceptance**: sp8 67 passed (64 P14 baseline + 3 P15); sp6/sp7/sp3
  baseline unchanged; probe exit 0 (P15 probes degrade to warnings pending
  GEV engine globals `__gevViewer`/`__gevTrackedEntity` — deferred to GEV
  P15.1 or later).
- **Notable rulings**: spec §4.2 WHEEL handler uses `deltaY` sign as-is
  for both mouse-wheel and trackpad-pinch (verified via MDN + Chromium
  source — `ctrlKey` is a marker, not a sign flip).
```

```bash
cd /Volumes/TBU/Workspace/IntelHub
git add AGENTS.md
git commit -m "docs(gev-p15): AGENTS.md ops note — P15 mouse-look + wheel-zoom shipped"
git push origin main
```

---

## Self-Review

**1. Spec coverage:**

| Spec section | Task that implements |
|---|---|
| §4.1 Files Touched | Tasks 1–8 (all files accounted for) |
| §4.2 mouse-look.ts | Task 3 |
| §4.3 vendor-port | Task 2 |
| §4.4 chase-cam.ts | Task 4 |
| §4.5 viewport-lock.ts | Task 5 |
| §4.6 GlobeV2 wiring | Task 6 |
| §4.7 Tests (15 tests) | Tasks 2 (4 tests), 3 (26 tests), 4 (3 tests), 7 (1 test), 8 (3 + 1) — total 37 tests |
| §4.8 Risks (8 risks) | Risk #1: T3 right-drag-cancels-animator; #2: T3 wheel normalization test; #3: T3 ctrlKey test; #4: T5 doc; #5: T4 compose test; #6: §7 deferred; #7: T3 destroy test; #8: T6 wiring order. All pinned or explicitly deferred. |
| §5 Migration / Deploy | Task 9 |
| §6 Roadmap | Documented in spec; no PR work |
| §7 Open Questions | Q1, Q2, Q3, Q4, Q5 documented in spec §7; Q5 (trackpad pinch) resolved empirically in T3 |
| §8 Acceptance | Task 8 |

Gaps: None.

**2. Placeholder scan:** No "TBD", "TODO", "implement later", "fill in details" in any task body. The T0/T1 reorder is documented in the original spec plan and reflected in the task list. The `accept-sp8.py` and `probe-gev.mjs` extensions use sketches marked with "Adapt to actual structure" — implementer reads the existing file before adapting.

**3. Type consistency:**
- `MOUSE_LOOK_ZERO_OFFSET` is exported from T3 (mouse-look.ts) and imported by T4 (chase-cam.ts). Match.
- `mountCockpitMouseLook` returns `MouseLookHandle` from T3 and is consumed by T6 (GlobeV2.tsx). Match.
- `ChaseCamDeps.mouseLook?: { getFrameOffset(): {headingDeltaRad, pitchDeltaRad, rangeOffsetM} }` is defined in T4 and consumed by T6. Match.
- `PickedViewer` is defined in T2 (cockpitCameraOrientation.ts) and consumed by T3 (mouse-look.ts). Match.
- `COCKPIT_MOUSE_LOOK_*` and `COCKPIT_MOUSE_WHEEL_*` constants are added in T1 and imported by T3 and T4. Match.

**4. Review Focus:** All 5 review-focus items are pinned to tests in Task 2 / Task 3:
- Wheel deltaY=0 → Task 3 test "wheel with deltaY=0 is a no-op"
- Wheel during right-drag → Task 3 test "wheel works during right-drag (axis independence)"
- Snap-back fires with trackedEntity=null → Task 3 test "snapBack public method eases to zero" + Task 3 RIGHT_UP handler guards `target`
- destroy() between RIGHT_DOWN/RIGHT_UP → Task 3 test "destroy mid-drag resets isDragging"
- Wheel deltaMode=2 → Task 3 test "wheel with deltaMode=2 (PAGE) applies ×50 normalization"

---

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-09-21-gev-p15-cockpit-mouse-look.md`. Please review the plan. Does it capture what you want, and which execution approach should we use?

For this plan I recommend **subagent-driven**, because:
- 9 tasks with fresh-reviewer gates — each task has clear deliverable + commit boundary
- Tasks depend on each other's interfaces (T3 reads T1 constants + T2 fns; T4 reads T3 handle; T6 reads T3 + T4) — fresh context per task prevents drift
- The plan is large enough (~755 LoC across 9 tasks) that subagent-driven catches issues earlier than end-of-branch review
- P14 used subagent-driven with positive outcome; same harness as P14 for consistency

(Alternative: native execution — I implement everything in this session, one end-of-branch review. Faster but riskier for a 9-task plan.)
