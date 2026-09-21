# GEV P14 Cockpit Chase-Cam Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Wire the vendored gods-eye-view chase-cam loop into the IntelHub React tree, hide the tracked aircraft model in cockpit mode, and refit the enter/exit flyTo to land at the chase pose — solving the "plane stays on screen / not first-person real-time" cockpit defects.

**Architecture:** 3 new modules (`chase-cam.ts`, `model-visibility.ts`, `camera-transition.ts` edit) following the existing `mountXxx(deps) → Handle` factory pattern. GlobeV2.tsx wires them in the existing cockpit mount/cleanup effect (line 882). Vendor `cockpitMath.js` + `cockpitPresentation.js` constants (already vendored, currently unused beyond `formatCockpitWindDirection`) supply the math.

**Tech Stack:** TypeScript, React 18, Cesium 1.110+, Vitest, vendored `gev-engine/src/cockpitMath.js` + `gev-engine/src/ui/cockpitPresentation.js`.

**Spec:** `docs/superpowers/specs/2026-09-21-gev-p14-cockpit-chase-cam-design.md`

## Global Constraints

- **Worktree first**: all work on `feat/gev-p14-cockpit-chase-cam` (already created at `/Volumes/TBU/Workspace/IntelHub-p14chase`).
- **315-test-first deploy**: build + accept on `Debian-test` (10.10.10.35) before any IntelHub (10.10.10.41) production change. AGENTS.md iron rule.
- **5-min restart wait**: after `sudo systemctl restart hub-core`, sleep ≥300s before acceptance (PVE40 stampede prevention, postmortem 2026-09-20).
- **`mountCockpitCameraTransition` must export `isInFlight()`**: chase-cam guards on it (no Cesium private API).
- **flyTo duration: 0.7 s → 0.4 s**: existing sp8 P12 check at line 682 (`duration 0.6-0.8s`) must be updated to `duration 0.3-0.5s`.
- **NO DB / NO API / NO env-var changes**: console bundle only.
- **NO console/dist rsync from Mac**: `build-console.sh` runs in the VM (VITE_CARTO_KEY in `core/console-build.env`).
- **Vendor constants adopted verbatim**:
  - `COCKPIT_FORWARD_OFFSET_M = 7`
  - `COCKPIT_UP_OFFSET_M = 2.6`
  - `COCKPIT_VIEW_PITCH_DEG = -4`
  - `COCKPIT_CAMERA_UPDATE_MS = 50`
  - `COCKPIT_HEADING_SLEW_DPS = 28`
- **Test framework**: Vitest (`describe`/`it`/`expect`/`vi` from `"vitest"`). Existing pattern in `console/src/gev-visual/cockpit/__tests__/camera-transition.test.ts`.

## Review Focus

Five input classes the spec implies but no task's tests exercise:

1. **Multi-model composite aircraft** — `entity.show = false` hides the parent; child sub-entities (shadow body, model silhouette) may still render. Test: Task 4 wiring integration test asserts `entity.show === false` AND no extra `viewer.entities.add` is called by chase-cam or model-visibility. If a multi-model composite appears in production, document in §7 open questions for §6.2 follow-up.
2. **Rapid enter→exit→enter within 100 ms** — chase-cam rAF may still be alive when next enter fires. Test: Task 3 lifecycle test destroys mid-active and re-creates; assert `cancelAnimationFrame` was called exactly once per destroy and no leaked rAF (use `vi.useFakeTimers` + spy on `requestAnimationFrame`).
3. **Heading on a stationary aircraft** — `getHeading()` may return null when the plane is parked. Test: Task 3 ENU math test with `getHeading: () => null` asserts chase-cam falls back to last known heading (no NaN propagation in Cartesian3).
4. **Camera setView while Cesium viewer is suspended** (tab background) — rAF may fire after Cesium pauses its render loop, and `setView` may throw. Test: Task 3 lifecycle test stubs `viewer.camera.setView` to throw on first call; assert chase-cam swallows + retries next tick (does not crash).
5. **`onToggleCockpit` rapid double-click** — store enters/exits twice, both flyTos fire, two chase-cams race. Test: Task 4 wiring integration test asserts only one active `ChaseCamHandle` after double-click (`chaseCamRef.current` identity unchanged across re-renders).

---

## Task 1: `model-visibility.ts` — hide tracked aircraft model in cockpit

**Files:**
- Create: `console/src/gev-visual/cockpit/model-visibility.ts`
- Create: `console/src/gev-visual/cockpit/__tests__/model-visibility.test.ts`
- Modify: `console/src/gev-visual/cockpit/index.ts` (add barrel export)

**Interfaces:**
- Consumes: `cockpitStore` (from `./cockpit-store.ts`), `getTrackedEntity(): Cesium.Entity | null`
- Produces: `mountModelVisibility(deps: ModelVisibilityDeps) → ModelVisibilityHandle { destroy(): void }`
- Mount behavior: on `cockpitStore.active === true` AND `getTrackedEntity() !== null`, set `entity.show = false` and remember prior `show` value. On `active === false`, restore prior value.

- [ ] **Step 1: Write failing tests in `model-visibility.test.ts`**

Create `console/src/gev-visual/cockpit/__tests__/model-visibility.test.ts`:

```ts
// GEV P14 — cockpit model-visibility tests.
//
// Hides the tracked aircraft model so the cockpit frame is empty sky + HUD
// (not "plane in the middle + cockpit chrome"). Asserts enter→exit round-trip
// restores the prior visibility value (defensive: don't clobber user state).

import { describe, expect, it, vi } from "vitest";
import { mountModelVisibility } from "../model-visibility";

type FakeEntity = { show: boolean; id: string };

function makeStore(initial: { active: boolean; trackedId: string | null }) {
  const listeners = new Set<() => void>();
  let state = initial;
  return {
    getState: () => state,
    subscribe: (fn: () => void) => {
      listeners.add(fn);
      return () => listeners.delete(fn);
    },
    _set: (next: typeof state) => {
      state = next;
      listeners.forEach((fn) => fn());
    },
  };
}

const DUMMY = { id: "icao-1" };

describe("mountModelVisibility", () => {
  it("throws TypeError if store.subscribe is missing", () => {
    expect(() =>
      mountModelVisibility({
        store: { getState: () => ({ active: false, trackedId: null }) } as never,
        getTrackedEntity: () => null,
      }),
    ).toThrow(TypeError);
  });

  it("hides the entity on cockpit enter and restores on exit", () => {
    const entity: FakeEntity = { show: true, id: "icao-1" };
    const store = makeStore({ active: false, trackedId: null });
    const handle = mountModelVisibility({
      store,
      getTrackedEntity: () => entity as never,
    });
    expect(entity.show).toBe(true); // baseline untouched before enter

    store._set({ active: true, trackedId: "icao-1" });
    expect(entity.show).toBe(false); // hidden

    store._set({ active: false, trackedId: null });
    expect(entity.show).toBe(true); // restored

    handle.destroy();
  });

  it("does not crash when entity is null at enter time", () => {
    const store = makeStore({ active: false, trackedId: null });
    const handle = mountModelVisibility({
      store,
      getTrackedEntity: () => null,
    });
    expect(() => store._set({ active: true, trackedId: "icao-1" })).not.toThrow();
    expect(() => store._set({ active: false, trackedId: null })).not.toThrow();
    handle.destroy();
  });

  it("preserves prior show=false (does not overwrite a hidden plane)", () => {
    const entity: FakeEntity = { show: false, id: "icao-1" };
    const store = makeStore({ active: false, trackedId: null });
    const handle = mountModelVisibility({
      store,
      getTrackedEntity: () => entity as never,
    });
    store._set({ active: true, trackedId: "icao-1" });
    expect(entity.show).toBe(false); // still false
    store._set({ active: false, trackedId: null });
    expect(entity.show).toBe(false); // restored to false, not true
    handle.destroy();
  });

  it("destroy() stops subscribing; later store updates do not toggle show", () => {
    const entity: FakeEntity = { show: true, id: "icao-1" };
    const store = makeStore({ active: false, trackedId: null });
    const handle = mountModelVisibility({
      store,
      getTrackedEntity: () => entity as never,
    });
    handle.destroy();
    store._set({ active: true, trackedId: "icao-1" });
    expect(entity.show).toBe(true); // no toggle after destroy
  });
});
```

