# GEV P14 — Cockpit View: Real-Time Chase-Cam + Model Hide + FlyTo Refit

**Status**: architectural design — minimum scope
**Date**: 2026-09-21
**Branch**: `feat/gev-p14-cockpit-chase-cam`
**Base**: `0086434` (main, post-GEV-P13)
**Path**: minimum only (§4). Roadmap for medium/large deferred to §6.

## 1. Context

Today, clicking an aircraft and selecting **驾驶舱视角 (cockpit view)** does not feel
first-person: the plane model stays pinned at the centre of the frame and the
camera does not follow the aircraft's motion. The current IntelHub
implementation wires only the cockpit enter/exit `flyTo` (GEV P12 T6) — a
one-shot cinematic transition to an overhead, south-offset, down-tilted pose
(`TRACK_LAT_OFFSET_DEG=-0.5`, `TRACK_ALT_OFFSET_M=1500`, pitch `-20°`).
There is **no continuous chase-cam loop**, so once the flight ends, the
camera freezes in space while the aircraft drifts off-frame.

The reference project `gods-eye-view/src/ui/cockpitCamera.js` solves this with
a 50 ms per-frame loop that:

1. reads the tracked entity's world position,
2. slews a smoothed heading from the aircraft's track angle,
3. builds an East-North-Up (ENU) frame at the aircraft position,
4. composes a forward direction from `slew(heading)` + `pitch`,
5. places the camera `COCKPIT_FORWARD_OFFSET_M=7 m` ahead of and
   `COCKPIT_UP_OFFSET_M=2.6 m` above the aircraft, and
6. drives `viewer.camera.setView(...)` (synchronous, no `flyTo`).

The GE project's React port in `console/` already vendors that engine
(`console/gev-engine/` is byte-identical to the reference) and imports
`cockpitMath.js` helpers, but **the chase-cam loop is not wired into the
React tree** — `briefing-mount.ts` explicitly states *"we do NOT instantiate
that controller"* (D1). The math + constants are reachable but unused.

## 2. Problem Statement

The cockpit overlay is functional (briefing, instruments, vision mode,
keyboard shortcuts) but the **camera driver is missing**. Three user-visible
defects follow:

| # | Defect | Root cause |
|---|---|---|
| 1 | Plane model sits in the middle of the screen, camera feels static | No per-frame chase loop; camera frozen post-flyTo |
| 2 | View is overhead-tilted, not first-person | `flyToTracked` lands at `lat+(-0.5°)`, `alt+1500 m`, pitch `-20°` |
| 3 | Plane model occupies the lower half of the view | Never hidden; with the chase-cam fixed, hiding it becomes the visible win |

## 3. Goals & Non-Goals

### Goals (minimum)

- **G1** Continuous chase-cam: when cockpit is active, camera follows the
  tracked aircraft with vendor `cockpitCamera.update()` math and cadence.
- **G2** Plane model hidden during cockpit, restored on exit (so the frame is
  empty sky + HUD + instruments, not "model + cockpit chrome").
- **G3** `flyToTracked` lands at the chase pose (in front of and slightly
  above the aircraft, pitch `-4°`) for a smooth 0.4 s settle-in, then the
  chase loop takes over.
- **G4** `flyBackToBaseline` flies back to the pre-cockpit pose (no change).
- **G5** Tests: chase-cam unit (cadence gate + ENU math), integration
  (mount/unmount/store-wired lifecycle), source-contracts guard.

### Non-goals (this PR)

- ❌ Mouse-look / right-drag-to-look-around — roadmap §6.1
- ❌ Wheel-zoom recovery — roadmap §6.1
- ❌ HUD upgrade (heading tape / altitude ladder / speed tape) — roadmap §6.2
- ❌ Re-evaluating GE constant values (`7 m / 2.6 m / -4° / 50 ms`) — they are
  proven; we adopt them verbatim.

## 4. Minimum-Scope Design

### 4.1 Files Touched

