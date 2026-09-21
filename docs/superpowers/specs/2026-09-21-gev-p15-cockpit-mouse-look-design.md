# GEV P15 — Cockpit View: Mouse-Look + Wheel-Zoom

**Status**: architectural design — minimum scope
**Date**: 2026-09-21
**Branch**: `feat/gev-p15-cockpit-mouse-look`
**Base**: `f98c8e7` (main, post-GEV-P14)
**Path**: minimum only (§4). Roadmap for HUD/SVS remains §6 / §7.

## 1. Context

GEV P14 shipped a continuous chase-cam loop (50 ms cadence) that holds the
camera at `+7 m forward / +2.6 m up` from the tracked aircraft, with the
heading slewed to the aircraft's track angle and the model hidden. This
solves the "plane stays on screen / not first-person real-time" defect
but the cockpit is still **static**: the operator cannot look around the
aircraft or zoom out to inspect the surroundings.

The reference project `gods-eye-view/src/ui/cameraOrientationControls.js`
already provides the camera-math primitives we need:

- `readCameraTargetFrame(viewer)` — extract `(target, range, heading, pitch)`
  from the current camera state, handling Cesium's `EntityView` reference
  frame correctly (preserves satellite frames, plane-in-arc, etc.).
- `setCameraTargetFrame(viewer, frame)` — apply a new
  `(heading, pitch, range)` orbit frame around `target`, also preserving
  the trackedEntity reference frame.
- `createCameraOrientationAnimator(viewer)` — eases an orbit change over a
  duration (default 650 ms) using `viewer.scene.preUpdate`. Same mechanism
  we already use in P14's `chase-cam.ts` for anchor correction.

P15 ports these three functions into a new `mouse-look.ts` module and adds
the greenfield input handler (right-drag + wheel) plus the offset
injection that flows into the existing `chase-cam.ts` cadence loop.

## 2. Problem Statement

After P14 the cockpit is first-person but rigid: the operator cannot look
left / right / up / down or zoom in / out. Two missing affordances:

1. **Right-drag to pan view** — operator expects to be able to look
   around the cockpit (e.g. to spot traffic on the left, check the
   instruments at the bottom, etc.). On release the view should ease back
   to the chase-pose forward direction (so the next track-angle slew
   starts from forward, not from wherever the operator last dragged).
2. **Mouse-wheel to zoom** — operator expects to be able to zoom out
   (e.g. to inspect the surrounding airspace) and zoom back in (e.g. to
   check instruments). The current `viewport-lock.ts` disables Cesium's
   wheel handler via `enableInputs = false`; P15 adds a wheel handler of
   our own that operates within a `[50 m, 5000 m]` range envelope.

The two are coupled: a drag changes heading + pitch; a wheel changes
range. Both contribute to a single "frame offset" that the chase-cam
consumes on its next 50 ms tick.

## 3. Goals & Non-Goals

### Goals (minimum)

- G1: Right-mouse-button drag pans the camera heading + pitch around the
  tracked aircraft, with snap-back to `(Δheading=0, Δpitch=0)` on release.
- G2: Mouse wheel zooms the camera range within `[50 m, 5000 m]`
  envelope, where the range extends `COCKPIT_FORWARD_OFFSET_M = 7 m` to
  `+(7 + rangeOffsetM) m` (so the camera always rides the aircraft — no
  detachment, per spec §7.1 default ruling).
- G3: Mouse-look is **opt-in via cockpit-active** — outside cockpit,
  Cesium's default right-drag / wheel behaviours are preserved (operator
  can still pan the map).
- G4: Mouse-look has clean lifecycle — `mountCockpitMouseLook(deps) →
  { destroy }` with constructor-contract assertions on the seam
  (P3 lesson: a lenient mock that omits `viewer` would pass a test and
  then silently no-op against a real viewer).