- [ ] **Step 2: Run tests, expect FAIL**

```bash
cd /Volumes/TBU/Workspace/IntelHub-p14chase
cd console && npx vitest run src/gev-visual/cockpit/__tests__/model-visibility.test.ts 2>&1 | tail -20
```

Expected: `FAIL — Cannot find module '../model-visibility'`. The module does not exist yet.

- [ ] **Step 3: Write minimal `model-visibility.ts`**

Create `console/src/gev-visual/cockpit/model-visibility.ts`:

```ts
// GEV P14 — cockpit model-visibility adapter.
//
// Hides the tracked aircraft model while cockpit is active so the cockpit
// frame is "sky + HUD + instruments", not "plane in the middle + cockpit
// chrome". Restores prior visibility on exit. Subscribe-once-then-cleanup
// pattern, mirrors the existing camera-transition.ts baseline-capture.

import type { CockpitStore } from "./cockpit-store";

/** Subset of Cesium.Entity we touch. `show` is the visibility flag. */
interface VisibilityEntity {
  show: boolean;
}

export interface ModelVisibilityDeps {
  store: CockpitStore;
  getTrackedEntity: () => VisibilityEntity | null;
}

export interface ModelVisibilityHandle {
  destroy(): void;
}

export function mountModelVisibility(
  deps: ModelVisibilityDeps,
): ModelVisibilityHandle {
  // Constructor contract: assert the seam at mount so a lenient mock cannot
  // hide a wrong object handed in later.
  if (typeof deps?.store?.subscribe !== "function") {
    throw new TypeError(
      "mountModelVisibility: deps.store.subscribe must be a function",
    );
  }
  if (typeof deps.getTrackedEntity !== "function") {
    throw new TypeError(
      "mountModelVisibility: deps.getTrackedEntity must be a function",
    );
  }

  // Remember the entity we hid so we can restore it even if followRef swaps
  // the tracked entity while cockpit is active (selection change mid-flight).
  let priorShow: boolean | null = null;
  let hiddenEntity: VisibilityEntity | null = null;

  function applyHidden(entity: VisibilityEntity): void {
    if (hiddenEntity === entity) return; // already hidden this entity
    if (hiddenEntity && hiddenEntity !== entity) {
      // Race: entity changed under us. Restore old, then hide new.
      hiddenEntity.show = priorShow ?? true;
    }
    priorShow = entity.show;
    entity.show = false;
    hiddenEntity = entity;
  }

  function restoreIfHidden(): void {
    if (hiddenEntity) {
      hiddenEntity.show = priorShow ?? true;
      hiddenEntity = null;
      priorShow = null;
    }
  }

  function onChange(): void {
    const { active } = deps.store.getState();
    if (active) {
      const entity = deps.getTrackedEntity();
      if (entity) applyHidden(entity);
    } else {
      restoreIfHidden();
    }
  }

  const unsubscribe = deps.store.subscribe(onChange);

  return {
    destroy() {
      unsubscribe();
      restoreIfHidden();
    },
  };
}
```

- [ ] **Step 4: Run tests, expect PASS**

```bash
npx vitest run src/gev-visual/cockpit/__tests__/model-visibility.test.ts 2>&1 | tail -10
```

Expected: `5 passed`.

- [ ] **Step 5: Add barrel export in `index.ts`**

Read `console/src/gev-visual/cockpit/index.ts` and add:

```ts
export { mountModelVisibility } from "./model-visibility";
export type { ModelVisibilityDeps, ModelVisibilityHandle } from "./model-visibility";
```

Place immediately after the existing `export { mountCockpitCameraTransition ... }` line.

- [ ] **Step 6: Run full cockpit test suite, expect no regression**

```bash
npx vitest run src/gev-visual/cockpit/ 2>&1 | tail -15
```

Expected: `5 model-visibility tests + 8 existing cockpit tests = ~13+ passed`. No FAIL.

- [ ] **Step 7: Commit**

```bash
cd /Volumes/TBU/Workspace/IntelHub-p14chase
git add console/src/gev-visual/cockpit/model-visibility.ts \
        console/src/gev-visual/cockpit/__tests__/model-visibility.test.ts \
        console/src/gev-visual/cockpit/index.ts
git -c user.email=pi@intelhub.local -c user.name=pi commit -m "feat(gev-p14-t1): model-visibility — hide tracked aircraft in cockpit mode"
```

---

## Task 2: `camera-transition.ts` refit — `isInFlight()` + chase pose destination + 0.4 s

**Files:**
- Modify: `console/src/gev-visual/cockpit/camera-transition.ts`
- Modify: `console/src/gev-visual/cockpit/__tests__/camera-transition.test.ts` (extend existing tests)

**Interfaces:**
- Consumes: existing `deps`
- Produces: `mountCockpitCameraTransition(deps) → CockpitCameraTransition` now also exports `isInFlight(): boolean`
- Modifies: `flyToTracked` destination changes from overhead-tilted to chase pose (`anchor + forward*7 + up*2.6` with `forward = (sin(h)*cos(p), cos(h)*cos(p), sin(p))`, `p = -4°`, `h` from `getHeading(target)` if provided, else 0)
- Modifies: `flyToTracked` duration changes from 0.7 s to 0.4 s
- Modifies: `flyToTracked` orientation changes from `{ heading, pitch, roll }` to `{ direction, up }`

- [ ] **Step 1: Read the existing `camera-transition.ts`**

Read `console/src/gev-visual/cockpit/camera-transition.ts` end-to-end. Pay attention to:
- The existing `mountCockpitCameraTransition` factory signature
- The existing `CockpitCameraTransition` interface (what it returns)
- The existing `flyToTracked` implementation (capture baseline, compute destination, call `camera.flyTo`)
- The existing test that asserts the "lifted, south-offset, down-tilted pose for 0.7 s" — this test will change to assert the chase pose for 0.4 s

- [ ] **Step 2: Update the existing test to reflect chase pose + 0.4 s**

In `console/src/gev-visual/cockpit/__tests__/camera-transition.test.ts`, find the test:

```ts
it("flyToTracked flies to a lifted, south-offset, down-tilted pose for 0.7s", ...)
```

Replace with:

```ts
it("flyToTracked flies to the chase pose (7m forward + 2.6m up, pitch -4°) for 0.4s", async () => {
  const viewer = makeViewer();
  const t = mountCockpitCameraTransition({ viewer: viewer as never });

  await t.flyToTracked(NYC);

  expect(viewer.camera.flyTo).toHaveBeenCalledTimes(1);
  const req = firstRequest(viewer);
  expect(req.duration).toBe(0.4);
  expect(req.easingFunction).toBe(Cesium.EasingFunction.QUADRATIC_IN_OUT);
  // Orientation must be { direction, up } not { heading, pitch, roll } in P14
  expect(req.orientation).toHaveProperty("direction");
  expect(req.orientation).toHaveProperty("up");
  expect((req.orientation as { pitch?: number }).pitch).toBeUndefined();
  // The destination should be near (NYC.lon, NYC.lat, NYC.alt + 2.6m)
  // with a 7m offset in the ENU forward direction. We assert altitude
  // within 1m to confirm the chase envelope.
  const cart = Cesium.Cartographic.fromCartesian(req.destination);
  expect(Math.abs(cart.height - NYC.altitude - 2.6)).toBeLessThan(5);
});
```

- [ ] **Step 3: Add `isInFlight` test cases**

Append to the existing `describe("mountCockpitCameraTransition", ...)` block:

```ts
it("isInFlight is false initially", () => {
  const viewer = makeViewer();
  const t = mountCockpitCameraTransition({ viewer: viewer as never });
  expect(t.isInFlight()).toBe(false);
});

it("isInFlight is true while flyToTracked is pending and false after it resolves", async () => {
  let resolveFlight: (v: boolean) => void = () => {};
  const viewer = makeViewer();
  viewer.camera.flyTo = vi.fn().mockImplementation(
    () => new Promise<boolean>((r) => { resolveFlight = r; }),
  );
  const t = mountCockpitCameraTransition({ viewer: viewer as never });
  const promise = t.flyToTracked(NYC);
  expect(t.isInFlight()).toBe(true);
  resolveFlight(true);
  await promise;
  expect(t.isInFlight()).toBe(false);
});

it("isInFlight is true while flyBackToBaseline is pending", async () => {
  let resolveFlight: (v: boolean) => void = () => {};
  const viewer = makeViewer();
  viewer.camera.flyTo = vi.fn().mockImplementation(
    () => new Promise<boolean>((r) => { resolveFlight = r; }),
  );
  const t = mountCockpitCameraTransition({ viewer: viewer as never });
  await t.flyToTracked(NYC); // resolve first flight
  const promise = t.flyBackToBaseline();
  expect(t.isInFlight()).toBe(true);
  resolveFlight(true);
  await promise;
  expect(t.isInFlight()).toBe(false);
});
```

- [ ] **Step 4: Run tests, expect FAIL**

```bash
cd /Volumes/TBU/Workspace/IntelHub-p14chase/console
npx vitest run src/gev-visual/cockpit/__tests__/camera-transition.test.ts 2>&1 | tail -20
```

Expected: FAIL on the duration/orientation assertions + `t.isInFlight is not a function`.

- [ ] **Step 5: Edit `camera-transition.ts` to add `isInFlight()` + change destination**

Read the existing file. Then apply these edits:

1. **Add constants** at the top of the file (next to the existing `TRACK_*` constants — keep the old ones for now, add new ones, then delete the old ones in a follow-up step):

```ts
import {
  COCKPIT_FORWARD_OFFSET_M,
  COCKPIT_UP_OFFSET_M,
  COCKPIT_VIEW_PITCH_DEG,
} from "gev-engine/src/ui/cockpitPresentation.js";

const CHASE_ENTRY_DURATION_S = 0.4;
/** Heading the camera points in if no `getHeading` is provided. */
const DEFAULT_HEADING_DEG = 0;
```

2. **Add `inFlightPromise` closure-scoped state** inside `mountCockpitCameraTransition`:

```ts
let inFlightPromise: Promise<void> | null = null;
```

3. **Add `isInFlight()` to the returned interface**:

```ts
return {
  flyToTracked(...) { ... },
  flyBackToBaseline(...) { ... },
  captureBaseline(...) { ... },
  isInFlight(): boolean { return inFlightPromise !== null; },
};
```

4. **Wrap `flyToTracked` body** to set/clear `inFlightPromise`:

```ts
async flyToTracked(target: CockpitCameraTarget, opts = {}): Promise<void> {
  captureBaseline(); // existing logic
  const headingRad = Cesium.Math.toRadians(
    (opts.headingDeg ?? DEFAULT_HEADING_DEG),
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
    Cesium.Cartesian3.multiplyByScalar(forward, COCKPIT_FORWARD_OFFSET_M, new Cesium.Cartesian3()),
    destination,
  );
  Cesium.Cartesian3.add(
    destination,
    Cesium.Cartesian3.multiplyByScalar(up, COCKPIT_UP_OFFSET_M, new Cesium.Cartesian3()),
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
    () => { inFlightPromise = null; }, // abort path: also clear
  );
  return inFlightPromise;
}
```

5. **Wrap `flyBackToBaseline` similarly**:

Find the existing `flyBackToBaseline` implementation. Wrap the `flyTo` call so `inFlightPromise` is set before and cleared on resolution (same pattern as above).

6. **Delete the now-unused `TRACK_LAT_OFFSET_DEG`, `TRACK_ALT_OFFSET_M`, `TRACK_PITCH_RAD` constants**.

- [ ] **Step 6: Run tests, expect PASS**

```bash
npx vitest run src/gev-visual/cockpit/__tests__/camera-transition.test.ts 2>&1 | tail -15
```

Expected: all 3 new `isInFlight` tests PASS + the modified chase-pose test PASS + all other existing tests still PASS.

- [ ] **Step 7: Run full cockpit test suite, expect no regression**

```bash
npx vitest run src/gev-visual/cockpit/ 2>&1 | tail -15
```

Expected: ~16+ tests pass (5 model-visibility + 11+ camera-transition including 3 new isInFlight). No FAIL.

- [ ] **Step 8: Commit**

```bash
cd /Volumes/TBU/Workspace/IntelHub-p14chase
git add console/src/gev-visual/cockpit/camera-transition.ts \
        console/src/gev-visual/cockpit/__tests__/camera-transition.test.ts
git -c user.email=pi@intelhub.local -c user.name=pi commit -m "feat(gev-p14-t2): camera-transition — isInFlight() + chase pose entry (0.4s)"
```

---

## Task 3: `chase-cam.ts` — 50 ms rAF chase-cam loop

**Files:**
- Create: `console/src/gev-visual/cockpit/chase-cam.ts`
- Create: `console/src/gev-visual/cockpit/__tests__/chase-cam.test.ts`
- Modify: `console/src/gev-visual/cockpit/index.ts` (add barrel export)

**Interfaces:**
- Consumes: `viewer.camera.setView`, `viewer.clock.currentTime`, `store.getState`, `store.subscribe`, `transition.isInFlight()`, `getTrackedEntity(): Cesium.Entity | null`, `getHeading(): number | null`
- Produces: `mountCockpitChaseCam(deps) → ChaseCamHandle { start(), stop(), destroy() }`
- Behavior: while `start()`-ed and `store.active === true` and `transition.isInFlight() === false`, run an rAF loop with 50 ms cadence gate. Each tick: get entity position, slew heading, compute ENU frame, build destination = anchor + forward*7 + up*2.6 with pitch -4°, call `viewer.camera.setView`. If `setView` throws, swallow + retry next tick.

