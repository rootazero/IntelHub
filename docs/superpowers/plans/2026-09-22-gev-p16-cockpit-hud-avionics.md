# GEV P16 — Cockpit HUD Avionics Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Render all 6 HUD avionics elements (heading tape / altitude ladder / speed tape / pitch ladder / bank indicator / VSI chevron) in the cockpit view, driven by chase-cam's resolved state + flight telemetry at 10 Hz.

**Architecture:** chase-cam extends with bank (Quaternion→roll) and VSI (8-sample altitude history window, 400ms). instruments-mount adds 3 frame fields (`pitchRad`, `bankRad`, `vsiMps`). A new `mountCockpitHudTick` owns the 10 Hz update timer. `HudCockpitInstruments.tsx` extends in place to render 5 new SVG groups (heading compass preserved).

**Tech Stack:** TypeScript, React 18, Cesium 1.x (HeadingPitchRoll.fromQuaternion), Vitest, jsdom + @testing-library/react.

**Spec:** `docs/superpowers/specs/2026-09-22-gev-p16-cockpit-hud-avionics-design.md` (commit `4e7bb70`)

## Global Constraints

- **No DB / No API / No env-var changes** — console bundle only (AGENTS.md GEV precedent).
- **5-min stampede wait** — `sudo systemctl restart hub-core` then `sleep 300` (AGENTS.md 2026-09-20 lesson).
- **315-test-first deploy** — Build + accept on `Debian-test` (10.10.10.35) before any `IntelHub` (10.10.10.41) change.
- **Brief-driven subagent execution** — subagent-driven-development skill; each task brief is self-contained (subagent has zero codebase context beyond the brief).
- **Vendor byte-stable math** — `cockpitMath.js` from P9 must remain byte-stable; do NOT modify it. New math (bank, VSI) goes in `chase-cam.ts` only.
- **Style gate (P9 R5)** — HUD renders unconditionally while cockpit is active; not affected by globe style picker.
- **10 Hz cadence** — HUD timer matches vendor `COCKPIT_HUD_UPDATE_MS = 100`. Do NOT throttle differently.
- **Pre-flight invariants** — chase-cam's `getResolvedState()` returns `null` when not initialized; HUD timer must check `viewer.isDestroyed()` per tick.

## Review Focus

These 5 failure modes are most likely to bite a person using the HUD. Each is pinned to a task's tests below.

1. **Quaternion is zero/null** → `HeadingPitchRoll.fromQuaternion` throws; bank should default to 0 with no crash. (Pinned to Task 1 tests.)
2. **VSI flickers wildly** → 4-sample window (200ms) is too jittery; the spec uses 8 samples (400ms). Tests must verify window saturates at length 8, not 4. (Pinned to Task 1 tests.)
3. **`entity.position.getValue` returns `undefined`** (entity has no position property) → existing chase-cam already handles; bank derivation must also handle. (Pinned to Task 1 tests.)
4. **HUD timer leaks if cleanup not called** → `start()`/`stop()`/`isRunning()` lifecycle; `viewer.isDestroyed()` must stop ticks. (Pinned to Task 4 tests.)
5. **Pitch ladder rotation flickers when `bankRad` is not clamped** → Cesium HeadingPitchRoll.roll can be ±π (180° bank), but the bank indicator arc is ±60°. Tests must verify bank stays in radians without producing NaN. (Pinned to Task 5 visual test.)

---

## Task 1: chase-cam — bank + VSI + altitude derivation

**Files:**
- Modify: `console/src/gev-visual/cockpit/chase-cam.ts` (P14/P15 file)
- Modify: `console/src/gev-visual/cockpit/__tests__/chase-cam.test.ts` (existing P14 test file)

**Interfaces:**
- Consumes: `Cesium.HeadingPitchRoll.fromQuaternion(quaternion)` (Cesium 1.x); `entity.orientation.getValue(time, result?)` (Cesium Entity API)
- Produces:
  - `ChaseCamResolvedState` interface (new export) — `{ heading: number|null, pitch: number|null, range: number|null, bankRad: number, vsiMps: number, altitudeM: number|null }`
  - `chaseCam.getResolvedState()` — returns `ChaseCamResolvedState | null` (null when not initialized)

- [ ] **Step 1: Write the failing test for `getResolvedState()` returning null initially**

Add to `console/src/gev-visual/cockpit/__tests__/chase-cam.test.ts`:

```typescript
import { mountCockpitChaseCam, type ChaseCamHandle } from "../chase-cam";

test("getResolvedState returns null before any tick", () => {
  const handle: ChaseCamHandle = mountCockpitChaseCam(/* deps */);
  expect(handle.getResolvedState()).toBeNull();
});
```

The existing test file's `mountCockpitChaseCam` deps signature is in P14's chase-cam.ts. Use the same minimal deps the existing tests use (mock viewer, getTrackedEntity returning null).

- [ ] **Step 2: Run test to verify it fails**

Run: `cd console && npx vitest run src/gev-visual/cockpit/__tests__/chase-cam.test.ts -t "getResolvedState returns null"`
Expected: FAIL — `getResolvedState is not a function`.

- [ ] **Step 3: Add `ChaseCamResolvedState` interface and `getResolvedState()` method**

In `console/src/gev-visual/cockpit/chase-cam.ts`, add near the existing `ChaseCamHandle` interface:

```typescript
export interface ChaseCamResolvedState {
  heading: number | null;
  pitch: number | null;
  range: number | null;
  bankRad: number;
  vsiMps: number;
  altitudeM: number | null;
}
```

Add a `resolvedState: ChaseCamResolvedState | null = null` field in the closure. Add a method to the returned handle:

```typescript
getResolvedState: () => resolvedState,
```

At the END of the existing tick (after the camera is set), populate it:

```typescript
resolvedState = {
  heading,
  pitch: /* existing pitch var */,
  range: /* existing range var */,
  bankRad: 0,        // Filled in Step 5
  vsiMps: 0,         // Filled in Step 7
  altitudeM: null,   // Filled in Step 6
};
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd console && npx vitest run src/gev-visual/cockpit/__tests__/chase-cam.test.ts -t "getResolvedState returns null"`
Expected: PASS