| File | Change | Lines (est.) |
|---|---|---|
| `console/src/gev-visual/cockpit/chase-cam.ts` | **new** — `mountCockpitChaseCam` factory + rAF loop | ~150 |
| `console/src/gev-visual/cockpit/index.ts` | barrel export | +2 |
| `console/src/gev-visual/cockpit/camera-transition.ts` | replace overhead pose with chase pose; rename for clarity | ~30 |
| `console/src/gev-visual/cockpit/model-visibility.ts` | **new** — `mountModelVisibility(deps)` show/hide helpers | ~40 |
| `console/src/pages/GlobeV2.tsx` | wire chase-cam + model-visibility in the cockpit enter effect (around line 882) | ~25 |
| `console/src/gev-visual/cockpit/__tests__/chase-cam.test.ts` | **new** — cadence + ENU + lifecycle | ~120 |
| `console/src/gev-visual/cockpit/__tests__/model-visibility.test.ts` | **new** — show on enter / restore on exit | ~60 |
| `console/src/gev-visual/cockpit/__tests__/source-contracts.test.ts` | **extend** — assert chase-cam URL/path literals unchanged | +30 |

**Total**: ~457 lines added across 8 files. No deletions.

### 4.2 Module: `chase-cam.ts`

```ts
// GEV P14 — cockpit chase-cam driver. Ports gods-eye-view
// src/ui/cockpitCamera.update() (lines 50-352) into the React tree.
//
// Why: GEV P9/P12 wired only the cockpit overlay + enter/exit flyTo; no
// per-frame camera driver. The user sees a frozen plane model after enter.
// Reference project proves a 50 ms rAF loop with slewHeading + ENU
// composition + camera.setView keeps the camera glued to the aircraft.
//
// Lifecycle:
//   mountCockpitChaseCam(deps) → ChaseCamHandle
//     .start()  — begin rAF, subscribe cockpitStore
//     .stop()   — pause rAF, keep subscription
//     .destroy() — cancel rAF, unsubscribe, drop refs

**In-flight cooperation**: chase-cam **must not** call `viewer.camera.setView`
while `camera-transition` is mid-`flyTo` (Cesium aborts the flight on
`setView`). `mountCockpitCameraTransition` exposes `isInFlight(): boolean`;
chase-cam consults it before every setView and skips the tick if true. No
Cesium private-API access needed.

export interface ChaseCamDeps {
  viewer: { camera: CesiumCameraLike; clock?: { currentTime: Date } };
  store: { getState(): { active: boolean; trackedId: string | null };
           subscribe(fn: () => void): () => void };
  getTrackedEntity: () => Cesium.Entity | null; // wraps followRef
  getHeading: () => number | null;              // wraps followRef.trackInfo
}

export interface ChaseCamHandle {
  start(): void;
  stop(): void;
  destroy(): void;
}
```

**Internal state** (closure-scoped, like vendor `cockpitCamera.js`):
- `scratchEnu, scratchForward, scratchUp, scratchTarget, scratchCamera` (5× `Cesium.Cartesian3`)
- `cockpitAnchor` (the smoothed anchor — corrected via `cockpitAnchorCorrectionStep`)
- `heading` (number, slewed via `slewHeading`)
- `lastFrameMs`, `lastCameraUpdateMs`, `cockpitAnchorValid`

**rAF body** (port of `cockpitCamera.update`):

```
1. if !active → poll cockpitStore.getState().active 4 Hz, return
2. if !cockpitUiUpdateDue(nowMs, lastCameraUpdateMs, 50) → return
3. lastCameraUpdateMs = nowMs
4. entity = getTrackedEntity(); if !entity → return (don't auto-exit; P14 keeps existing auto-exit semantics in camera-transition exit handler)
5. target = entity.position.getValue(viewer.clock.currentTime)
6. dtSec = clamp((nowMs - lastFrameMs)/1000, 0, 0.1)
7. if heading finite → heading = slewHeading(heading, info.track, 28 * dtSec)
8. if !cockpitAnchorValid → cockpitAnchor = clone(target)
9. else → cockpitAnchor += cockpitAnchorCorrectionStep(distanceToTarget, speed, dtSec)  // vendor: clamps anchor to a stable spot behind/with the plane
10. ENU = eastNorthUpToFixedFrame(cockpitAnchor)
11. forward = (sin(h)*cos(pitch), cos(h)*cos(pitch), sin(pitch)) rotated by ENU
12. up     = ENU.up * 1
13. cameraPos = cockpitAnchor + forward*7 + up*2.6
14. camera.setView({ destination: cameraPos, orientation: { direction: forward, up } })
```