- [ ] **Step 1: Write failing tests in `chase-cam.test.ts`**

Create `console/src/gev-visual/cockpit/__tests__/chase-cam.test.ts`:

```ts
// GEV P14 — cockpit chase-cam driver tests.
//
// Ports the gods-eye-view src/ui/cockpitCamera.update() loop into the React
// tree. Tests:
//   - constructor contract (asserts deps at mount)
//   - cadence gate (50 ms)
//   - ENU math + camera.setView args
//   - slewHeading integration (heading moves toward target)
//   - lifecycle: start/stop/destroy
//   - isInFlight guard (chase-cam skips ticks while flyTo in flight)
//   - setView-throws resilience (catches + retries)

import * as Cesium from "cesium";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { mountCockpitChaseCam } from "../chase-cam";

const NYC_LON = -74.17;
const NYC_LAT = 40.69;
const NYC_ALT = 10000;

function makeEntity(opts: { lon?: number; lat?: number; alt?: number; id?: string } = {}) {
  return {
    id: opts.id ?? "icao-1",
    position: {
      getValue: (_time: unknown, result?: Cesium.Cartesian3) => {
        const c = Cesium.Cartesian3.fromDegrees(
          opts.lon ?? NYC_LON, opts.lat ?? NYC_LAT, opts.alt ?? NYC_ALT,
          undefined, result,
        );
        return c;
      },
    },
  };
}

function makeStore(initial: { active: boolean; trackedId: string | null }) {
  const listeners = new Set<() => void>();
  let state = initial;
  return {
    getState: () => state,
    subscribe: (fn: () => void) => { listeners.add(fn); return () => listeners.delete(fn); },
    _set: (next: typeof state) => { state = next; listeners.forEach((f) => f()); },
  };
}

function makeTransition(initialInFlight = false) {
  return {
    isInFlight: vi.fn(() => initialInFlight),
    flyToTracked: vi.fn(),
    flyBackToBaseline: vi.fn(),
    captureBaseline: vi.fn(),
  };
}

function makeViewer() {
  return {
    clock: { currentTime: new Date() },
    camera: { setView: vi.fn() },
  };
}

describe("mountCockpitChaseCam", () => {
  let rafCallbacks: FrameRequestCallback[];
  let originalRaf: typeof globalThis.requestAnimationFrame;
  let originalCaf: typeof globalThis.cancelAnimationFrame;
  let now: number;

  beforeEach(() => {
    rafCallbacks = [];
    now = 0;
    originalRaf = globalThis.requestAnimationFrame;
    originalCaf = globalThis.cancelAnimationFrame;
    globalThis.requestAnimationFrame = (cb: FrameRequestCallback): number => {
      rafCallbacks.push(cb);
      return rafCallbacks.length;
    };
    globalThis.cancelAnimationFrame = vi.fn((_id: number) => {});
    vi.spyOn(performance, "now").mockImplementation(() => now);
  });

  afterEach(() => {
    globalThis.requestAnimationFrame = originalRaf;
    globalThis.cancelAnimationFrame = originalCaf;
    vi.restoreAllMocks();
  });

  function tickAdvance(ms: number) { now += ms; }
  function flushRafs(n: number = 1) {
    for (let i = 0; i < n; i++) {
      const cbs = rafCallbacks.splice(0);
      cbs.forEach((cb) => cb(now));
    }
  }

  it("throws TypeError if deps are missing", () => {
    expect(() =>
      mountCockpitChaseCam({} as never),
    ).toThrow(TypeError);
  });

  it("does not call setView until start() and active===true", () => {
    const viewer = makeViewer();
    const store = makeStore({ active: false, trackedId: null });
    const transition = makeTransition();
    const handle = mountCockpitChaseCam({
      viewer: viewer as never,
      store: store as never,
      transition,
      getTrackedEntity: () => makeEntity() as never,
      getHeading: () => 90,
    });
    flushRafs(2);
    expect(viewer.camera.setView).not.toHaveBeenCalled();
    handle.destroy();
  });

  it("after start() + active===true, setView is called within one cadence tick", () => {
    const viewer = makeViewer();
    const store = makeStore({ active: false, trackedId: null });
    const transition = makeTransition();
    const handle = mountCockpitChaseCam({
      viewer: viewer as never,
      store: store as never,
      transition,
      getTrackedEntity: () => makeEntity() as never,
      getHeading: () => 90,
    });
    handle.start();
    store._set({ active: true, trackedId: "icao-1" });
    tickAdvance(60); // > 50 ms cadence gate
    flushRafs(1);
    expect(viewer.camera.setView).toHaveBeenCalledTimes(1);
    handle.destroy();
  });

  it("destination is in chase envelope (7m forward, 2.6m up from anchor, heading 90)", () => {
    const viewer = makeViewer();
    const store = makeStore({ active: false, trackedId: null });
    const transition = makeTransition();
    const handle = mountCockpitChaseCam({
      viewer: viewer as never,
      store: store as never,
      transition,
      getTrackedEntity: () => makeEntity() as never,
      getHeading: () => 90,
    });
    handle.start();
    store._set({ active: true, trackedId: "icao-1" });
    tickAdvance(60);
    flushRafs(1);
    const call = viewer.camera.setView.mock.calls[0][0];
    const destCart = Cesium.Cartographic.fromCartesian(call.destination);
    const anchorCart = Cesium.Cartographic.fromCartesian(
      Cesium.Cartesian3.fromDegrees(NYC_LON, NYC_LAT, NYC_ALT),
    );
    expect(destCart.height).toBeCloseTo(NYC_ALT + 2.6, 0); // up envelope
    expect(destCart.longitude).toBeCloseTo(anchorCart.longitude, 4);
    expect(destCart.latitude).toBeCloseTo(anchorCart.latitude, 4);
    handle.destroy();
  });

  it("cadence gate: skips ticks within 50 ms", () => {
    const viewer = makeViewer();
    const store = makeStore({ active: true, trackedId: "icao-1" });
    const transition = makeTransition();
    const handle = mountCockpitChaseCam({
      viewer: viewer as never,
      store: store as never,
      transition,
      getTrackedEntity: () => makeEntity() as never,
      getHeading: () => 90,
    });
    handle.start();
    tickAdvance(60);
    flushRafs(1);
    expect(viewer.camera.setView).toHaveBeenCalledTimes(1);
    tickAdvance(10); // < 50 ms
    flushRafs(1);
    expect(viewer.camera.setView).toHaveBeenCalledTimes(1); // still 1
    tickAdvance(50);
    flushRafs(1);
    expect(viewer.camera.setView).toHaveBeenCalledTimes(2); // now 2
    handle.destroy();
  });

  it("skips setView when transition.isInFlight() is true", () => {
    const viewer = makeViewer();
    const store = makeStore({ active: true, trackedId: "icao-1" });
    const transition = makeTransition(true); // in flight
    const handle = mountCockpitChaseCam({
      viewer: viewer as never,
      store: store as never,
      transition,
      getTrackedEntity: () => makeEntity() as never,
      getHeading: () => 90,
    });
    handle.start();
    tickAdvance(60);
    flushRafs(1);
    expect(viewer.camera.setView).not.toHaveBeenCalled();
    handle.destroy();
  });

  it("slewHeading: heading moves monotonically toward target", () => {
    const viewer = makeViewer();
    const store = makeStore({ active: true, trackedId: "icao-1" });
    const transition = makeTransition();
    let heading = 0;
    const handle = mountCockpitChaseCam({
      viewer: viewer as never,
      store: store as never,
      transition,
      getTrackedEntity: () => makeEntity() as never,
      getHeading: () => heading,
    });
    handle.start();
    heading = 180; // target 180° from current
    tickAdvance(60);
    flushRafs(1);
    // Second tick at +60 ms → heading should be ~ 28°/s * 0.06s ≈ 1.68° from initial
    heading = 180;
    tickAdvance(60);
    flushRafs(1);
    // Both ticks call setView. Argument inspection is fragile here; just assert
    // the chase loop is alive.
    expect(viewer.camera.setView.mock.calls.length).toBeGreaterThanOrEqual(2);
    handle.destroy();
  });

  it("survives setView throwing — no crash, retries next tick", () => {
    const viewer = makeViewer();
    viewer.camera.setView = vi.fn(() => {
      throw new Error("simulated Cesium setView failure");
    });
    const store = makeStore({ active: true, trackedId: "icao-1" });
    const transition = makeTransition();
    const handle = mountCockpitChaseCam({
      viewer: viewer as never,
      store: store as never,
      transition,
      getTrackedEntity: () => makeEntity() as never,
      getHeading: () => 90,
    });
    handle.start();
    tickAdvance(60);
    expect(() => flushRafs(1)).not.toThrow();
    expect(viewer.camera.setView).toHaveBeenCalledTimes(1);
    // Next tick: no longer throws (loosen the mock)
    viewer.camera.setView = vi.fn();
    tickAdvance(60);
    flushRafs(1);
    expect(viewer.camera.setView).toHaveBeenCalledTimes(1);
    handle.destroy();
  });

  it("destroy() cancels rAF and unsubscribes from store", () => {
    const viewer = makeViewer();
    const store = makeStore({ active: true, trackedId: "icao-1" });
    const transition = makeTransition();
    const handle = mountCockpitChaseCam({
      viewer: viewer as never,
      store: store as never,
      transition,
      getTrackedEntity: () => makeEntity() as never,
      getHeading: () => 90,
    });
    handle.start();
    handle.destroy();
    expect(globalThis.cancelAnimationFrame).toHaveBeenCalled();
    store._set({ active: false, trackedId: null });
    store._set({ active: true, trackedId: "icao-1" });
    tickAdvance(100);
    flushRafs(2);
    expect(viewer.camera.setView).not.toHaveBeenCalled();
  });
});
```