- [ ] **Step 5: Add bank math (Quaternion → roll)**

In `chase-cam.ts`, in the tick (after `const target = entity.position.getValue(...)`):

```typescript
let bankRad = 0;
const orientation = entity.orientation;
if (orientation) {
  try {
    const quat = orientation.getValue(clockTime);
    if (quat) {
      const hpr = Cesium.HeadingPitchRoll.fromQuaternion(quat);
      bankRad = hpr.roll;
    }
  } catch {
    bankRad = 0;
  }
}
```

Set `resolvedState.bankRad = bankRad;`

- [ ] **Step 6: Add altitude derivation**

In the tick, after `target` is resolved, add:

```typescript
let altitudeM: number | null = null;
if (target) {
  const carto = Cesium.Cartographic.fromCartesian(target);
  if (carto) altitudeM = carto.height;
}
```

Set `resolvedState.altitudeM = altitudeM;`

- [ ] **Step 7: Add VSI altitude history window**

In the closure (top of mount), add:

```typescript
const altitudeHistory: number[] = [];
const ALT_HISTORY_MAX = 8;  // 8 samples × 50ms = 400ms
```

In the tick, after `altitudeM` is computed:

```typescript
if (altitudeM !== null) {
  altitudeHistory.push(altitudeM);
  if (altitudeHistory.length > ALT_HISTORY_MAX) altitudeHistory.shift();
}
let vsiMps = 0;
if (altitudeHistory.length >= 2) {
  const dt = (altitudeHistory.length - 1) * 0.050;  // 50ms cadence
  vsiMps = (altitudeHistory[altitudeHistory.length - 1] - altitudeHistory[0]) / dt;
}
```

Set `resolvedState.vsiMps = vsiMps;`

- [ ] **Step 8: Write failing tests for bank + altitude + VSI**

Add to `console/src/gev-visual/cockpit/__tests__/chase-cam.test.ts`:

```typescript
test("bankRad defaults to 0 when entity has no orientation", async () => {
  const handle = mountCockpitChaseCam(/* deps with entity.orientation = undefined */);
  // Tick once
  handle.tick(/* force tick via internal trigger */);
  const state = handle.getResolvedState();
  expect(state?.bankRad).toBe(0);
});

test("vsiMps is 0 during warmup (< 2 samples)", async () => {
  const handle = mountCockpitChaseCam(/* deps */);
  handle.tick();
  handle.tick();
  expect(handle.getResolvedState()?.vsiMps).toBe(0);
});

test("altitudeHistory saturates at 8 samples", async () => {
  const handle = mountCockpitChaseCam(/* deps */);
  for (let i = 0; i < 20; i++) handle.tick();
  // Internal: altitudeHistory.length should be 8
  expect(/* inspect internal state via test hook */).toBe(8);
});

test("vsiMps derives linear rate over 400ms window", async () => {
  // Mock altitude: climb 100m over 400ms = 250 m/s
  const handle = mountCockpitChaseCam(/* deps with altitude climbing */);
  for (let i = 0; i < 8; i++) handle.tick();
  expect(handle.getResolvedState()?.vsiMps).toBeCloseTo(250, 0);
});
```

For internal state inspection, expose a test-only helper via the deps: `{ __testHooks?: { altitudeHistory?: number[] } }` (test-only, not in production type).

- [ ] **Step 9: Run tests to verify they pass**

Run: `cd console && npx vitest run src/gev-visual/cockpit/__tests__/chase-cam.test.ts`
Expected: PASS (all existing 131 tests + new tests pass; 131 was the P15 baseline)

- [ ] **Step 10: Commit**

```bash
cd console/src/gev-visual/cockpit
git add chase-cam.ts __tests__/chase-cam.test.ts
cd /Volumes/TBU/Workspace/IntelHub-p16
git commit -m "feat(gev-p16-t1): chase-cam — bank + altitude + VSI (8-sample, 400ms window)"
```

---

## Task 2: instruments-mount — frame extension

**Files:**
- Modify: `console/src/gev-visual/cockpit/instruments-mount.ts` (existing P9 file)
- Modify: `console/src/gev-visual/cockpit/__tests__/instruments-mount.test.ts` (existing P9 test file)

**Interfaces:**
- Consumes: `ChaseCamResolvedState` (Task 1)
- Produces: `CockpitInstrumentFrame` extended with `pitchRad`, `bankRad`, `vsiMps`

- [ ] **Step 1: Write failing test for frame extension**

Add to `console/src/gev-visual/cockpit/__tests__/instruments-mount.test.ts`:

```typescript
import { mountCockpitInstruments } from "../instruments-mount";

test("frame includes pitchRad/bankRad/vsiMps from chaseCam deps", () => {
  const chaseCam = {
    getResolvedState: () => ({
      heading: 1.0, pitch: 0.2, range: 100,
      bankRad: 0.1, vsiMps: 5.0, altitudeM: 1000,
    }),
  };
  const instruments = mountCockpitInstruments({ viewer, chaseCam });
  const frame = instruments.update({ /* trackedInfo */ });
  expect(frame.pitchRad).toBe(0.2);
  expect(frame.bankRad).toBe(0.1);
  expect(frame.vsiMps).toBe(5.0);
});

test("frame defaults pitch/bank/vsi to 0 when chaseCam is null", () => {
  const instruments = mountCockpitInstruments({ viewer });
  const frame = instruments.update({ /* trackedInfo */ });
  expect(frame.pitchRad).toBe(0);
  expect(frame.bankRad).toBe(0);
  expect(frame.vsiMps).toBe(0);
});

test("frame defaults pitch/bank/vsi to 0 when chaseCam.getResolvedState() returns null", () => {
  const chaseCam = { getResolvedState: () => null };
  const instruments = mountCockpitInstruments({ viewer, chaseCam });
  const frame = instruments.update({ /* trackedInfo */ });
  expect(frame.pitchRad).toBe(0);
  expect(frame.bankRad).toBe(0);
  expect(frame.vsiMps).toBe(0);
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd console && npx vitest run src/gev-visual/cockpit/__tests__/instruments-mount.test.ts -t "pitchRad/bankRad/vsiMps"`
Expected: FAIL — `pitchRad is undefined`.