**Vendor imports** (already vendored, currently unused):
```ts
import { slewHeading, cockpitAnchorCorrectionStep, cockpitUiUpdateDue }
  from "gev-engine/src/cockpitMath.js";
import {
  COCKPIT_FORWARD_OFFSET_M, COCKPIT_UP_OFFSET_M,
  COCKPIT_HEADING_SLEW_DPS, COCKPIT_VIEW_PITCH_DEG,
  COCKPIT_CAMERA_UPDATE_MS,
} from "gev-engine/src/ui/cockpitPresentation.js";
```

### 4.3 Module: `model-visibility.ts`

Tiny adapter — hides the tracked entity's `.show` while cockpit is active.
Subscribes to `cockpitStore`; on `enter` records `entity.show === true`, sets
`entity.show = false`; on `exit` restores the prior value. Same pattern as
existing `camera-transition.ts` baseline-capture (D3 lesson: assert the seam
at mount so a lenient mock cannot hide a wrong object).

### 4.4 Module: `camera-transition.ts` (edit)

Replace the destination math:

```ts
// BEFORE (overhead tilted — GEV P12 T6)
destination = (lon, lat + TRACK_LAT_OFFSET_DEG, alt + TRACK_ALT_OFFSET_M)
orientation = { heading: 0, pitch: -20°, roll: 0 }

// AFTER (chase pose entry — GEV P14 T1)
destination = chaseEntryPose(target)  // = anchor + forward*7 + up*2.6
orientation = { direction: forward, up: ENU.up }  // same as chase-cam uses
duration    = 0.4 s   // was 0.7 s — chase loop takes over sooner
```

`flyBackToBaseline` unchanged. The transition becomes "settle-in to chase
pose" rather than "establish a fixed overhead view".

**New export** — chase-cam needs to know whether a flyTo is in flight:

```ts
export interface CockpitCameraTransition {
  // existing fields...
  /** True while a flyTo is in flight (chase-cam skips setView ticks until false). */
  isInFlight(): boolean;
}
```

Implementation: closure-scoped `inFlightPromise: Promise<void> | null`;
`flyToTracked` sets it before `camera.flyTo(...)` and clears it on the
returned promise's resolution. `flyBackToBaseline` does the same.

### 4.5 Wiring: `pages/GlobeV2.tsx` (line ~882)

Add to the existing cockpit enter/exit effect (the one that already
destroys `cockpitVisionRef`, `cockpitBriefingRef`, `cockpitInstrumentsRef`):

```ts
const chaseCamRef = useRef<ChaseCamHandle | null>(null);
const modelVisRef = useRef<ModelVisibilityHandle | null>(null);

// inside the cockpit mount effect:
chaseCamRef.current = mountCockpitChaseCam({
  viewer: viewerRef.current,
  store: cockpitStore,
  getTrackedEntity: () => followRef.current?.trackedEntity?.() ?? null,
  getHeading: () => followRef.current?.trackInfo?.()?.track ?? null,
});
chaseCamRef.current.start();
modelVisRef.current = mountModelVisibility({
  store: cockpitStore,
  getTrackedEntity: () => followRef.current?.trackedEntity?.() ?? null,
});

// inside the cleanup:
chaseCamRef.current?.destroy();  chaseCamRef.current = null;
modelVisRef.current?.destroy();  modelVisRef.current = null;
```

The existing `onToggleCockpit` at line 1001 calls `followRef.current.follow("flight", id)`
**first**, then `cockpitStore.enter(id)` (verified by reading line 1018-1019).
This is the right order for follow — it resolves the entity pointer so
chase-cam's first rAF tick has `getTrackedEntity()` non-null. The mount
effect (line 882 area) listens for `cockpitStore.active === true` and
inside it calls `flyToTracked()` and `chaseCamRef.current.start()`. The
chase-cam body guards each setView on `transition.isInFlight()` (see
`mountCockpitCameraTransition` §4.4) so the in-flight flyTo is not aborted.

### 4.6 Tests

**Unit (`__tests__/chase-cam.test.ts`)**:
- cadence gate: 50 ms spacing (use `jest.useFakeTimers` + manual rAF flush)
- ENU math: given a known entity position + heading, output camera destination
  matches `cockpitPresentation` formula within 1 cm