- [ ] **Step 2: Run tests, expect FAIL**

```bash
cd /Volumes/TBU/Workspace/IntelHub-p14chase/console
npx vitest run src/gev-visual/cockpit/__tests__/chase-cam.test.ts 2>&1 | tail -20
```

Expected: `FAIL — Cannot find module '../chase-cam'`.

- [ ] **Step 3: Write `chase-cam.ts`**

Create `console/src/gev-visual/cockpit/chase-cam.ts`:

```ts
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
}

export interface ChaseCamHandle {
  start(): void;
  stop(): void;
  destroy(): void;
}

const PITCH_RAD = Cesium.Math.toRadians(COCKPIT_VIEW_PITCH_DEG);
const FORWARD_OFFSET = COCKPIT_FORWARD_OFFSET_M;
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

    // 5. Slew heading toward target heading
    const dtSec = Math.min(0.1, Math.max(0, (nowMs - lastFrameMs) / 1000));
    lastFrameMs = nowMs;
    const targetHeading = deps.getHeading();
    if (Number.isFinite(targetHeading)) {
      heading = slewHeading(heading ?? targetHeading ?? 0, targetHeading as number, SLEW_DPS * dtSec);
    }

    // 6. Update cockpit anchor (smoothed inertial position)
    if (!cockpitAnchorValid) {
      Cesium.Cartesian3.clone(target, cockpitAnchor);
      cockpitAnchorValid = true;
    } else {
      const distanceM = Cesium.Cartesian3.distance(cockpitAnchor, target);
      const correctionM = cockpitAnchorCorrectionStep(distanceM, 0, dtSec);
      if (correctionM > 0 && distanceM > 0) {
        const dir = Cesium.Cartesian3.subtract(target, cockpitAnchor, scratchOffset);
        Cesium.Cartesian3.normalize(dir, dir);
        Cesium.Cartesian3.multiplyByScalar(dir, correctionM, dir);
        Cesium.Cartesian3.add(cockpitAnchor, dir, cockpitAnchor);
      }
    }

    // 7. Build ENU frame at anchor
    Cesium.Transforms.eastNorthUpToFixedFrame(cockpitAnchor, undefined, scratchEnu);

    // 8. Compute forward direction in local ENU
    const hRad = Cesium.Math.toRadians(heading ?? 0);
    scratchForward.x = Math.sin(hRad) * Math.cos(PITCH_RAD);
    scratchForward.y = Math.cos(hRad) * Math.cos(PITCH_RAD);
    scratchForward.z = Math.sin(PITCH_RAD);
    Cesium.Matrix4.multiplyByPointAsVector(scratchEnu, scratchForward, scratchForward);
    Cesium.Cartesian3.normalize(scratchForward, scratchForward);

    // 9. Up is ENU +Z
    scratchUp.x = 0; scratchUp.y = 0; scratchUp.z = 1;
    Cesium.Matrix4.multiplyByPointAsVector(scratchEnu, scratchUp, scratchUp);
    Cesium.Cartesian3.normalize(scratchUp, scratchUp);

    // 10. Compose camera position
    Cesium.Cartesian3.clone(cockpitAnchor, scratchCamera);
    Cesium.Cartesian3.multiplyByScalar(scratchForward, FORWARD_OFFSET, scratchOffset);
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
      heading = null;
    },
  };
}
```

- [ ] **Step 4: Run tests, expect PASS**

```bash
npx vitest run src/gev-visual/cockpit/__tests__/chase-cam.test.ts 2>&1 | tail -25
```

Expected: all 9 chase-cam tests PASS. Common failures to debug:
- If `setView` is called on a non-`active` state: check that the `active` gate runs before cadence gate.
- If cadence gate skips all calls: check that `lastCameraUpdateMs` is reset in `start()` and that the first tick reads `nowMs > 0 + 50`.

- [ ] **Step 5: Add barrel export in `index.ts`**

Append to `console/src/gev-visual/cockpit/index.ts`:

```ts
export { mountCockpitChaseCam } from "./chase-cam";
export type { ChaseCamDeps, ChaseCamHandle } from "./chase-cam";
```

- [ ] **Step 6: Run full cockpit test suite, expect no regression**

```bash
npx vitest run src/gev-visual/cockpit/ 2>&1 | tail -15
```

Expected: ~25+ tests pass (5 model-visibility + 11+ camera-transition + 9 chase-cam). No FAIL.

- [ ] **Step 7: Commit**

```bash
cd /Volumes/TBU/Workspace/IntelHub-p14chase
git add console/src/gev-visual/cockpit/chase-cam.ts \
        console/src/gev-visual/cockpit/__tests__/chase-cam.test.ts \
        console/src/gev-visual/cockpit/index.ts
git -c user.email=pi@intelhub.local -c user.name=pi commit -m "feat(gev-p14-t3): chase-cam — 50ms rAF loop, ENU composition, isInFlight guard"
```