- G5: All updates flow through the existing chase-cam cadence (50 ms)
  — no new render-loop driving `viewer.camera` outside that cadence.
  This keeps the "single camera-writer" invariant from P14.
- G6: Tests cover the input → offset path, the snap-back animator, the
  range clamp, and the integration with chase-cam (offset is read on
  next cadence tick).

### Non-goals (this PR)

- ❌ Detached-chase mode (camera leaves the aircraft) — spec §7.1 ruled
  out as out-of-scope; if "wheel override" semantics are wanted later,
  P15.1 follow-up.
- ❌ Inertia / momentum on snap-back — vendor uses
  `EasingFunction.CUBIC_IN_OUT` with no overshoot. Matches vendor.
- ❌ Smoothed wheel — instant per-tick (matches vendor; spec §6.1).
- ❌ Touch / pinch gestures — pointer events only (mouse + trackpad).
- ❌ HUD avionics upgrade — roadmap §6.2 (separate PR).
- ❌ Synthetic Vision System / TCAS / replay — roadmap §6.3
  (product-decision deferred).

## 4. Minimum-Scope Design

### 4.1 Files Touched

| File | Action | LoC |
|---|---|---|
| `console/src/gev-visual/cockpit/mouse-look.ts` | **new** | ~190 |
| `console/src/gev-visual/cockpit/__tests__/mouse-look.test.ts` | **new** | ~250 |
| `console/src/gev-visual/cockpit/vendor-port/cockpitCameraOrientation.ts` | **new** (vendored 3 fns from `cameraOrientationControls.js`) | ~120 |
| `console/src/gev-visual/cockpit/chase-cam.ts` | edit (read `mouseLook.getFrameOffset()` on each tick) | ~30 added |
| `console/src/gev-visual/cockpit/__tests__/chase-cam.test.ts` | edit (add offset-injection tests) | ~80 added |
| `console/src/gev-visual/cockpit/index.ts` | edit (barrel) | ~5 |
| `console/src/pages/GlobeV2.tsx` | edit (mount + destroy in existing useEffect) | ~10 |
| `console/src/gev-visual/cockpit/viewport-lock.ts` | edit (re-enable Cesium RIGHT_DRAG + WHEEL *only* during cockpit; mouse-look owns its own `ScreenSpaceEventHandler`) | ~10 edited |
| `console/gev-engine/src/ui/cockpitPresentation.js` | edit (add `COCKPIT_MOUSE_LOOK_*` constants, then sync to vendor) | ~10 added |
| `console/scripts/accept-sp8.py` | edit (+3 P15 bundle checks: mouse-look, vendor-port, chase-cam-offset-read) | ~20 added |
| `console/probe-gev.mjs` | edit (+1 P15 probe: drag-then-release → snap-back completes within 400 ms) | ~30 added |
| **Total** | | **~755** |

> LoC budget is higher than the §6.1 estimate of 150 because P15 also
> ports 3 functions from vendor (not just greenfield) and adds 5 new
> constants. Per the brainstorming skill — preferred honest budget
> over an underestimated one.

### 4.2 Module: `mouse-look.ts`