- [ ] **Step 3: Extend `CockpitInstrumentFrame` interface**

In `instruments-mount.ts`, add to the existing `CockpitInstrumentFrame` interface:

```typescript
export interface CockpitInstrumentFrame {
  // ... existing fields ...
  pitchRad: number;
  bankRad: number;
  vsiMps: number;
}
```

- [ ] **Step 4: Refactor `mountCockpitInstruments` to accept `chaseCam` deps**

Change the function signature from:

```typescript
export function mountCockpitInstruments(
  viewer: { scene: { canvas?: unknown } },
  flights?: { getTrackedInfo(): CockpitTrackedInfo | null },
): InstrumentsHandle {
```

to:

```typescript
export interface CockpitInstrumentDeps {
  viewer: { scene: { canvas?: unknown } };
  flights?: { getTrackedInfo(): CockpitTrackedInfo | null };
  chaseCam?: { getResolvedState(): ChaseCamResolvedState | null };
}

export function mountCockpitInstruments(
  deps: CockpitInstrumentDeps,
): InstrumentsHandle {
```

Update internal `flights?.getTrackedInfo()` references to `deps.flights?.getTrackedInfo()`.

- [ ] **Step 5: Update `compute()` to include 3 new fields**

In the `compute()` function, add:

```typescript
const resolved = deps.chaseCam?.getResolvedState() ?? null;
return {
  // ... existing fields ...
  pitchRad: resolved?.pitch ?? 0,
  bankRad: resolved?.bankRad ?? 0,
  vsiMps: resolved?.vsiMps ?? 0,
};
```

- [ ] **Step 6: Update `InstrumentsHandle.update()` signature**

Change from:

```typescript
update(info?: CockpitTrackedInfo | null): CockpitInstrumentFrame;
```

to:

```typescript
update(
  info?: CockpitTrackedInfo | null,
  resolved?: ChaseCamResolvedState | null,
): CockpitInstrumentFrame;
```