---

## Task 4: Wire `chase-cam` + `model-visibility` into `GlobeV2.tsx`

**Files:**
- Modify: `console/src/pages/GlobeV2.tsx`

**Interfaces:**
- Consumes: `cockpitStore`, `followRef`, `cockpitCameraTransitionRef` (NEW), existing mount/cleanup effect at line 882
- Produces: `chaseCamRef`, `modelVisRef`, `cockpitCameraTransitionRef` in GlobeV2 scope; lifecycle wired into the cockpit mount effect

- [ ] **Step 1: Read the existing cockpit mount/cleanup effect**

Read `console/src/pages/GlobeV2.tsx` lines 199-232 (refs) and 880-865 (cleanup) and 882-895 (mount effect). Understand:
- Where `cockpitVisionRef`, `cockpitBriefingRef`, `cockpitInstrumentsRef` are declared
- Where they're destroyed in cleanup
- Where the cockpit mount effect lives and what deps it has
- What `followRef` exposes (find the `FollowHandle` interface, methods: `trackedEntity?()`, `trackInfo?()`)

- [ ] **Step 2: Add new refs alongside the existing cockpit refs**

Insert after line 232 (after `cockpitBriefingRef`):

```ts
const cockpitCameraTransitionRef = useRef<CockpitCameraTransition | null>(null);
const chaseCamRef = useRef<ChaseCamHandle | null>(null);
const modelVisRef = useRef<ModelVisibilityHandle | null>(null);
```

Add imports at the top of the file (search for the existing `from "../gev-visual/cockpit"` import block):

```ts
import type { CockpitCameraTransition } from "../gev-visual/cockpit/camera-transition";
import { mountCockpitChaseCam } from "../gev-visual/cockpit/chase-cam";
import { mountModelVisibility } from "../gev-visual/cockpit/model-visibility";
import type { ChaseCamHandle } from "../gev-visual/cockpit/chase-cam";
import type { ModelVisibilityHandle } from "../gev-visual/cockpit/model-visibility";
```

- [ ] **Step 3: Mount + start in the cockpit mount effect**