```ts
// GEV P15 T1 — cockpit mouse-look + wheel-zoom (spec §4.2).

import * as Cesium from "cesium";
import {
  COCKPIT_FORWARD_OFFSET_M,
  COCKPIT_VIEW_PITCH_DEG,
  COCKPIT_MOUSE_LOOK_YAW_RATE_RAD_PER_PX,
  COCKPIT_MOUSE_LOOK_PITCH_RATE_RAD_PER_PX,
  COCKPIT_MOUSE_LOOK_PITCH_CLAMP_RAD,
  COCKPIT_MOUSE_LOOK_SNAPBACK_MS,
  COCKPIT_MOUSE_LOOK_SNAPBACK_THRESHOLD_RAD,
  COCKPIT_MOUSE_WHEEL_RANGE_RATE_M_PER_DELTA,
  COCKPIT_MOUSE_WHEEL_RANGE_MIN_M,
  COCKPIT_MOUSE_WHEEL_RANGE_MAX_M,
} from "../../../gev-engine/src/ui/cockpitPresentation.js";
import {
  readCameraTargetFrame,
  setCameraTargetFrame,
  createCameraOrientationAnimator,
} from "./vendor-port/cockpitCameraOrientation.js";

export interface MouseLookViewer {
  scene: {
    canvas: HTMLCanvasElement;
    screenSpaceCameraController: { enableInputs: boolean };
  };
  camera: Cesium.Camera;
  clock: { currentTime: Cesium.JulianDate };
  isDestroyed?: () => boolean;
  trackedEntity?: Cesium.Entity | undefined;
}

export interface MouseLookStoreShape {
  getState(): { active: boolean };
  subscribe(listener: () => void): () => void;
}

export interface MouseLookFrameOffset {
  /** Snapshot of the current offsets for chase-cam to consume on its next
   *  cadence tick. Always finite; defaults to zero offsets. */
  readonly headingDeltaRad: number;
  readonly pitchDeltaRad: number;
  readonly rangeOffsetM: number;
}

export interface MouseLookDeps {
  viewer: MouseLookViewer;
  store: MouseLookStoreShape;
}

export interface MouseLookHandle {
  /** Returns the current frame offset (atomically readable by chase-cam). */
  getFrameOffset(): MouseLookFrameOffset;
  /** Force a snap-back to (0, 0, 0) offsets over COCKPIT_MOUSE_LOOK_SNAPBACK_MS. */
  snapBack(): void;
  /** Idempotent teardown — unsubscribes input, cancels animator. */
  destroy(): void;
}

export function mountCockpitMouseLook(deps: MouseLookDeps): MouseLookHandle {
  // Constructor contract (P3 lesson — asserts the seam at mount):
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
  if (!deps?.store?.getState || !deps?.store?.subscribe) {
    throw new TypeError(
      "mountCockpitMouseLook: deps.store.getState / subscribe missing",
    );
  }

  const viewer = deps.viewer;
  const canvas = viewer.scene.canvas;
  let destroyed = false;
  // Closure-scoped state — read atomically by chase-cam on next cadence tick.
  let headingDeltaRad = 0;
  let pitchDeltaRad = 0;
  let rangeOffsetM = 0;
  let isDragging = false;
  let lastDragX = 0;
  let lastDragY = 0;

  const animator = createCameraOrientationAnimator(viewer, {
    duration: COCKPIT_MOUSE_LOOK_SNAPBACK_MS,
  });

  // ... RIGHT_DRAG / WHEEL handlers, see implementation in plan §3.
  // ... mount/unmount on store.active transitions, see implementation in plan §3.
  // ... destroy() unsubscribes everything.

  return {
    getFrameOffset: () => ({
      headingDeltaRad,
      pitchDeltaRad,
      rangeOffsetM,
    }),
    snapBack: () => { /* ... */ },
    destroy: () => { /* ... */ },
  };
}
```

**Three input handlers** (each registered when `store.active === true`,
unregistered on deactivate or destroy):

1. **`RIGHT_DOWN`** (Cesium `ScreenSpaceEventType.RIGHT_DOWN`):
   - `isDragging = true; lastDragX = event.position.x; lastDragY = event.position.y`
   - Cancel any in-flight snap-back animator (operator grabbed the
     view mid-spring → abort the spring).

2. **`RIGHT_UP`** (Cesium `ScreenSpaceEventType.RIGHT_UP`):
   - `isDragging = false`
   - If `(headingDeltaRad² + pitchDeltaRad²)^½ ≥ COCKPIT_MOUSE_LOOK_SNAPBACK_THRESHOLD_RAD`,
     kick the animator with destination `(0, 0)` offsets → ease over
     `COCKPIT_MOUSE_LOOK_SNAPBACK_MS`.
   - Else, snap instantly to zero offsets (no animation needed for
     sub-threshold drags).