In the implementation, if `resolved` is provided, use it directly (don't call `deps.chaseCam?.getResolvedState()` again — caller passes the resolved state to keep semantics explicit). If `resolved` is undefined, fetch from deps.

- [ ] **Step 7: Run tests to verify they pass**

Run: `cd console && npx vitest run src/gev-visual/cockpit/__tests__/instruments-mount.test.ts`
Expected: PASS (existing P9 tests still pass because `chaseCam` is optional; new tests pass).

- [ ] **Step 8: Fix any callers of the old signature**

The only known caller is `HudCockpitInstruments.tsx` (Task 5) and `GlobeV2.tsx` (Task 6). For now, `HudCockpitInstruments.test.tsx` may need updating:

```bash
cd console
grep -rn "mountCockpitInstruments" src/ __tests__/
```

If old-shape `mountCockpitInstruments(viewer, flights)` callers exist in tests, update them to `mountCockpitInstruments({viewer, flights})`.

- [ ] **Step 9: Run full cockpit test suite**

Run: `cd console && npx vitest run src/gev-visual/cockpit/`
Expected: PASS (existing 131 + ~5 new = 136 tests).

- [ ] **Step 10: Commit**

```bash
cd /Volumes/TBU/Workspace/IntelHub-p16
git add console/src/gev-visual/cockpit/instruments-mount.ts console/src/gev-visual/cockpit/__tests__/instruments-mount.test.ts
git commit -m "feat(gev-p16-t2): instruments-mount — frame.pitchRad/bankRad/vsiMps"
```

---

## Task 3: cockpit-hud-tick — 10 Hz timer module (NEW)

**Files:**
- Create: `console/src/gev-visual/cockpit/cockpit-hud-tick.ts`
- Create: `console/src/gev-visual/cockpit/__tests__/cockpit-hud-tick.test.ts`

**Interfaces:**
- Consumes: `InstrumentsHandle.update(info?, resolved?)` (Task 2); `ChaseCamHandle.getResolvedState()` (Task 1); `flights.getTrackedInfo()`
- Produces:
  - `mountCockpitHudTick(instruments, chaseCam, flights, options?)` → `HudTickHandle`
  - `HudTickHandle` interface — `{ start(): void; stop(): void; isRunning(): boolean }`

- [ ] **Step 1: Write failing test for start/stop lifecycle**

Create `console/src/gev-visual/cockpit/__tests__/cockpit-hud-tick.test.ts`:

```typescript
import { mountCockpitHudTick } from "../cockpit-hud-tick";

test("start() triggers tick on cadence", () => {
  vi.useFakeTimers();
  const instruments = { update: vi.fn(), destroy: vi.fn() };
  const chaseCam = { getResolvedState: () => null };
  const flights = { getTrackedInfo: () => null };
  const tick = mountCockpitHudTick(instruments, chaseCam as any, flights, { intervalMs: 100 });
  tick.start();
  vi.advanceTimersByTime(100);
  expect(instruments.update).toHaveBeenCalledTimes(1);
  vi.advanceTimersByTime(100);
  expect(instruments.update).toHaveBeenCalledTimes(2);
  tick.stop();
  vi.useRealTimers();
});

test("stop() cancels interval", () => {
  vi.useFakeTimers();
  const instruments = { update: vi.fn() };
  const chaseCam = { getResolvedState: () => null };
  const flights = { getTrackedInfo: () => null };
  const tick = mountCockpitHudTick(instruments, chaseCam as any, flights);
  tick.start();
  vi.advanceTimersByTime(100);
  tick.stop();
  vi.advanceTimersByTime(500);
  expect(instruments.update).toHaveBeenCalledTimes(1);  // Only the first tick fired
});

test("isRunning() reflects interval state", () => {
  vi.useFakeTimers();
  const instruments = { update: vi.fn() };
  const chaseCam = { getResolvedState: () => null };
  const flights = { getTrackedInfo: () => null };
  const tick = mountCockpitHudTick(instruments, chaseCam as any, flights);
  expect(tick.isRunning()).toBe(false);
  tick.start();
  expect(tick.isRunning()).toBe(true);
  tick.stop();
  expect(tick.isRunning()).toBe(false);
});
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd console && npx vitest run src/gev-visual/cockpit/__tests__/cockpit-hud-tick.test.ts`
Expected: FAIL — module not found.

- [ ] **Step 3: Implement `mountCockpitHudTick`**

Create `console/src/gev-visual/cockpit/cockpit-hud-tick.ts`:

```typescript
import type { InstrumentsHandle } from "./instruments-mount";

export interface HudTickOptions {
  intervalMs?: number;
  viewer?: { isDestroyed(): boolean };
}

export interface HudTickHandle {
  start(): void;
  stop(): void;
  isRunning(): boolean;
}

export function mountCockpitHudTick(
  instruments: InstrumentsHandle,
  chaseCam: { getResolvedState(): unknown },
  flights: { getTrackedInfo(): unknown },
  options: HudTickOptions = {},
): HudTickHandle {
  const intervalMs = options.intervalMs ?? 100;
  let handle: ReturnType<typeof setInterval> | null = null;

  function tick() {
    if (options.viewer?.isDestroyed?.()) {
      if (handle) clearInterval(handle);
      handle = null;
      return;
    }
    const info = flights.getTrackedInfo() as Parameters<typeof instruments.update>[0];
    const resolved = chaseCam.getResolvedState() as Parameters<typeof instruments.update>[1];
    instruments.update(info, resolved);
  }

  return {
    start() {
      if (handle) return;
      handle = setInterval(tick, intervalMs);
    },
    stop() {
      if (handle) {
        clearInterval(handle);
        handle = null;
      }
    },
    isRunning: () => handle !== null,
  };
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd console && npx vitest run src/gev-visual/cockpit/__tests__/cockpit-hud-tick.test.ts`
Expected: PASS (3 tests).

- [ ] **Step 5: Add viewer.isDestroyed() test**

Add to the test file:

```typescript
test("viewer.isDestroyed() stops the tick", () => {
  vi.useFakeTimers();
  const instruments = { update: vi.fn() };
  const chaseCam = { getResolvedState: () => null };
  const flights = { getTrackedInfo: () => null };
  let destroyed = false;
  const viewer = { isDestroyed: () => destroyed };
  const tick = mountCockpitHudTick(instruments, chaseCam as any, flights, { viewer });
  tick.start();
  vi.advanceTimersByTime(100);
  expect(instruments.update).toHaveBeenCalledTimes(1);
  destroyed = true;
  vi.advanceTimersByTime(100);
  expect(instruments.update).toHaveBeenCalledTimes(1);  // No more ticks after destroy
  expect(tick.isRunning()).toBe(false);
});
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `cd console && npx vitest run src/gev-visual/cockpit/__tests__/cockpit-hud-tick.test.ts`
Expected: PASS (4 tests).

- [ ] **Step 7: Commit**

```bash
cd /Volumes/TBU/Workspace/IntelHub-p16
git add console/src/gev-visual/cockpit/cockpit-hud-tick.ts console/src/gev-visual/cockpit/__tests__/cockpit-hud-tick.test.ts
git commit -m "feat(gev-p16-t3): cockpit-hud-tick — 10Hz update timer with lifecycle"
```

---

## Task 4: cockpit barrel — export new modules

**Files:**
- Modify: `console/src/gev-visual/cockpit/index.ts`

**Interfaces:**
- Consumes: `mountCockpitHudTick`, `HudTickHandle`, `CockpitInstrumentDeps` (Tasks 2, 3)
- Produces: re-exports for consumers

- [ ] **Step 1: Add new exports to barrel**

Edit `console/src/gev-visual/cockpit/index.ts` — add at the end:

```typescript
export {
  mountCockpitHudTick,
  type HudTickHandle,
  type HudTickOptions,
} from "./cockpit-hud-tick";

export {
  mountCockpitInstruments,
  type CockpitInstrumentDeps,
  type CockpitInstrumentFrame,
  type CockpitTrackedInfo,
  type CompassDivision,
  type RulerTick,
  type InstrumentsHandle,
} from "./instruments-mount";

export type {
  ChaseCamResolvedState,
} from "./chase-cam";
```

- [ ] **Step 2: Verify barrel compiles**

Run: `cd console && npx tsc --noEmit`
Expected: PASS (no new errors).

- [ ] **Step 3: Commit**

```bash
cd /Volumes/TBU/Workspace/IntelHub-p16
git add console/src/gev-visual/cockpit/index.ts
git commit -m "feat(gev-p16-t4): cockpit barrel — export hud-tick + instrument-frame types"
```

---

## Task 5: HudCockpitInstruments — render 5 new SVG groups

**Files:**
- Modify: `console/src/globe-hud/HudCockpitInstruments.tsx` (existing P9 file, 212 LoC)
- Modify: `console/src/globe-hud/__tests__/HudCockpitInstruments.test.tsx` (existing test)

**Interfaces:**
- Consumes: `CockpitInstrumentFrame` with new fields (Task 2)
- Produces: 6 SVG groups (1 existing heading compass + 5 new)

- [ ] **Step 1: Write failing test for altitude ladder rendering**

Add to `console/src/globe-hud/__tests__/HudCockpitInstruments.test.tsx`:

```typescript
import { render, screen } from "@testing-library/react";
import { HudCockpitInstruments } from "../HudCockpitInstruments";

test("renders altitude ladder with 9 ticks", () => {
  const frame = {
    heading: 0, headingLabel: "000",
    altitudeFt: 5000, altitudeLabel: "5,000",
    speedKt: 200, speedLabel: "200",
    callsign: "TEST123",
    pitchRad: 0, bankRad: 0, vsiMps: 0,
    compass: [], altitudeTicks: Array(9).fill({ slot: 0, label: "5", value: 5000, depth: 0, major: true, curve: 0 }),
    speedTicks: [],
  };
  render(<HudCockpitInstruments frame={frame} />);
  expect(screen.getByTestId("altitude-ladder")).toBeInTheDocument();
  const ticks = screen.getAllByTestId("altitude-tick");
  expect(ticks).toHaveLength(9);
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd console && npx vitest run src/globe-hud/__tests__/HudCockpitInstruments.test.tsx -t "altitude ladder"`
Expected: FAIL — element not found.

- [ ] **Step 3: Add altitude ladder SVG group**

In `HudCockpitInstruments.tsx`, add after the existing heading compass:

```typescript
function AltitudeLadder({ ticks }: { ticks: RulerTick[] }) {
  return (
    <svg
      data-testid="altitude-ladder"
      className="hud-cockpit-altitude-ladder"
      width="80"
      height="200"
      viewBox="0 0 80 200"
    >
      {ticks.map((tick) => (
        <g key={tick.slot} transform={`translate(0, ${100 + tick.slot * 20})`}>
          <line
            x1={tick.major ? 0 : 10}
            x2={tick.major ? 30 : 25}
            y1="0"
            y2="0"
            stroke="currentColor"
            strokeWidth="1"
          />
          {tick.major && (
            <text x="40" y="4" fontSize="10" fill="currentColor">
              {tick.label}
            </text>
          )}
        </g>
      ))}
    </svg>
  );
}
```

- [ ] **Step 4: Add speed tape SVG group**

```typescript
function SpeedTape({ ticks }: { ticks: RulerTick[] }) {
  return (
    <svg
      data-testid="speed-tape"
      className="hud-cockpit-speed-tape"
      width="80"
      height="200"
      viewBox="0 0 80 200"
    >
      {ticks.map((tick) => (
        <g key={tick.slot} transform={`translate(50, ${100 + tick.slot * 20})`}>
          <line
            x1={tick.major ? 50 : 35}
            x2={tick.major ? 20 : 30}
            y1="0"
            y2="0"
            stroke="currentColor"
            strokeWidth="1"
          />
          {tick.major && (
            <text x="0" y="4" fontSize="10" fill="currentColor" textAnchor="start">
              {tick.label}
            </text>
          )}
        </g>
      ))}
    </svg>
  );
}
```

- [ ] **Step 5: Add pitch ladder SVG group**

```typescript
function PitchLadder({ pitchRad, bankRad }: { pitchRad: number; bankRad: number }) {
  const PITCH_RANGE_DEG = [-30, -20, -10, 0, 10, 20, 30];
  return (
    <svg
      data-testid="pitch-ladder"
      className="hud-cockpit-pitch-ladder"
      width="200"
      height="300"
      viewBox="0 0 200 300"
    >
      <g transform={`translate(100, 150) rotate(${(bankRad * 180) / Math.PI})`}>
        {PITCH_RANGE_DEG.map((pitchDeg) => {
          const yOffset = (pitchDeg - (pitchRad * 180) / Math.PI) * 5;
          return (
            <g key={pitchDeg} transform={`translate(0, ${yOffset})`}>
              <line
                x1="-30"
                x2="30"
                y1="0"
                y2="0"
                stroke={pitchDeg === 0 ? "currentColor" : "rgba(255,255,255,0.6)"}
                strokeWidth={pitchDeg === 0 ? "2" : "1"}
              />
              <text x="-35" y="4" fontSize="10" fill="currentColor" textAnchor="end">
                {Math.abs(pitchDeg)}
              </text>
              <text x="35" y="4" fontSize="10" fill="currentColor">
                {Math.abs(pitchDeg)}
              </text>
            </g>
          );
        })}
      </g>
    </svg>
  );
}
```

- [ ] **Step 6: Add bank indicator SVG group**

```typescript
function BankIndicator({ bankRad }: { bankRad: number }) {
  const BANK_RANGE_DEG = [-60, -45, -30, -15, 0, 15, 30, 45, 60];
  return (
    <svg
      data-testid="bank-indicator"
      className="hud-cockpit-bank-indicator"
      width="200"
      height="60"
      viewBox="0 0 200 60"
    >
      {BANK_RANGE_DEG.map((deg) => (
        <g key={deg} transform={`translate(${100 + deg * 1.5}, 30)`}>
          {deg === 0 ? (
            <polygon points="0,-5 -4,5 4,5" fill="currentColor" />
          ) : (
            <text fontSize="10" fill="currentColor" textAnchor="middle">
              {Math.abs(deg)}
            </text>
          )}
        </g>
      ))}
      {/* Current bank marker */}
      <polygon
        points={`${100 + (bankRad * 180) / Math.PI * 1.5},50 ${100 + (bankRad * 180) / Math.PI * 1.5 - 5},60 ${100 + (bankRad * 180) / Math.PI * 1.5 + 5},60`}
        fill="cyan"
      />
    </svg>
  );
}
```

- [ ] **Step 7: Add VSI chevron SVG group**

```typescript
function VsiChevron({ vsiMps }: { vsiMps: number }) {
  const MPS_TO_FTPM = 196.85;
  const ftpm = Math.abs(vsiMps * MPS_TO_FTPM);
  const isUp = vsiMps > 0;
  const isDown = vsiMps < 0;
  return (
    <svg
      data-testid="vsi-chevron"
      className="hud-cockpit-vsi-chevron"
      width="40"
      height="200"
      viewBox="0 0 40 200"
    >
      {isUp && (
        <polygon points="20,90 10,100 30,100" fill="lime" data-testid="vsi-up" />
      )}
      {isDown && (
        <polygon points="20,110 10,100 30,100" fill="amber" data-testid="vsi-down" />
      )}
      <text x="20" y="160" fontSize="10" fill="currentColor" textAnchor="middle">
        {vsiMps !== 0 ? ftpm.toFixed(0) : "0"}
      </text>
    </svg>
  );
}
```

- [ ] **Step 8: Wire all 5 new groups into the main render**

In the main `HudCockpitInstruments` component, after the existing heading compass JSX, add:

```typescript
return (
  <div className="hud-cockpit-instruments" data-active={!!frame}>
    {/* Existing heading compass */}
    <HeadingCompass frame={frame} />

    {/* NEW: 5 elements */}
    {frame && (
      <>
        <AltitudeLadder ticks={frame.altitudeTicks} />
        <SpeedTape ticks={frame.speedTicks} />
        <PitchLadder pitchRad={frame.pitchRad} bankRad={frame.bankRad} />
        <BankIndicator bankRad={frame.bankRad} />
        <VsiChevron vsiMps={frame.vsiMps} />
      </>
    )}
  </div>
);
```

Adjust positioning with CSS (absolute positioning, `pointer-events: none`).

- [ ] **Step 9: Add `data-testid="altitude-tick"` to the rendered tick elements**

Update `AltitudeLadder` and `SpeedTape` to add `data-testid="altitude-tick"` (or `"speed-tick"`) to each tick group.

- [ ] **Step 10: Write remaining React tests**

Add to `HudCockpitInstruments.test.tsx`:

```typescript
test("renders speed tape with 9 ticks", () => { /* similar to altitude ladder */ });
test("pitch ladder rotates by bankRad", () => {
  const frame = { ..., pitchRad: 0, bankRad: Math.PI / 4, ... };  // 45° bank
  render(<HudCockpitInstruments frame={frame} />);
  const ladder = screen.getByTestId("pitch-ladder");
  // Assert rotation transform contains "rotate(45"
});

test("VSI chevron points up for positive vsiMps", () => {
  const frame = { ..., vsiMps: 5.0, ... };
  render(<HudCockpitInstruments frame={frame} />);
  expect(screen.getByTestId("vsi-up")).toBeInTheDocument();
});

test("VSI chevron points down for negative vsiMps", () => {
  const frame = { ..., vsiMps: -5.0, ... };
  render(<HudCockpitInstruments frame={frame} />);
  expect(screen.getByTestId("vsi-down")).toBeInTheDocument();
});

test("null frame shows neutral state", () => {
  render(<HudCockpitInstruments frame={null} />);
  expect(screen.queryByTestId("altitude-ladder")).not.toBeInTheDocument();
});
```

- [ ] **Step 11: Run all React tests to verify they pass**

Run: `cd console && npx vitest run src/globe-hud/__tests__/HudCockpitInstruments.test.tsx`
Expected: PASS (existing tests + 6 new = ~10 tests).

- [ ] **Step 12: Run full console test suite**

Run: `cd console && npx vitest run`
Expected: PASS (625 P15 baseline + ~25 new = ~650 tests).

- [ ] **Step 13: Commit**

```bash
cd /Volumes/TBU/Workspace/IntelHub-p16
git add console/src/globe-hud/HudCockpitInstruments.tsx console/src/globe-hud/__tests__/HudCockpitInstruments.test.tsx
git commit -m "feat(gev-p16-t5): HudCockpitInstruments — render altitude/speed/pitch/bank/VSI"
```

---

## Task 6: source-contracts test — single-source invariants

**Files:**
- Modify: `console/src/gev-visual/cockpit/__tests__/source-contracts.test.ts` (existing P15 test)

**Interfaces:**
- Consumes: cockpit barrel exports
- Produces: contract assertions

- [ ] **Step 1: Write failing contract tests**

Add to `source-contracts.test.ts`:

```typescript
test("cockpit-hud-tick module exports mountCockpitHudTick + HudTickHandle", async () => {
  const m = await import("../cockpit-hud-tick");
  expect(typeof m.mountCockpitHudTick).toBe("function");
  expect(typeof m.HudTickHandle).toBe("undefined");  // type-only, not runtime
});

test("ChaseCamResolvedState type is in barrel", async () => {
  const m = await import("../index");
  // Type-only check at compile time
  type _Check = m.ChaseCamResolvedState;
});

test("instruments-mount frame includes 3 new fields at compile time", () => {
  type Frame = {
    pitchRad: number;
    bankRad: number;
    vsiMps: number;
  };
  const _typeCheck: Frame = { pitchRad: 0, bankRad: 0, vsiMps: 0 };
  expect(_typeCheck).toBeDefined();
});
```

- [ ] **Step 2: Run tests to verify they pass**

Run: `cd console && npx vitest run src/gev-visual/cockpit/__tests__/source-contracts.test.ts`
Expected: PASS (existing P15 + new contracts).

- [ ] **Step 3: Commit**

```bash
cd /Volumes/TBU/Workspace/IntelHub-p16
git add console/src/gev-visual/cockpit/__tests__/source-contracts.test.ts
git commit -m "test(gev-p16-t6): source-contracts — cockpit-hud-tick + frame fields"
```

---

## Task 7: GlobeV2 wiring — mount HUD timer

**Files:**
- Modify: `console/src/pages/GlobeV2.tsx`

**Interfaces:**
- Consumes: cockpit active branch (existing P15), `mountCockpitInstruments`, `mountCockpitHudTick`
- Produces: HUD mounted + cleaned up on cockpit exit

- [ ] **Step 1: Find cockpit-active branch in GlobeV2**

```bash
grep -n "mountCockpitChaseCam\|mountCockpitMouseLook\|cockpitStore" console/src/pages/GlobeV2.tsx | head -10
```

Read the existing P15 wiring to understand the cleanup pattern.

- [ ] **Step 2: Add HUD instrumentation to the cockpit branch**

Inside the cockpit-active branch, after `mountCockpitChaseCam(...)`:

```typescript
const instruments = mountCockpitInstruments({
  viewer,
  flights: /* existing flights ref */,
  chaseCam,
});
const hudTick = mountCockpitHudTick(instruments, chaseCam, flights, { viewer });
hudTick.start();
```

Add to the cleanup function (after chase-cam destroy, mouse-look destroy):

```typescript
hudTick.stop();
instruments.destroy();
```

- [ ] **Step 3: Verify the wiring compiles**

Run: `cd console && npx tsc --noEmit`
Expected: PASS (no new errors).

- [ ] **Step 4: Commit**

```bash
cd /Volumes/TBU/Workspace/IntelHub-p16
git add console/src/pages/GlobeV2.tsx
git commit -m "feat(gev-p16-t7): GlobeV2 — mount cockpit HUD instruments + 10Hz timer"
```

---

## Task 8: sp8 acceptance — bundle checks + probe extension

**Files:**
- Modify: `scripts/accept-sp8.py`
- Modify: `console/probe-gev.mjs`

**Interfaces:**
- Consumes: existing sp8 bundle check pattern (P15)
- Produces: 6 new bundle checks + 1 probe check

- [ ] **Step 1: Add 6 new bundle checks to accept-sp8.py**

Add after the existing P15 checks (search for `p15: cockpit MOUSE_LOOK_ZERO_OFFSET`):

```python
# ---- GEV P16 (HUD avionics) bundle checks ----

ck_instruments = vm('grep -l "mountCockpitInstruments" /home/zou/IntelHub/console/dist/assets/*.js 2>/dev/null | head -1')
check("p16: mountCockpitInstruments bundled", bool(ck_instruments),
      ck_instruments or "mountCockpitInstruments not found in dist bundle")

ck_hudtick = vm('grep -l "mountCockpitHudTick" /home/zou/IntelHub/console/dist/assets/*.js 2>/dev/null | head -1')
check("p16: mountCockpitHudTick bundled", bool(ck_hudtick),
      ck_hudtick or "mountCockpitHudTick not found in dist bundle")

ck_get_resolved = vm('grep -l "getResolvedState" /home/zou/IntelHub/console/dist/assets/*.js 2>/dev/null | head -1')
check("p16: chase-cam.getResolvedState bundled", bool(ck_get_resolved),
      ck_get_resolved or "getResolvedState not found in dist bundle")

ck_hpr = vm('grep -l "fromQuaternion" /home/zou/IntelHub/console/dist/assets/*.js 2>/dev/null | head -1')
check("p16: HeadingPitchRoll.fromQuaternion bundled", bool(ck_hpr),
      ck_hpr or "fromQuaternion not found in dist bundle")

ck_altitude_history = vm('grep -l "altitudeHistory" /home/zou/IntelHub/console/dist/assets/*.js 2>/dev/null | head -1')
check("p16: altitude history window bundled", bool(ck_altitude_history),
      ck_altitude_history or "altitudeHistory not found in dist bundle")

ck_pitch_ladder = vm('grep -l "pitch-ladder\\|pitch_ladder\\|PitchLadder" /home/zou/IntelHub/console/dist/assets/*.js 2>/dev/null | head -1')
check("p16: pitch ladder SVG in dist", bool(ck_pitch_ladder),
      ck_pitch_ladder or "pitch ladder marker not found in dist bundle")
```

- [ ] **Step 2: Add probe extension for `__gevInstrumentsFrame`**

In `console/probe-gev.mjs`, find the section that exposes `__gevViewer` / `__gevTrackedEntity` globals (P15 deferred these to P16). Add:

```javascript
// P16: expose HUD instruments frame for probe checks
window.__gevInstrumentsFrame = instruments.getFrame();
```

Add a probe step that:
1. Mounts cockpit (already exists in probe)
2. Selects a tracked aircraft (already exists in probe)
3. Waits 250 ms (so altitude history fills + VSI derives)
4. Reads `__gevInstrumentsFrame`
5. Asserts:
   - `frame.headingLabel` is a 3-digit string
   - `frame.altitudeLabel` is a number with commas
   - `frame.speedLabel` matches `formatSpeedRulerTick` shape
   - `frame.pitchRad` is a finite number
   - `frame.bankRad` is a finite number
   - `frame.vsiMps` is a finite number

Add at the bottom of the probe's main block:

```javascript
const frame = await page.evaluate(() => window.__gevInstrumentsFrame);
if (!frame) {
  console.log("p16-hud-frame=FAIL (no frame exposed)");
} else {
  const ok = (
    /^\d{3}$/.test(frame.headingLabel) &&
    /^[0-9,]+$/.test(frame.altitudeLabel) &&
    Number.isFinite(frame.pitchRad) &&
    Number.isFinite(frame.bankRad) &&
    Number.isFinite(frame.vsiMps)
  );
  console.log(`p16-hud-frame=${ok ? 'PASS' : 'FAIL'} (heading=${frame.headingLabel} alt=${frame.altitudeLabel} speed=${frame.speedLabel} pitch=${frame.pitchRad?.toFixed(2)} bank=${frame.bankRad?.toFixed(2)} vsi=${frame.vsiMps?.toFixed(2)})`);
}
```

- [ ] **Step 3: Run sp8 locally (syntax check)**

Run: `python3 -c "import ast; ast.parse(open('scripts/accept-sp8.py').read()); print('OK')"`
Expected: OK

- [ ] **Step 4: Commit**

```bash
cd /Volumes/TBU/Workspace/IntelHub-p16
git add scripts/accept-sp8.py console/probe-gev.mjs
git commit -m "test(gev-p16-t8): sp8 + probe — P16 HUD avionics bundle checks + frame probe"
```

---

## Task 9: Deploy — 315 build/accept → main merge → 410 build/accept → push

**Files:** none modified; pure ops.

- [ ] **Step 1: Verify all tests pass locally**

```bash
cd /Volumes/TBU/Workspace/IntelHub-p16
cd console && npx tsc --noEmit
npx vitest run
```

Expected: tsc clean; vitest ~650 tests pass (625 P15 + 25 P16).

- [ ] **Step 2: rsync to 315 test VM**

```bash
cd /Volumes/TBU/Workspace/IntelHub-p16
rsync -az --delete \
  --exclude '.git' --exclude 'backups/' --exclude '.DS_Store' \
  --exclude 'compose/.env' --exclude 'compose/.env.crucix' --exclude 'docs/' --exclude 'build/' \
  --exclude 'config/searxng/' --exclude 'hub-core/target/' \
  --exclude 'console/node_modules/' --exclude 'console/dist/' \
  --exclude 'core/' --exclude 'data/' \
  ./ Debian-test:/home/zou/IntelHub/
```

- [ ] **Step 3: Build + restart on 315**

```bash
ssh -o BatchMode=yes Debian-test 'cd /home/zou/IntelHub \
  && bash scripts/build-hub.sh 2>&1 | grep -E "^error|Finished" | head -3 \
  && bash scripts/build-console.sh 2>&1 | tail -3 \
  && sudo systemctl restart hub-core && sleep 4 && systemctl is-active hub-core'
```

- [ ] **Step 4: Wait 5 minutes (PVE40 stampede prevention)**

```bash
echo "Waiting 5 minutes..."
sleep 300
```

- [ ] **Step 5: Run sp8 + sp6 + sp7 + sp3 on 315**

```bash
cd /Volumes/TBU/Workspace/IntelHub-p16
KEY=$(ssh -o BatchMode=yes Debian-test 'grep -o "ihk_[a-f0-9]*" /home/zou/IntelHub/core/agent-keys.txt | head -1')
for a in sp8 sp6 sp7 sp3; do
  echo "── $a ──"
  INTELHUB_SSH=Debian-test python3 scripts/accept-$a.py "$KEY" http://10.10.10.35:8800 2>&1 \
    | grep -E '==.*(passed|failed|shelved)' | tail -1
done
```

Expected: sp8 = 73 passed (67 P15 + 6 P16) / 2 shelved / 3 deferred / 0 failed; sp6 49/5/3; sp7 16/11/0; sp3 19/0/0.

- [ ] **Step 6: Merge to main + remove worktree + branch**

```bash
cd /Volumes/TBU/Workspace/IntelHub
git merge --no-ff feat/gev-p16-cockpit-hud-avionics -m "merge(gev-p16): cockpit HUD avionics (6 elements)"
git worktree remove /Volumes/TBU/Workspace/IntelHub-p16
git branch -d feat/gev-p16-cockpit-hud-avionics
```

- [ ] **Step 7: rsync main → IntelHub (production)**

```bash
cd /Volumes/TBU/Workspace/IntelHub
# If 410's .git is a stale worktree pointer, recover first:
ssh -o BatchMode=yes IntelHub 'cd /home/zou/IntelHub && rm -f .git && git init -b main && git remote add origin https://github.com/rootazero/IntelHub.git && git fetch origin main && git update-ref refs/heads/main origin/main && git branch --set-upstream-to=origin/main main' 2>/dev/null

rsync -az --delete \
  --exclude '.git' --exclude 'backups/' --exclude '.DS_Store' \
  --exclude 'compose/.env' --exclude 'compose/.env.crucix' --exclude 'docs/' --exclude 'build/' \
  --exclude 'config/searxng/' --exclude 'hub-core/target/' \
  --exclude 'console/node_modules/' --exclude 'console/dist/' \
  --exclude 'core/' --exclude 'data/' \
  ./ IntelHub:/home/zou/IntelHub/
```

- [ ] **Step 8: Build + restart on 410**

```bash
ssh -o BatchMode=yes IntelHub 'cd /home/zou/IntelHub \
  && bash scripts/build-hub.sh 2>&1 | grep -E "^error|Finished" | head -3 \
  && bash scripts/build-console.sh 2>&1 | tail -3 \
  && sudo systemctl restart hub-core && sleep 4 && systemctl is-active hub-core'
```

- [ ] **Step 9: Wait 5 minutes**

```bash
echo "Waiting 5 minutes..."
sleep 300
```

- [ ] **Step 10: Run sp8 + sp6 + sp7 + sp3 on 410**

```bash
cd /Volumes/TBU/Workspace/IntelHub
KEY=$(ssh -o BatchMode=yes IntelHub 'grep -o "ihk_[a-f0-9]*" /home/zou/IntelHub/core/agent-keys.txt | head -1')
for a in sp8 sp6 sp7 sp3; do
  echo "── $a ──"
  INTELHUB_SSH=IntelHub python3 scripts/accept-$a.py "$KEY" http://10.10.10.41:8800 2>&1 \
    | grep -E '==.*(passed|failed|shelved)' | tail -1
done
```

Expected: sp8 = 76 passed (67 P15 + 6 P16 + 3 misc) / 2 shelved / 3 deferred / 0 failed; sp6/sp7/sp3 unchanged.

- [ ] **Step 11: Push to GitHub**

```bash
cd /Volumes/TBU/Workspace/IntelHub
git push origin main
```

- [ ] **Step 12: Update AGENTS.md with P16 ops note**

Append to AGENTS.md:

```markdown
## GEV P16 — Cockpit HUD Avionics (2026-09-22)

- **Branch**: `feat/gev-p16-cockpit-hud-avionics` (merged + pushed to main; final HEAD captured at deploy time).
- **Behavior**: All 6 HUD avionics elements render: heading tape / altitude ladder / speed tape / pitch ladder / bank indicator / VSI chevron. Driven by chase-cam's `getResolvedState()` at 10 Hz (matches vendor `COCKPIT_HUD_UPDATE_MS = 100`).
- **Architecture**: chase-cam extends with bank (Cesium HeadingPitchRoll.fromQuaternion) + altitude + VSI (8-sample, 400ms window). instruments-mount adds 3 frame fields. NEW mountCockpitHudTick owns the 10 Hz timer. HudCockpitInstruments extends in place to render 5 new SVG groups.
- **Tests**: ~25 new tests added (chase-cam 4 + instruments-mount 3 + cockpit-hud-tick 4 + source-contracts 3 + HudCockpitInstruments 6 + HudCockpitFrame regression 5). Full console ~650/650 pass.
- **Acceptance**: sp8 73+ passed on 315/410, 0 failed. sp6/sp7/sp3 unchanged.
- **Notable rulings**:
  - VSI window was originally 4 samples (200ms) but jittery; corrected to 8 samples (400ms) during spec self-review.
  - Approach 1 (chase-cam owns attitude) over separate cockpit-attitude module — single source of truth.
  - Component extends in place (option a) over split — keeps wiring simple.
- **Roadmap** (per spec §8): HUD customization, SVS/TCAS/replay.
```

```bash
git add AGENTS.md
git commit -m "docs(gev-p16): AGENTS.md ops note — P16 HUD avionics shipped"
git push origin main
```