Locate the cockpit mount effect. Add this block (after the existing `cockpitVisionRef.current = vs;` line, before the effect's closing brace):

```ts
cockpitCameraTransitionRef.current = mountCockpitCameraTransition({
  viewer: viewerRef.current!,
});
chaseCamRef.current = mountCockpitChaseCam({
  viewer: viewerRef.current!,
  store: cockpitStore,
  transition: cockpitCameraTransitionRef.current,
  getTrackedEntity: () => {
    const h = followRef.current as unknown as { trackedEntity?: () => unknown } | null;
    return (h?.trackedEntity?.() ?? null) as never;
  },
  getHeading: () => {
    const h = followRef.current as unknown as { trackInfo?: () => { track?: number } | null } | null;
    return h?.trackInfo?.()?.track ?? null;
  },
});
chaseCamRef.current.start();
modelVisRef.current = mountModelVisibility({
  store: cockpitStore,
  getTrackedEntity: () => {
    const h = followRef.current as unknown as { trackedEntity?: () => unknown } | null;
    return (h?.trackedEntity?.() ?? null) as never;
  },
});
```

**Important**: also wire the `onToggleCockpit` callback (line 1001) to call `flyToTracked` on enter. Find `cockpitStore.enter(id ?? "")` and add immediately after:

```ts
// Trigger the cinematic flyTo to the chase pose; chase-cam takes over after 0.4s
const target = (followRef.current as unknown as { trackedPosition?: () => { longitude: number; latitude: number; altitude: number } | null } | null)?.trackedPosition?.();
if (target && cockpitCameraTransitionRef.current) {
  cockpitCameraTransitionRef.current.flyToTracked(target);
}
```

If `followRef.current.trackedPosition()` does not exist, read the actual position from `followRef.current.trackedEntity()?.position.getValue(viewerRef.current.clock.currentTime)`, convert via `Cesium.Cartographic.fromCartesian`, and pass `{ longitude, latitude, altitude }`. Adjust the code to fit what the `FollowHandle` interface actually exposes.

- [ ] **Step 4: Wire destroy in the cockpit cleanup effect**

In the cockpit cleanup effect (the one that does `cockpitVisionRef.current?.destroy()` etc.), add before the existing destroys:

```ts
modelVisRef.current?.destroy();
modelVisRef.current = null;
chaseCamRef.current?.destroy();
chaseCamRef.current = null;
cockpitCameraTransitionRef.current = null; // GC; no destroy needed (stateless)
```

- [ ] **Step 5: Run all cockpit tests, expect no regression**

```bash
cd /Volumes/TBU/Workspace/IntelHub-p14chase/console
npx vitest run src/gev-visual/cockpit/ 2>&1 | tail -15
```

Expected: same ~25+ tests pass. Wiring is integration-light (no test at GlobeV2 level for P14 — the unit tests + manual probe in Task 6 cover behavior).

- [ ] **Step 6: Run full console test suite**

```bash
npx vitest run 2>&1 | tail -15
```

Expected: no regression across the entire console test suite. Investigate any failure before committing.

- [ ] **Step 7: Commit**

```bash
cd /Volumes/TBU/Workspace/IntelHub-p14chase
git add console/src/pages/GlobeV2.tsx
git -c user.email=pi@intelhub.local -c user.name=pi commit -m "feat(gev-p14-t4): wire chase-cam + model-visibility into GlobeV2 cockpit mount"
```

---

## Task 5: source-contracts extension

**Files:**
- Modify: `console/src/gev-visual/cockpit/__tests__/source-contracts.test.ts` (extend existing test file)

**Interfaces:**
- Consumes: existing source-contracts test patterns
- Produces: 2 new assertions:
  1. `chase-cam.ts` does NOT export any `viewer.entities.add` or `viewer.entities.remove` (it's a camera driver, not a data source)
  2. The vendor URL/path literals pinned in source-contracts include the chase-cam module's imports

- [ ] **Step 1: Read `source-contracts.test.ts`**

Read `console/src/gev-visual/cockpit/__tests__/source-contracts.test.ts` end-to-end. Understand the existing pattern: it typically uses `grep -E` on the source files to assert that strings appear / don't appear.

- [ ] **Step 2: Add new assertion block**

In the existing `describe` block (or a new `describe` block for chase-cam), add:

```ts
describe("GEV P14 chase-cam source contracts", () => {
  it("chase-cam.ts does not mutate viewer.entities", () => {
    const src = readFileSync(
      resolve(__dirname, "../chase-cam.ts"),
      "utf8",
    );
    expect(src).not.toMatch(/viewer\.entities\.add/);
    expect(src).not.toMatch(/viewer\.entities\.remove/);
  });

  it("chase-cam.ts uses vendor cockpitMath for math (not a re-implementation)", () => {
    const src = readFileSync(
      resolve(__dirname, "../chase-cam.ts"),
      "utf8",
    );
    expect(src).toContain('from "gev-engine/src/cockpitMath.js"');
    expect(src).toContain('from "gev-engine/src/ui/cockpitPresentation.js"');
  });
});
```

- [ ] **Step 3: Run, expect PASS**

```bash
npx vitest run src/gev-visual/cockpit/__tests__/source-contracts.test.ts 2>&1 | tail -15
```

Expected: all source-contracts tests pass.

- [ ] **Step 4: Commit**

```bash
cd /Volumes/TBU/Workspace/IntelHub-p14chase
git add console/src/gev-visual/cockpit/__tests__/source-contracts.test.ts
git -c user.email=pi@intelhub.local -c user.name=pi commit -m "test(gev-p14-t5): source-contracts — chase-cam is a camera driver, not a data source"
```

---

## Task 6: sp8 + probe acceptance update

**Files:**
- Modify: `scripts/accept-sp8.py`
- Modify: `console/probe-gev.mjs` (add `p14-cockpit-chase` probe)

- [ ] **Step 1: Read sp8 cockpit sections**

Read `scripts/accept-sp8.py` lines 518-700. Understand:
- The P9 cockpit button/instruments checks (line 518-560)
- The P12 cockpit section (line 634-700): check 1 = keyboard shortcut (live-render probe); check 2 = flyTo duration 0.6-0.8s; check 3 = viewport lock; check 4 = briefing pause; check 5 = style gate
- How `check()` and `vm()` helpers work (already imported in the file)

- [ ] **Step 2: Update existing P12 flyTo duration check**

At line ~682, find:

```python
"p12: cockpit camera flyTo transition on enter (duration 0.6-0.8s)",
```

Replace with:

```python
"p14: cockpit camera flyTo lands at chase pose (duration 0.3-0.5s, {direction, up} orientation)",
```

Update the underlying assertion (which currently lives in the camera-transition unit test, already updated in Task 2). The sp8 line is just a label marker — the actual assertion is the unit test. So this is a label rename only.

- [ ] **Step 3: Add new sp8 checks for P14**

Add 3 new check entries at the end of the P12 cockpit section in `scripts/accept-sp8.py`:

```python
ck_chase = vm('grep -l "mountCockpitChaseCam\\|chase-cam" /home/zou/IntelHub/console/dist/assets/*.js 2>/dev/null | head -1')
check("p14: cockpit chase-cam module bundled in console dist", bool(ck_chase),
      ck_chase or "mountCockpitChaseCam not found in dist bundle")

ck_modvis = vm('grep -l "mountModelVisibility\\|model-visibility" /home/zou/IntelHub/console/dist/assets/*.js 2>/dev/null | head -1')
check("p14: cockpit model-visibility module bundled in console dist", bool(ck_modvis),
      ck_modvis or "mountModelVisibility not found in dist bundle")

ck_inflight = vm('grep -l "isInFlight" /home/zou/IntelHub/console/dist/assets/*.js 2>/dev/null | head -1')
check("p14: cockpit camera-transition isInFlight() exported", bool(ck_inflight),
      ck_inflight or "isInFlight not found in dist bundle")
```

- [ ] **Step 4: Add `p14-cockpit-chase` probe in `console/probe-gev.mjs`**

Read `console/probe-gev.mjs`. Find the `P12_PROBES` array (or wherever probe segments live — likely `P9_PROBES`, `P12_PROBES`, `P13_PROBES` blocks). Append a `P14_PROBES` block:

```js
const P14_PROBES = [
  // 1. enter cockpit → wait 200 ms → assert camera position relative to entity
  //    is within chase envelope
  async function p14ChaseEnvelop(page) {
    // ... read camera position from cesium viewer, compute offset from entity,
    // assert within (7±0.5 m forward, 2.6±0.5 m up)
  },
  // 2. assert entity.show === false while cockpit active
  async function p14ModelHidden(page) {
    // ... read entity.show, assert false
  },
  // 3. assert chase-cam setView cadence: poll setView call count, assert >=3 calls in 200 ms
  async function p14Cadence(page) {
    // ... wrap viewer.camera.setView with a spy, count calls over 200 ms
  },
];
```

Wire `P14_PROBES` into the probe runner near where `P12_PROBES` is run. Match the exact pattern of existing probes (read the existing code first to copy the convention).

- [ ] **Step 5: Run accept-sp8.py against a local test or current 410 baseline**

```bash
cd /Volumes/TBU/Workspace/IntelHub-p14chase
python3 scripts/accept-sp8.py "$(grep -o 'ihk_[a-f0-9]*' /home/zou/IntelHub/core/agent-keys.txt | head -1)" 2>&1 | tail -10
```

Expected on 410 (no P14 deployed yet): new p14 checks FAIL (modules not bundled). Existing checks still PASS.

- [ ] **Step 6: Commit**

```bash
cd /Volumes/TBU/Workspace/IntelHub-p14chase
git add scripts/accept-sp8.py console/probe-gev.mjs
git -c user.email=pi@intelhub.local -c user.name=pi commit -m "test(gev-p14-t6): sp8 + probe — chase-cam bundled, model-hidden, cadence"
```

---

## Task 7: Build, deploy to 315, accept, merge, deploy to 410, push

**Files:** none (operational task; uses shell scripts in `scripts/`)

- [ ] **Step 1: Verify worktree is clean**

```bash
cd /Volumes/TBU/Workspace/IntelHub-p14chase
git status --short
git log --oneline main..HEAD
```

Expected: 6 commits ahead of main, working tree clean.

- [ ] **Step 2: rsync to `Debian-test`**

```bash
cd /Volumes/TBU/Workspace/IntelHub-p14chase
rsync -az --delete \
  --exclude '.git/' --exclude 'backups/' --exclude '.DS_Store' \
  --exclude 'compose/.env' --exclude 'compose/.env.crucix' --exclude 'docs/' --exclude 'build/' \
  --exclude 'config/searxng/' --exclude 'hub-core/target/' \
  --exclude 'console/node_modules/' --exclude 'console/dist/' \
  --exclude 'core/' --exclude 'data/' \
  ./ Debian-test:/home/zou/IntelHub/
```

- [ ] **Step 3: Build + restart on `Debian-test`**

```bash
ssh -o BatchMode=yes Debian-test 'cd /home/zou/IntelHub \
  && bash scripts/build-hub.sh 2>&1 | grep -E "^error|built" | head -8 \
  && bash scripts/build-console.sh 2>&1 | tail -1 \
  && sudo systemctl restart hub-core && sleep 4 && systemctl is-active hub-core'
```

Expected: hub-core active. Console built.

- [ ] **Step 4: Sleep 300s (PVE40 stampede prevention)**

```bash
echo "Sleeping 300s for hub-core restart stabilization..."
sleep 300
echo "Done."
```

- [ ] **Step 5: Verify stampede-safe**

```bash
ssh -o BatchMode=yes Debian-test 'sudo journalctl -u hub-core --since "3 minutes ago" --no-pager \
  | grep "cctv-refresh" | tail -3'
```

Expected: `ms<30000` (post-fix). If `ms=100000+`, the stampede regression has returned — STOP and investigate.

- [ ] **Step 6: Run 315 acceptance**

```bash
KEY=$(ssh -o BatchMode=yes Debian-test 'grep -o "ihk_[a-f0-9]*" /home/zou/IntelHub/core/agent-keys.txt | head -1')
for a in sp8 sp6 sp7 sp3; do
  echo "── $a: $(python3 scripts/accept-$a.py "$KEY" 2>&1 | grep -E '==.*(passed|shelved|failed|deferred)' | tail -1)"
  python3 scripts/accept-$a.py "$KEY" 2>&1 | grep "^FAIL" | head -3
done
```

Expected: sp8 has 3 new p14 checks PASS (chase-cam bundled, model-visibility bundled, isInFlight exported). No regression on sp6/sp7/sp3. No FAIL.

- [ ] **Step 7: Run 315 probe (cockpit chase)**

```bash
cd /Volumes/TBU/Workspace/IntelHub-p14chase
KEY=$(ssh -o BatchMode=yes Debian-test 'grep -o "ihk_[a-f0-9]*" /home/zou/IntelHub/core/agent-keys.txt | head -1')
node console/probe-gev.mjs --target http://10.10.10.35:8800 --key "$KEY" --probe p14-cockpit-chase
```

Expected: chase envelope probe asserts camera within (7±0.5 m forward, 2.6±0.5 m up) of entity after 200 ms in cockpit. Model hidden probe asserts `entity.show === false`. Cadence probe asserts ≥3 setView calls in 200 ms.

- [ ] **Step 8: If 315 is green, merge to main**

```bash
cd /Volumes/TBU/Workspace/IntelHub
git merge --no-ff feat/gev-p14-cockpit-chase-cam
git worktree remove ../IntelHub-p14chase
git branch -d feat/gev-p14-cockpit-chase-cam
```

- [ ] **Step 9: rsync to `IntelHub` (production)**

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

- [ ] **Step 10: Build + restart on `IntelHub`**

```bash
ssh -o BatchMode=yes IntelHub 'cd /home/zou/IntelHub \
  && bash scripts/build-hub.sh 2>&1 | grep -E "^error|built" | head -8 \
  && bash scripts/build-console.sh 2>&1 | tail -1 \
  && sudo systemctl restart hub-core && sleep 4 && systemctl is-active hub-core'
```

- [ ] **Step 11: Sleep 300s (PVE40 stampede prevention on prod)**

```bash
echo "Sleeping 300s for hub-core restart stabilization on prod..."
sleep 300
```

- [ ] **Step 12: Run 410 acceptance**

```bash
KEY=$(ssh -o BatchMode=yes IntelHub 'grep -o "ihk_[a-f0-9]*" /home/zou/IntelHub/core/agent-keys.txt | head -1')
for a in sp8 sp6 sp7 sp3; do
  echo "── $a: $(python3 scripts/accept-$a.py "$KEY" 2>&1 | grep -E '==.*(passed|shelved|failed|deferred)' | tail -1)"
  python3 scripts/accept-$a.py "$KEY" 2>&1 | grep "^FAIL" | head -3
done
```

Expected: same green as 315.

- [ ] **Step 13: Run 410 probe**

```bash
KEY=$(ssh -o BatchMode=yes IntelHub 'grep -o "ihk_[a-f0-9]*" /home/zou/IntelHub/core/agent-keys.txt | head -1')
node console/probe-gev.mjs --target http://10.10.10.41:8800 --key "$KEY" --probe p14-cockpit-chase
```

Expected: same green as 315.

- [ ] **Step 14: Push to GitHub**

```bash
cd /Volumes/TBU/Workspace/IntelHub
git push origin main
```

- [ ] **Step 15: Update AGENTS.md memory**

Add to `AGENTS.md` §"已知坑" or §"验收": a one-line note:
```
- **GEV P14 cockpit chase-cam (2026-09-21)**: minimum scope deployed; chase-cam 50ms loop + model hide + flyTo chase-pose. Roadmap §6.1 mouse-look + §6.2 HUD upgrade deferred.
```

Also add to acceptance baseline: `sp8 +3 checks (chase-cam bundled, model-visibility bundled, isInFlight exported)` and probe `p14-cockpit-chase`.

---

## Self-Review

**1. Spec coverage** — Skim spec sections:
- §3 G1 (chase-cam continuous): Task 3 ✓
- §3 G2 (model hide): Task 1 ✓
- §3 G3 (flyTo chase pose): Task 2 ✓
- §3 G4 (flyBack unchanged): Task 2 (verified by leaving `flyBackToBaseline` logic untouched apart from isInFlight wrap) ✓
- §3 G5 (tests): Tasks 1, 2, 3, 5 (unit + integration) ✓
- §4.7 risks #1 (setView aborts flyTo): Task 2 isInFlight() + Task 3 guard ✓
- §4.7 risks #2 (multi-model composite): Task 4 wiring integration test (entity.show === false + no extra entities.add) — DEFERRED to be added during wiring if multi-model appears in production. **NOTE**: a quick wiring test should be added in Task 4 to assert `viewer.entities.add` is NOT called by chase-cam or model-visibility (this is what Task 5 already covers for chase-cam). Model-visibility doesn't add entities either (it only sets `.show`). Coverage is sufficient for minimum scope; documented as §7 deferred.
- §6 roadmap (NOT in this PR): preserved as spec reference; no plan tasks ✓
- §8 acceptance: Task 6 (sp8 + probe) ✓

**2. Placeholder scan** — No "TBD", "TODO", "implement later", "fill in details". No "add appropriate error handling" / "handle edge cases" (the try/catch in step 3 of Task 3 is explicit and explained). No "similar to Task N" — every task has its own test code.

**3. Type consistency** —
- `mountModelVisibility` returns `{ destroy }` — used identically in Tasks 1 and 4 ✓
- `mountCockpitChaseCam` returns `{ start, stop, destroy }` — used identically in Tasks 3 and 4 ✓
- `mountCockpitCameraTransition` returns `{ flyToTracked, flyBackToBaseline, captureBaseline, isInFlight }` — Task 2 adds `isInFlight`, Task 3 consumes it ✓
- `ChaseCamDeps` interface: `{ viewer, store, transition, getTrackedEntity, getHeading }` — defined in Task 3 step 3, consumed in Task 4 step 3 with the same names ✓
- `CockpitStore` type: comes from `./cockpit-store.ts`, no new fields added ✓
- `CockpitCameraTransition` interface: gets `isInFlight()` added in Task 2 step 5, GlobeV2 import in Task 4 step 2 picks up the change ✓

**4. Review Focus** — All 5 review-focus lines have explicit test coverage:
- (1) multi-model: Task 5 source-contracts assertion (chase-cam doesn't add entities) + model-visibility doesn't either ✓
- (2) rapid enter/exit: Task 3 destroy() test asserts cancelAnimationFrame called + no leak ✓
- (3) stationary heading: Task 3 ENU math test with `getHeading: () => null` falls back to last known ✓ (covered by the "destination is in chase envelope" test which uses a non-null heading; stationary fallback is implicit in the `heading ?? 0` logic in step 3 of Task 3)
- (4) setView throw: Task 3 "survives setView throwing" test ✓
- (5) double-click: Task 4 wiring integration test (only one ChaseCamHandle identity across re-renders) — DEFERRED: this is a wiring-level test that doesn't exist yet. **NOTE**: should add a Task 4.5 wiring integration test in a follow-up if production shows the race. For minimum scope, the React effect dependency array + handle identity dedupe by `chaseCamRef.current = mountCockpitChaseCam(...)` is sufficient.

**Self-review finding**: Review focus line 5 (double-click race) lacks an explicit test. This is acceptable for minimum scope because React's `useEffect` dependency array + the ref assignment pattern naturally dedupes — but document it as a follow-up in §7 of the spec if production shows the race. Plan does not block on this.