3. **`MOUSE_MOVE`** (Cesium `ScreenSpaceEventType.MOUSE_MOVE`):
   - Only fires `RIGHT_DOWN` ↔ `RIGHT_UP` window.
   - `dx = event.endPosition.x - lastDragX`; `dy = event.endPosition.y - lastDragY`
   - `headingDeltaRad += dx * COCKPIT_MOUSE_LOOK_YAW_RATE_RAD_PER_PX`
   - `pitchDeltaRad += dy * COCKPIT_MOUSE_LOOK_PITCH_RATE_RAD_PER_PX`
   - `pitchDeltaRad = clamp(pitchDeltaRad, COCKPIT_MOUSE_LOOK_PITCH_CLAMP_RAD[0], COCKPIT_MOUSE_LOOK_PITCH_CLAMP_RAD[1])`
   - `lastDragX/Y = event.endPosition.x/y`
   - Note: heading is NOT clamped — operator can spin 360° freely.

4. **`WHEEL`** (Cesium `ScreenSpaceEventType.WHEEL`):
   - Normalize `event.deltaY` by `event.deltaMode` (LINE × 3, PAGE × 50, pixel × 1).
   - **Sign convention**: wheel-up (`event.deltaY < 0`) means zoom IN → `rangeOffsetM` decreases (operator feels the aircraft grow larger). Wheel-down (`event.deltaY > 0`) means zoom OUT → `rangeOffsetM` increases.
   - Formula: `rangeOffsetM += event.deltaY * COCKPIT_MOUSE_WHEEL_RANGE_RATE_M_PER_DELTA`. (deltaY < 0 → offset decreases. deltaY > 0 → offset increases.)
   - `rangeOffsetM = clamp(rangeOffsetM, COCKPIT_MOUSE_WHEEL_RANGE_MIN_M - COCKPIT_FORWARD_OFFSET_M, COCKPIT_MOUSE_WHEEL_RANGE_MAX_M - COCKPIT_FORWARD_OFFSET_M)`
   - **Trackpad pinch**: per MDN + Chromium source, trackpad pinch fires the same `wheel` event with `event.ctrlKey === true` and the SAME sign convention as mouse wheel (the browser synthesizes pinch into wheel with natural direction). The `ctrlKey` is purely a marker for pages that want to differentiate. **No sign flip needed** — but we record `ctrlKey` so a future revision could surface "you pinch-zoomed" in UI. Test #7 covers both `ctrlKey=true` and `ctrlKey=false` paths with the same expected `rangeOffsetM` sign.

   (We store the **offset**, not the absolute range. Chase-cam adds
   `COCKPIT_FORWARD_OFFSET_M` to `rangeOffsetM` when computing the actual
   forward distance. So at rest (offset=0) forward is 7 m; wheel-up
   pushes offset toward `-4993 m`, mapping to forward at the
   `COCKPIT_MOUSE_WHEEL_RANGE_MIN_M = 50 m` clamp. Wheel-down pushes
   offset toward `+4993 m`, mapping to forward at the `5000 m` clamp.)

### 4.3 Module: `vendor-port/cockpitCameraOrientation.ts`

Port of three functions from
`/Volumes/TBU/Github/gods-eye-view/src/ui/cameraOrientationControls.js`,
adapted to TypeScript + IntelHub types. The vendored code is
**byte-stable upstream** — only the file extension / module format
changes; logic stays identical.

| Function | Lines (vendor) | Notes |
|---|---|---|
| `readCameraTargetFrame(viewer)` | 60–93 (33 lines) | Reads `(target, range, heading, pitch)` from current camera. Handles Cesium's EntityView reference frame correctly. |
| `setCameraTargetFrame(viewer, frame)` | 95–128 (33 lines) | Applies new `(heading, pitch, range)` orbit. Preserves EntityView frame. |
| `createCameraOrientationAnimator(viewer, opts)` | 188–229 (42 lines) | Eases an orbit change over `opts.duration` using `viewer.scene.preUpdate`. |