- slewHeading integration: bank turn over 5 s produces monotonic heading
  delta ≤ `COCKPIT_HEADING_SLEW_DPS`
- lifecycle: `destroy()` cancels rAF, unsubscribes, sets refs to null

**Unit (`__tests__/model-visibility.test.ts`)**:
- `enter` with tracked entity: `entity.show === false`
- `exit` after enter: `entity.show === true` (restored)
- `enter` without tracked entity: no-op (graceful — user might click before
  follow resolves)
- `destroy()` mid-active: no further show toggles

**Source contracts (`__tests__/source-contracts.test.ts` extension)**:
- assert `chase-cam.ts` does NOT export any `viewer.entities.add` (it's a
  camera driver, not a data source)
- assert the chase-cam URL/path literals (vendor cockpitMath,
  cockpitPresentation) match what source-contracts pins

**Acceptance**:
- sp8 cockpit section (P3 deferred items + new chase-cam probe)
- probe: enter cockpit on a flight → after 3 s, camera position relative to
  entity is within `(7±0.5 m forward, 2.6±0.5 m up, -4°±0.5° pitch)`

### 4.7 Risks

| Risk | Mitigation |
|---|---|
| Vendor `cockpitCamera.update()` uses `this.cockpitAnchorValid` flag and `this.contextNavigationDeadlineMs` — porting may miss context-switch race | Drop context navigation (not in scope). Add explicit unit test for `setView` cadence |
| Cesium model inside the camera frustum still renders (the entity `.show = false` is one model; multi-model composite planes may need recursive hide) | For P14: only hide the top-level tracked entity. If multi-model composites appear in production, layer 2 in §6.2 |
| `setView` while another `flyTo` is in flight aborts the flyTo | `mountCockpitCameraTransition` exposes `isInFlight()`; chase-cam skips ticks while true. Cesium docs §Camera: `camera.setView` cancels any in-flight `flyTo` |
| `flyToTracked` 0.4 s and chase-cam first tick can fight for one frame | Add a `flyInProgress` flag: chase-cam checks `viewer.scene.camera._currentFlight` (Cesium internal) before each setView; if a flight is in progress, skip that tick. Fall back to "always setView after 500 ms regardless" if Cesium internal API proves unstable |
| Camera-clipping inside the model: at `+0 m forward, +0.6 m up` you'd see inside the cockpit — we're using `+7 m forward` which keeps the camera in front of the nose | Documented in §6 — pure first-person is a separate future task |
| First-frame race: `onToggleCockpit` does `follow(...)` then `cockpitStore.enter(...)`. The mount effect on `active===true` fires AFTER the next React commit, so chase-cam's rAF starts ≥0–1 frame after the follow call | Tests cover: enter → wait 200 ms → camera position relative to entity is in chase envelope (chase-cam has had time to start ticking) |
| P9 viewport-lock + cockpit lock already in production — conflict with chase-cam resize | `viewport-lock.ts` is a separate concern (window resize); no conflict (verified by reading the file in GEV P12 T7) |

## 5. Migration / Deploy Notes

- No DB migration. No API changes. No new env vars.
- Console rebuild only (Rust hub unchanged).
- Deploy order: 315 test VM first (per AGENTS.md iron rule). sp8 cockpit
  probe must pass before merging to main, then 410 production redeploy
  console bundle.
- `build-console.sh` produces the bundle with embedded VITE_CARTO_KEY
  (already exercised for P9/P12). No new env.

## 6. Roadmap (NOT in this PR)

### 6.1 Medium — Mouse-look + wheel-zoom (estimated ~150 LoC)

**Scope**: when cockpit is active, drag with right mouse button pans the
camera around the aircraft (mouse-look, no entity movement), releasing
snaps back to forward view; mouse-wheel zooms within `[50 m, 5000 m]`
range (currently `viewport-lock.ts` disables wheel during cockpit).

**Files**: `gev-visual/cockpit/mouse-look.ts` (new, ~90 LoC),
`gev-visual/cockpit/viewport-lock.ts` (extend, ~20 LoC), tests ~40 LoC.

**Why deferred**: P14 minimum fixes the camera-frames-the-aircraft problem;
mouse-look is a separate ergonomic concern. We don't want to ship two UX
changes at once if either has a subtle issue.