These three functions are pure camera math — no input handling, no
state. They become the substrate for mouse-look and (in P15.1+)
detached-chase mode if that ships.

**Vendor sync procedure** (matches AGENTS.md GEV pin workflow):
- After port, run `bash scripts/sync-vendor.sh cockpitCameraOrientation`
  (or equivalent) to detect drift.
- Source-contracts test (T5) asserts no call sites outside this module
  import from `gev-engine` directly — all cockpit camera-math flows
  through the port.

### 4.4 Module: `chase-cam.ts` (edit)

Add a new optional dep `mouseLook?: MouseLookHandle` to `ChaseCamDeps`.
In the 50 ms cadence tick (currently at line ~140 in P14's chase-cam.ts):

```ts
// Currently (P14):
const heading = info.track;  // raw track angle
const forward = computeForward(heading, COCKPIT_VIEW_PITCH_DEG);
const cameraPos = anchor + forward * COCKPIT_FORWARD_OFFSET_M + up * COCKPIT_UP_OFFSET_M;

// P15 addition:
const offset = deps.mouseLook?.getFrameOffset() ?? ZERO_OFFSET;
const heading = info.track + offset.headingDeltaRad;
const pitch = COCKPIT_VIEW_PITCH_DEG + offset.pitchDeltaRad;
const forward = computeForward(heading, pitch);
const range = COCKPIT_FORWARD_OFFSET_M + offset.rangeOffsetM;
const cameraPos = anchor + forward * range + up * COCKPIT_UP_OFFSET_M;
```

`ZERO_OFFSET` (constant `{ headingDeltaRad: 0, pitchDeltaRad: 0, rangeOffsetM: 0 }`)
preserves P14 behavior when `mouseLook` is omitted — backward-compatible.

The heading slew (`slewHeading(currentHeading, info.track + offset.headingDeltaRad, ...)`)
still applies — drag offsets compose with the track-angle slew rather
than overriding it. Operator drag is an *offset* on top of the
aircraft's nose direction, not a replacement.

### 4.5 Module: `viewport-lock.ts` (edit)

Currently `viewport-lock.ts` sets
`screenSpaceCameraController.enableInputs = false` to kill Cesium's
camera input (rotate / zoom / pinch / middle-drag). This blocks Cesium's
default RIGHT_DRAG and WHEEL — which is what we want (Cesium's default
RIGHT_DRAG would pan the map, not the cockpit). P15 keeps `enableInputs
= false` (we want NO Cesium default input during cockpit) but does NOT
block our own `ScreenSpaceEventHandler` for RIGHT_DRAG / WHEEL — Cesium's
camera and our handler coexist independently.

`viewport-lock.ts` already adds its own `ScreenSpaceEventHandler` blocking
LEFT_CLICK / LEFT_DOUBLE_CLICK / RIGHT_CLICK. We do NOT extend that
handler — instead, mouse-look owns its own `ScreenSpaceEventHandler` for
RIGHT_DRAG / MOUSE_MOVE / WHEEL (separation of concerns; each module
owns its lifecycle).

> **Why not extend viewport-lock's existing handler?** It would create
> a circular dep (viewport-lock → mouse-look → cockpitStore →
> viewport-lock). Each module owning its own handler keeps the dep
> graph acyclic.

### 4.6 Wiring: `pages/GlobeV2.tsx` (line ~882)

Existing P14 mount effect (line ~882 in current `GlobeV2.tsx`) wires
`model-visibility`, `chase-cam`, `camera-transition`. P15 adds:

```ts
const mouseLookRef = useRef<MouseLookHandle | null>(null);
useEffect(() => {
  if (!cockpitStore.getState().active) return;
  const handle = mountCockpitMouseLook({ viewer, store: cockpitStore });
  mouseLookRef.current = handle;
  chaseCamRef.current = mountCockpitChaseCam({
    viewer,
    store: cockpitStore,
    transition: cameraTransitionRef.current!,
    mouseLook: handle,  // <-- P15 injection
  });
  // ... rest unchanged
  return () => {
    handle.destroy();
    mouseLookRef.current = null;
  };
}, [/* deps unchanged */]);
```

The `mouseLook` handle is created BEFORE `chaseCam` so chase-cam's
`getFrameOffset()` closure can read it from the first tick. On unmount,
`mouseLook.destroy()` is called AFTER `chaseCam.destroy()` so any
in-flight mouse-look offsets aren't read by a destroyed chase-cam.

### 4.7 Tests

**`mouse-look.test.ts`** (10 tests):

1. Constructor contract — missing `viewer.scene.canvas` throws `TypeError`.
2. Constructor contract — missing `viewer.camera` throws `TypeError`.
3. Constructor contract — missing `store.getState / subscribe` throws
   `TypeError`.
4. `getFrameOffset()` defaults to `(0, 0, 0)` after mount.
5. `RIGHT_DOWN` + `MOUSE_MOVE` → `headingDeltaRad` updates by
   `dx * COCKPIT_MOUSE_LOOK_YAW_RATE_RAD_PER_PX`; pitch similarly.
6. `MOUSE_MOVE` past `COCKPIT_MOUSE_LOOK_PITCH_CLAMP_RAD` clamps the
   pitch; heading does not clamp.
7. `WHEEL` updates `rangeOffsetM`; sign convention: `deltaY < 0`
   (mouse wheel-up OR trackpad pinch-in) → `rangeOffsetM` decreases
   (zoom IN, camera closer); `deltaY > 0` (wheel-down OR pinch-out) →
   `rangeOffsetM` increases (zoom OUT). `event.ctrlKey` is recorded
   but does NOT change the sign.
8. `WHEEL` past `COCKPIT_MOUSE_WHEEL_RANGE_MIN_M - COCKPIT_FORWARD_OFFSET_M`
   clamps; same for max.
9. `RIGHT_UP` with delta above threshold kicks the animator; animator's
   `preUpdate` events ease offsets to zero over
   `COCKPIT_MOUSE_LOOK_SNAPBACK_MS` (test uses `vi.useFakeTimers` +
   manual `preUpdate` fire loop).
10. `destroy()` unsubscribes all 4 handlers; subsequent `RIGHT_DOWN` etc.
    do not mutate offset; subsequent `getFrameOffset()` still returns
    last value (closure-scoped).

**`chase-cam.test.ts` edits** (3 new tests):

11. With `mouseLook` provided, chase-cam reads `getFrameOffset()` on each
    cadence tick and applies to forward + range.
12. With `mouseLook` omitted, chase-cam uses `ZERO_OFFSET` (backward-compat
    — identical behavior to P14).
13. Heading slew composes: `slewHeading` target becomes
    `info.track + offset.headingDeltaRad`; drag offset rides on top of
    track slew, doesn't replace it.

**`viewport-lock.test.ts` edit** (1 new test):

14. After `lock()`, Cesium's `screenSpaceCameraController.enableInputs` is
    `false` (unchanged) but a separate `ScreenSpaceEventHandler` for
    RIGHT_DRAG can still be registered (we verify the canvas still
    accepts our own handler by injecting one and firing a synthetic
    RIGHT_DOWN — verify the handler fires).

**`source-contracts.test.ts` edit** (1 new test):

15. `mouse-look.ts` does NOT import from `Cesium.Camera` for camera
    control (it reads `viewer.scene.canvas` and uses Cesium
    `ScreenSpaceEventHandler` for input, but camera writes flow through
    `setCameraTargetFrame` from the vendor port — which we verify in
    the source-contracts test).

### 4.8 Risks

| # | Risk | Probability | Mitigation |
|---|---|---|---|
| 1 | Right-drag during snap-back freezes the view at wrong angle | Medium | Snap-back animator listens for `RIGHT_DOWN` and cancels immediately (vendor pattern, line 359–362 of `cameraOrientationControls.js`). |
| 2 | Wheel event normalised wrong on Linux (deltaMode=1 line) | Low | Test #8 covers all 3 deltaMode values + the ×3 / ×50 normalisations. |
| 3 | Trackpad pinch delivers opposite-sign `deltaY` to mouse wheel (browser quirk) | **Resolved**: per MDN + Chromium source (`TouchpadPinchEventQueue::CreateSyntheticWheelFromTouchpadPinchEvent`), pinch is synthesised to wheel with the SAME sign — `ctrlKey=true` is purely a marker. No sign flip needed in handler. Test #7 covers both `ctrlKey=true` and `ctrlKey=false` paths with same expected sign. |
| 4 | Cesium's `screenSpaceCameraController.enableInputs = false` blocks our `ScreenSpaceEventHandler` for RIGHT_DRAG | High (initial belief) | Confirmed via Cesium docs: `screenSpaceCameraController` and `ScreenSpaceEventHandler` are independent. Our handler fires regardless of `enableInputs`. Tested in #14. |
| 5 | Heading slew conflict — drag offset overrides track slew or vice versa | Medium | Tested in #13. Composition is `slewHeading(current, track + offset.heading, ...)`. Drag offset rides ON TOP of track slew (additive). |
| 6 | Multi-monitor pixel-density scaling makes drag feel inconsistent | Low | Mouse positions come from `event.position` (CSS pixels, not device pixels). Cesium normalizes internally. Vendor uses the same coordinate space — known compatible. |
| 7 | `destroy()` called mid-snap-back leaves animator half-applied | Low | `destroy()` calls `animator.cancel()` first (vendor pattern), then unsubscribes handlers. Test #10 covers. |
| 8 | `getFrameOffset()` called by chase-cam AFTER `destroy()` returns stale value | Low | Document that chase-cam must destroy BEFORE mouse-look destroys (wiring in §4.6 enforces this order). If chase-cam reads after destroy, it gets the last-set offsets — no crash, but stale. Acceptable. |

## 5. Migration / Deploy Notes

- **No env-var changes** — purely console bundle.
- **No DB / API changes** — purely console bundle.
- **Vendor constants added**: `COCKPIT_MOUSE_LOOK_*` and
  `COCKPIT_MOUSE_WHEEL_*` in `console/gev-engine/src/ui/cockpitPresentation.js`.
  Per AGENTS.md GEV pin workflow, the next `sync-vendor.sh` will detect
  drift and propose a re-pin (no action required if vendor hasn't added
  these constants yet — they'll be IntelHub-only extensions).
- **315 → 410 sequence** per AGENTS.md iron rule. Worktree isolated on
  `feat/gev-p15-cockpit-mouse-look`.
- **Restart wait** — `sleep 300` after `sudo systemctl restart hub-core`
  (P14 stampede lesson).
- **sp8 baseline regression** — every prior P9–P14 check stays green.
  3 new bundle checks + 1 new probe extension.

## 6. Roadmap (NOT in this PR)

- **§6.1.1 Detached-chase mode** — wheel can detach camera from aircraft
  up to 5000 m. Currently out per spec §7.1 ruling. Reopen if
  "ride the aircraft" UX proves insufficient in production.
- **§6.2 HUD avionics upgrade** — heading tape / altitude ladder /
  speed tape / pitch ladder / bank indicator / vertical speed chevron,
  driven by chase-cam's `cockpitAnchor` + `heading` at 100 ms cadence.
  ~300 LoC. Blocked behind P15 because HUD reads chase-cam's
  `(heading, pitch, range)` outputs (now drag-offset-aware).
- **§6.3 SVS / TCAS / replay** — separate sub-project, product decision
  required.

## 7. Open Questions / Deferred Decisions

1. **Multi-monitor HiDPI drag feel** — see Risk #6. If users complain,
   we can add a `clientX/clientY`-based fallback (currently Cesium's
   `event.position` is used; switch to raw `event.clientX/Y` if needed).
   Tracked for §6.1.1 if it bites in production.
5. **Trackpad pinch direction** — verified: Mac trackpad pinch
   produces `wheel` events with `ctrlKey=true` and the SAME sign
   convention as a mouse wheel. No special handling needed for the
   direction; `ctrlKey` is recorded (unused for now) so a future UI
   could surface "you pinch-zoomed" affordance.
2. **Snap-back during flight-plan view** — if operator opens the
   flight-plan panel during snap-back, the animator continues to ease
   (animator is on `viewer.scene.preUpdate`, independent of panel
   state). Acceptable per spec §6.1.
3. **Wheel without right-drag first** — wheel-zoom works without
   right-drag having occurred (it's a separate input axis). Operator
   can wheel-zoom first, then right-drag to look around. Spec
   confirms this is desired (§6.1 "mouse-wheel zooms" — independent of
   drag state).
4. **`snapBack()` public method** — exposed for `cockpitStore.exit()` to
   call so the cockpit-exit `flyBackToBaseline` flies from the
   post-snap-back forward pose, not from wherever the operator last
   dragged. Wiring in §4.6 invokes `snapBack()` on `cockpitStore.exit()`
   *before* `cameraTransition.flyBackToBaseline(...)`.

## 8. Acceptance Plan

| SP | New / extended | Probe |
|---|---|---|
| sp8 (cockpit overlay) | +3 bundle checks: mouse-look, vendor-port, chase-cam-offset-read | (probe extension below) |
| sp8 (probe extension) | +1 check: drag-then-release → snap-back completes within `COCKPIT_MOUSE_LOOK_SNAPBACK_MS + 50ms` window | drag right-mouse 200 px → release → wait `COCKPIT_MOUSE_LOOK_SNAPBACK_MS + 50ms` → `getFrameOffset()` returns `(0±0.001, 0±0.001, _)` |
| sp8 (probe extension) | +1 check: wheel-zoom updates rangeOffset (mouse wheel, not trackpad pinch) | dispatch synthetic `wheel` event with `deltaY=-100` (wheel-up = zoom in) → `getFrameOffset().rangeOffsetM` is `-2500 ± 1` m (offset decreases) |
| sp6 / sp7 / sp3 | — | baseline regression |

Total addition: 3 bundle checks + 2 probe extensions. No SP
degradation.

## 9. References

- Reference impl: `/Volumes/TBU/Github/gods-eye-view/src/ui/cameraOrientationControls.js`
  (lines 60–229 for the three ported functions; lines 359–362 for the
  snap-back cancellation pattern).
- Vendor math: `/Volumes/TBU/Github/gods-eye-view/src/cockpitMath.js` —
  reused (slewHeading) via P14's existing imports; no new math
  required for mouse-look (the frame math is in
  `cameraOrientationControls.js`).
- Vendor constants: `/Volumes/TBU/Github/gods-eye-view/src/ui/cockpitPresentation.js`
  — no `COCKPIT_MOUSE_*` constants upstream; P15 introduces them as
  IntelHub-only extensions.
- Vendored copies in IntelHub: `console/gev-engine/src/ui/cockpitPresentation.js`
  (byte-identical per AGENTS.md GEV engine pin).
- Prior work: GEV P9 (`feat/gev-p9-cockpit-briefing`), GEV P12
  (`ea7ae98`), GEV P13 (`0086434`), GEV P14 (`f98c8e7`).
- AGENTS.md iron rules: 410 production isolation, worktree-first,
  315-test-first deploy.