**Why not larger**: a first-person mouse-look requires the camera to be
inside the model (clipped geometry) or hidden model with an external rig;
both have open questions. Better to land the foundational chase loop first,
then iterate.

### 6.2 Large — HUD avionics upgrade (estimated ~300 LoC)

**Scope**: while cockpit is active, replace the current flat overlay
instruments with a reactive heads-up display:

- Heading tape (compass ribbon) along the top
- Altitude ladder (right side, vertical)
- Speed tape (left side, vertical)
- Pitch ladder (centre, banking indicator)
- Bank indicator (horizon line)
- Vertical speed chevron (in altitude ladder)

All driven by the same `cockpitAnchor` and `heading` already computed by
the chase-cam module — no second source of truth. Updates at 100 ms
cadence (vendor `COCKPIT_HUD_UPDATE_MS`).

**Files**: `gev-visual/cockpit/instruments-mount.ts` (extend, ~150 LoC),
`hud.css` (~50 LoC), tests ~100 LoC.

**Why deferred**: the chase-cam and HUD can land independently — but
they share `cockpitAnchor` + `heading`. If the chase-cam API changes
between P14 and the HUD PR, HUD has to track. Locking the chase-cam API
first avoids rework.

**Why not larger**: real-deal flight-sim fidelity (terrain-aware pitch,
synthetic vision) is its own product question (would the user even want
it?). P14's HUD upgrade is a presentation refinement, not a feature.

### 6.3 Even-larger (separate sub-project, not in this roadmap)

- Synthetic vision system (SVS) — terrain-rendered HUD overlay
- TCAS-style traffic indicators in HUD
- Replay-mode cockpit (fly a recorded flight through any past track)

These cross into "interactive flight simulator", which is a different
product category. Flag for product decision.

## 7. Open Questions / Deferred Decisions

1. **Wheel-zoom behaviour during chase**: does mouse-zoom override the
   chase-cam forward offset, or scale it (i.e. wheel out → camera moves
   further from plane, wheel in → camera comes closer)? Default for §6.1
   is "scale the forward offset" so the user always rides the aircraft.
2. **Multi-model composite aircraft**: OpenSky / ADSBX feeds may give
   plane models with nested sub-entities (e.g. shadow + body). `entity.show
   = false` hides the parent; verify children aren't auto-rendered. Tracked
   under §4.7 risks.
3. **`setView` vs `flyTo` mid-chase**: if user clicks another aircraft
   while cockpit is active, should the camera fly there (cinematic) or
   snap (instant)? Current P9 behavior: snap (selection updates entity
   pointer, no camera animation). P14 keeps snap. Future option.

## 8. Acceptance Plan

| SP | New / extended | Probe |
|---|---|---|
| sp8 (cockpit overlay) | +2 checks: chase-cam cadence gate, model-visibility toggle | enter cockpit → wait 3 s → camera position relative to entity within `(7±0.5 m forward, 2.6±0.5 m up)` |
| sp8 (probe extension) | +1 check: heading slew — bank 90° over 4 s → camera heading reaches target within `COCKPIT_HEADING_SLEW_DPS * 4 = 112°` window |
| sp6 (no change) | — | baseline regression |
| sp3 (no change) | — | baseline regression |

Total addition: 3 cockpit-section checks + 1 probe extension. No SP
degradation.

## 9. References

- Reference impl: `/Volumes/TBU/Github/gods-eye-view/src/ui/cockpitCamera.js` (lines 50-352, the `update()` body)
- Vendor math: `/Volumes/TBU/Github/gods-eye-view/src/cockpitMath.js`
- Vendor constants: `/Volumes/TBU/Github/gods-eye-view/src/ui/cockpitPresentation.js` (`COCKPIT_*`)
- Vendored copies in IntelHub: `console/gev-engine/src/cockpitMath.js`, `console/gev-engine/src/ui/cockpitPresentation.js` (byte-identical per AGENTS.md GEV engine pin)
- Prior work: GEV P9 (`feat/gev-p9-cockpit-briefing`), GEV P12 (`ea7ae98`), GEV P13 (`0086434`)
- AGENTS.md iron rules: 410 production isolation, worktree-first, 315-test-first deploy