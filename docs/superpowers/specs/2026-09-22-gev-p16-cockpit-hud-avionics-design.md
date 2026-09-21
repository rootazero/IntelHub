# GEV P16 — Cockpit HUD Avionics (Spec)

> P16 of the GEV (gods-eye-view) console port. Follows P15 (mouse-look +
> wheel-zoom) on branch `feat/gev-p16-cockpit-hud-avionics`. Unlocks
> spec §6.2 from P15.

## 1. Background

P14 added a chase-cam that glues the operator's camera to a tracked
aircraft. P15 added right-drag mouse-look + wheel-zoom on top. Both PRs
delivered the camera system but left the HUD minimal — only a heading
compass rendered via `HudCockpitInstruments.tsx` (212 LoC).

Operators flying the cockpit today see no altitude readout, no speed
tape, no pitch reference, no bank indicator, and no vertical-speed
(VSI) chevron. Spec §6.2 (sketched in P15's spec lines 461–466)
identified this gap; this PR closes it.

Vendor `gods-eye-view` has the same problem space solved — its
`src/ui/cockpitInstruments.js` (260 LoC) renders a compass + speed
tape + altitude tape using math from `src/cockpitMath.js` (which we
already ported into `console/gev-engine/src/cockpitMath.js` via P9).
Our `console/src/gev-visual/cockpit/instruments-mount.ts` (P9, 168
LoC) is the pure-computation adapter that wraps vendor math into a
`CockpitInstrumentFrame` consumable by React.

The missing pieces are: (a) extending the frame with attitude fields
(pitch / bank / VSI) that vendor doesn't track, (b) React SVG
rendering of the 6 elements, (c) a 10 Hz HUD update timer (vendor's
`COCKPIT_HUD_UPDATE_MS = 100`).

## 2. Goals & Non-Goals

### Goals

- Render all 6 HUD elements specified in P15 spec §6.2:
  1. **Heading tape** (compass — already exists; preserve)
  2. **Altitude ladder** (left edge, 9 ticks, keyhole curve)
  3. **Speed tape** (right edge, 9 ticks, keyhole curve)
  4. **Pitch ladder** (center, vertical, ±30° visible)
  5. **Bank indicator** (arc above pitch ladder, ±60° visible)
  6. **VSI chevron** (right-edge, ft/min or m/s)
- 10 Hz update cadence (vendor `COCKPIT_HUD_UPDATE_MS = 100`)
- Read from chase-cam's existing resolved state (P14/P15 produces
  `{heading, pitch, range}` every 50 ms)
- Derive bank from tracked entity's `Quaternion` (Cesium native) and
  VSI from an 8-sample altitude history window (sliding, ~400 ms)
- Style-gate parity with existing heading compass (P9 R5): HUD chrome
  is hidden if globe style is gated by cockpit vision mode
- Tests + acceptance added to sp8 (bundle checks + probe extension)

### Non-Goals (deferred to P17+)

- SVS (Synthetic Vision System — terrain rendering on pitch ladder) —
  §6.3 in P15 spec
- TCAS (traffic collision avoidance) — §6.3 in P15 spec
- Replay system — §6.3 in P15 spec
- HUD customization (user resizes / hides individual elements) — P17
- Multi-monitor HiDPI drag feel — P15 §7 Open Question #1 (deferred)
- HUD keyboard shortcuts — none requested

## 3. Architecture

### 3.1 Data flow

```
            ┌─────────────────────────────────────────────┐
            │   chase-cam.ts (P14/P15, extend)            │
            │  ──────────────────────────────            │
            │   • 50 ms cadence (existing)                │
            │   • NEW getResolvedState():                 │
            │       { heading, pitch, range,              │
            │         bankRad (entity.quat→roll),         │
            │         vsiMps (altitude history window),   │
            │         altitudeM (latest sample) }         │
            │   • NEW altitudeHistory: float[4] (sliding) │
            │   • NEW bank from Quaternion (Cesium native)│
            └────────┬──────────────────────┬─────────────┘
                     │                      │
                     ▼                      ▼
            ┌────────────────────┐  ┌───────────────────────┐
            │ HUD timer 10Hz     │  │ flights.getTrackedInfo() │
            │ (new — interval    │  │ (existing P9 adapter)  │
            │  setInterval 100)  │  └─────────┬─────────────┘
            └────────┬───────────┘            │
                     │                        │
                     ▼                        ▼
            ┌─────────────────────────────────────────────┐
            │   instruments-mount.ts (extend, P9)         │
            │  ──────────────────────────────             │
            │   • ADD 3 fields to CockpitInstrumentFrame: │
            │       pitchRad, bankRad, vsiMps             │
            │   • Existing: heading, altitudeFt, speedKt, │
            │     compass, altitudeTicks, speedTicks      │
            └────────┬────────────────────────────────────┘
                     │
                     ▼
            ┌─────────────────────────────────────────────┐
            │  HudCockpitInstruments.tsx (extend in place)│
            │  ──────────────────────────────────────     │
            │  • Existing: heading compass SVG            │
            │  • NEW: altitude ladder SVG (~40 LoC)       │
            │  • NEW: speed tape SVG (~40 LoC)            │
            │  • NEW: pitch ladder SVG (~50 LoC)          │
            │  • NEW: bank indicator SVG (~30 LoC)        │
            │  • NEW: VSI chevron SVG (~40 LoC)           │
            └─────────────────────────────────────────────┘
```

### 3.2 Module ownership

| Module | Responsibility | Owner of |
|---|---|---|
| `chase-cam.ts` (extend) | Camera state + attitude derivation | `getResolvedState()`, bank math, altitude history |
| `cockpitMath.js` (existing P9 port) | Vendor ruler math | `compassDivisions`, `altitudeRulerTicks`, `speedRulerTicks` |
| `instruments-mount.ts` (extend) | Pure-computation adapter | `CockpitInstrumentFrame` assembly (adds 3 fields) |
| `HudCockpitInstruments.tsx` (extend) | React SVG rendering | All 6 elements |
| `cockpit-hud-tick.ts` (NEW) | 10 Hz timer + lifecycle | `setInterval(100)` wrapper, calls `update(info)` |

### 3.3 Why chase-cam owns attitude (not a new module)

Brainstorming surfaced three plausible ownership options; user chose
Approach 1 (chase-cam owns bank + VSI). Rationale:

- chase-cam already maintains the per-tick state for camera + tracked
  entity (50 ms cadence). Adding bank + VSI to the same tick is a 40-LoC
  extension to an existing file, not a new module.
- bank from `Quaternion` is a Cesium-native operation — it belongs near
  the code that already pulls entity state (`entity.position.getValue`,
  `entity.orientation`).
- VSI needs an altitude history buffer; that buffer is meaningless
  outside the chase-cam tick context.
- "Single source of truth for camera + attitude" matches the spec's
  "HUD reads chase-cam's `(heading, pitch, range)` outputs" framing.

### 3.4 What this PR does NOT touch

- **No DB migrations** — HUD reads `flights.getTrackedInfo()` + chase-cam state
- **No API additions** — no new REST endpoints, no new MCP tools
- **No env-var changes** — no new secrets, no new config keys
- **No vendor math changes** — `cockpitMath.js` is byte-stable from P9;
  bank + VSI are net-new (no vendor equivalent)
- **No style-gate policy changes** — HUD inherits P9 R5 behavior

## 4. Detailed Design

### 4.1 chase-cam.ts extension

#### 4.1.1 `getResolvedState()` (NEW)

```typescript
export interface ChaseCamResolvedState {
  heading: number | null;   // radians, 0=N
  pitch: number | null;     // radians, 0=horizon
  range: number | null;     // meters
  bankRad: number;          // radians, 0=level, +right
  vsiMps: number;           // m/s vertical speed (positive = up)
  altitudeM: number | null; // latest altitude sample
}

export interface ChaseCamHandle {
  // ... existing methods ...
  getResolvedState(): ChaseCamResolvedState | null;
}
```

Returns null if not initialized (no tick has run yet).

#### 4.1.2 `bankRad` derivation

```typescript
// Quaternion → heading/pitch/roll via Cesium HeadingPitchRoll
const hpr = Cesium.HeadingPitchRoll.fromQuaternion(entity.orientation);
const bankRad = hpr.roll;  // radians; positive = right bank
```

Edge cases:
- `entity.orientation === undefined` → bankRad = 0 (level)
- `entity.orientation` is a `Property` (not value) → call `.getValue(time)`
- `Quaternion → HeadingPitchRoll` throws on zero quaternion → catch, default 0

#### 4.1.3 `vsiMps` derivation

Sliding window of 8 altitude samples (~400 ms at 50 ms cadence).
A 200 ms window (4 samples) is too jittery for VSI; a 400 ms window
gives a stable reading without lag.

```typescript
const altitudeHistory: number[] = [];  // max length 8

// Each tick:
altitudeHistory.push(altitudeM);
if (altitudeHistory.length > 8) altitudeHistory.shift();

// Derive VSI: linear regression on (t, altitude) pairs
if (altitudeHistory.length < 2) {
  vsiMps = 0;  // warmup
} else {
  const dt = (altitudeHistory.length - 1) * 0.050;  // 50ms cadence
  vsiMps = (altitudeHistory[altitudeHistory.length - 1] - altitudeHistory[0]) / dt;
}
```

#### 4.1.4 LoC budget

| Change | LoC |
|---|---|
| `ChaseCamResolvedState` interface | 12 |
| `getResolvedState()` method | 6 |
| Quaternion → bank math (per-tick) | 8 |
| Altitude history buffer + VSI derivation | 12 |
| Tests (mock Cesium, verify state) | 30 |
| **Total** | **~70** |

### 4.2 instruments-mount.ts extension

#### 4.2.1 Frame extension

```typescript
export interface CockpitInstrumentFrame {
  // ... existing fields ...
  pitchRad: number;        // NEW: radians, 0=horizon
  bankRad: number;         // NEW: radians, 0=level
  vsiMps: number;          // NEW: m/s, positive = up
}
```

#### 4.2.2 Adapter signature

`mountCockpitInstruments` gains an optional `chaseCam?` dependency:

```typescript
export interface CockpitInstrumentDeps {
  viewer: { scene: { canvas?: unknown } };
  flights?: { getTrackedInfo(): CockpitTrackedInfo | null };
  chaseCam?: { getResolvedState(): ChaseCamResolvedState | null };
}

export function mountCockpitInstruments(
  deps: CockpitInstrumentDeps,
): InstrumentsHandle {
  // ...
  function compute(info, resolvedState) {
    // existing frame fields ...
    return {
      // ...
      pitchRad: resolvedState?.pitch ?? 0,
      bankRad: resolvedState?.bankRad ?? 0,
      vsiMps: resolvedState?.vsiMps ?? 0,
    };
  }
}
```

When `chaseCam` is not provided (or `getResolvedState()` returns null),
all 3 fields default to 0 — HUD renders neutral.

#### 4.2.3 LoC budget

| Change | LoC |
|---|---|
| `CockpitInstrumentDeps` interface | 10 |
| `pitchRad/bankRad/vsiMps` frame fields | 6 |
| `compute()` extension | 12 |
| `update()` signature (accept resolvedState) | 6 |
| Tests (frame assembly, deps handling) | 25 |
| **Total** | **~60** |

### 4.3 HudCockpitInstruments.tsx extension

#### 4.3.1 Layout

```
   ┌──────────────┬──────────────┬──────────────┐
   │              │  pitch ladder │              │
   │  altitude    │       ▼       │  speed tape  │
   │  ladder      │   [ATTITUDE]  │              │
   │  (left)      │       │       │  (right)     │
   │              │  bank ind.    │              │
   ├──────────────┴───┬────┴───────┴──────────────┤
   │   heading tape   │ VSI chevron (right edge) │
   │   (bottom center)│                          │
   └──────────────────┴──────────────────────────┘
```

CSS-grid layout (HUD already uses absolute positioning; SVG groups
position via `transform: translate(...)`).

#### 4.3.2 SVG element sketches

**Altitude ladder** (left, 9 ticks):
```jsx
<svg width="80" height="200" className="hud-cockpit-altitude-ladder">
  {frame.altitudeTicks.map(tick => (
    <g key={tick.slot} transform={`translate(0, ${tick.slot * 20})`}>
      <line x1={tick.major ? 0 : 10} x2="40" y1="0" y2="0" />
      {tick.major && <text x="50" y="4">{tick.label}</text>}
    </g>
  ))}
</svg>
```

**Speed tape** (right, 9 ticks) — symmetric.

**Pitch ladder** (center, vertical, ±30°):
```jsx
<svg width="200" height="300" viewBox="0 0 200 300">
  <g transform={`translate(100, 150) rotate(${frame.bankRad * 180 / Math.PI})`}>
    {[-30, -20, -10, 0, 10, 20, 30].map(pitch => (
      <g key={pitch} transform={`translate(0, ${pitch * 5 - frame.pitchRad * 180 / Math.PI * 5})`}>
        <line x1="-30" x2="30" y1="0" y2="0" />
        <text x="-35" y="4" textAnchor="end">{pitch}°</text>
        <text x="35" y="4">{pitch}°</text>
      </g>
    ))}
  </g>
</svg>
```

The whole ladder rotates by `bankRad` so the bank angle is visually
encoded — this is how real glass cockpits work.

**Bank indicator** (arc above pitch ladder, ±60°):
```jsx
<svg width="200" height="60">
  {[-60, -45, -30, -15, 0, 15, 30, 45, 60].map(deg => (
    <text key={deg} x={100 + deg * 1.5} y="40" textAnchor="middle">
      {deg === 0 ? '◆' : Math.abs(deg)}
    </text>
  ))}
  {/* Current bank marker */}
  <polygon points={`${100 + frame.bankRad * 180 / Math.PI * 1.5},20 ${100 + frame.bankRad * 180 / Math.PI * 1.5 - 5},30 ${100 + frame.bankRad * 180 / Math.PI * 1.5 + 5},30`} fill="cyan" />
</svg>
```

**VSI chevron** (right edge, scaled):
```jsx
<svg width="40" height="200">
  {frame.vsiMps !== 0 && (
    <polygon
      points={frame.vsiMps > 0
        ? `20,100 10,110 30,110`  // up chevron
        : `20,100 10,90 30,90`}   // down chevron
      fill={frame.vsiMps > 0 ? 'lime' : 'amber'}
    />
  )}
  <text x="20" y="160" textAnchor="middle">{Math.abs(frame.vsiMps * 196.85).toFixed(0)}</text>  {/* ft/min */}
</svg>
```

VSI unit: **m/s internally**, displayed as ft/min (matches vendor's
aviation convention).

#### 4.3.3 LoC budget

| Change | LoC |
|---|---|
| Imports (Cesium types not needed) | 4 |
| Existing heading compass (preserve) | 60 |
| Altitude ladder SVG | 40 |
| Speed tape SVG | 40 |
| Pitch ladder SVG | 50 |
| Bank indicator SVG | 30 |
| VSI chevron SVG | 40 |
| Style-gate wrapper | 10 |
| Tests (snapshot per element + state props) | 50 |
| **Total** | **~320** |

### 4.4 cockpit-hud-tick.ts (NEW)

10 Hz HUD update timer. Owns `setInterval(100)` lifecycle.

```typescript
export interface HudTickHandle {
  start(): void;
  stop(): void;
  isRunning(): boolean;
}

export function mountCockpitHudTick(
  instruments: InstrumentsHandle,
  chaseCam: { getResolvedState(): ChaseCamResolvedState | null },
  flights: { getTrackedInfo(): CockpitTrackedInfo | null },
  options?: { intervalMs?: number; viewer?: { isDestroyed(): boolean } },
): HudTickHandle {
  const intervalMs = options?.intervalMs ?? 100;  // matches vendor

  let handle: ReturnType<typeof setInterval> | null = null;

  function tick() {
    if (options?.viewer?.isDestroyed?.()) {
      handle && clearInterval(handle);
      handle = null;
      return;
    }
    const info = flights.getTrackedInfo();
    const resolved = chaseCam.getResolvedState();
    instruments.update(info, resolved);
  }

  return {
    start() {
      if (handle) return;
      handle = setInterval(tick, intervalMs);
    },
    stop() {
      if (handle) clearInterval(handle);
      handle = null;
    },
    isRunning: () => handle !== null,
  };
}
```

#### 4.4.1 LoC budget

| Change | LoC |
|---|---|
| `mountCockpitHudTick` impl | 50 |
| Tests (interval fires, viewer destroyed stops) | 30 |
| **Total** | **~80** |

### 4.5 Wiring in GlobeV2.tsx

Existing cockpit-active branch (P14/P15) already mounts chase-cam +
mouse-look. Add:

```typescript
if (cockpitStore.active) {
  const instruments = mountCockpitInstruments({ viewer, flights, chaseCam });
  const hudTick = mountCockpitHudTick(instruments, chaseCam, flights, { viewer });
  hudTick.start();
  // ...
  cleanup.push(() => {
    hudTick.stop();
    instruments.destroy();
  });
}
```

Net addition: ~15 LoC to GlobeV2.tsx.

### 4.6 Style gate (P9 R5)

Existing `style-gate.ts` gates the globe style picker, NOT the HUD
chrome. The HUD renders unconditionally while cockpit is active. This
is the current P9 behavior for the heading compass; §6.2 follows the
same pattern.

If a user picks a globe style while cockpit is active, the globe
undergoes the vision-mode intensity dance, but the HUD continues to
render (otherwise the operator loses reference data mid-flight).

## 5. Testing Strategy

### 5.1 Unit tests

| Module | Test surface | Tests |
|---|---|---|
| `chase-cam.ts` | `getResolvedState()` returns new fields | 4 |
| `chase-cam.ts` | bank from valid Quaternion | 2 |
| `chase-cam.ts` | bank defaults to 0 when no orientation | 2 |
| `chase-cam.ts` | altitude history window warmup → VSI derivation | 4 |
| `instruments-mount.ts` | frame includes `pitchRad/bankRad/vsiMps` | 3 |
| `instruments-mount.ts` | `chaseCam` deps wiring (null → defaults) | 3 |
| `cockpit-hud-tick.ts` | `start()` triggers ticks | 2 |
| `cockpit-hud-tick.ts` | `stop()` cancels interval | 2 |
| `cockpit-hud-tick.ts` | `viewer.isDestroyed()` stops tick | 1 |
| **Total** | | **~23** |

### 5.2 SVG / React tests

| Component | Test surface | Tests |
|---|---|---|
| `HudCockpitInstruments` | renders 6 elements given full frame | 1 |
| `HudCockpitInstruments` | altitude ladder renders 9 ticks | 1 |
| `HudCockpitInstruments` | speed tape renders 9 ticks | 1 |
| `HudCockpitInstruments` | pitch ladder rotates by bankRad | 1 |
| `HudCockpitInstruments` | bank indicator marker position | 1 |
| `HudCockpitInstruments` | VSI chevron direction (up/down) | 2 |
| `HudCockpitInstruments` | null frame → dashes / neutral | 1 |
| **Total** | | **~8** |

### 5.3 Existing tests (must not regress)

- 133 cockpit tests (P14/P15 baseline) — all green
- 625 full console tests — all green
- 95 cockpit dir baseline → 95 + 23 + 8 = **~126 tests** after P16

### 5.4 sp8 probe extension

Add a probe step that:

1. Mounts cockpit (already exists in probe)
2. Selects a tracked aircraft (already exists in probe)
3. Waits 250 ms (so altitude history fills + VSI derives)
4. Reads `__gevInstrumentsFrame` (new global, parallel to `__gevViewer`)
5. Asserts:
   - `frame.headingLabel` is a 3-digit string
   - `frame.altitudeLabel` is a number with commas
   - `frame.speedLabel` matches `formatSpeedRulerTick` shape
   - `frame.pitchRad` is a finite number
   - `frame.bankRad` is a finite number (likely 0 if no orientation)
   - `frame.vsiMps` is a finite number

## 6. Acceptance Criteria

### 6.1 Functional

- All 6 HUD elements render when cockpit is active and a tracked
  aircraft is selected
- HUD updates at 10 Hz (no jank, no missed ticks under normal load)
- No tracked entity → neutral state (dashes, 0s)
- Quaternion without orientation → bank = 0
- < 4 altitude samples → VSI = 0 (warmup)
- viewer destroyed → HUD timer stops on next tick
- Style-gate (P9 R5) does not affect HUD chrome

### 6.2 Test/Acceptance (sp8)

- **+6 bundle checks** (vendor-port trio analog):
  - `mountCockpitInstruments` accepts `chaseCam` deps
  - `mountCockpitHudTick` exported
  - `pitchRad/bankRad/vsiMps` in `CockpitInstrumentFrame` type
  - `chase-cam.getResolvedState()` returns the new fields
  - Quaternion → bank math bundled
  - Altitude history window bundled

- **+1 probe check** (frame fields populated with real data):
  - mount cockpit → select tracked aircraft → wait 250ms →
    `__gevInstrumentsFrame` returns frame with all 9 fields finite

- All baseline checks (67 sp8, 49 sp6, 16 sp7, 19 sp3) remain green

### 6.3 Performance budget

- 10 Hz tick = 100 ms cadence
- Each tick: 1 `getResolvedState()` (cheap) + 1 `getTrackedInfo()`
  (cheap) + 1 `instruments.update()` (pure-function frame assembly) +
  1 React setState (batched by React 18)
- Total per-tick CPU: <1 ms (frame assembly is just JS object literal)
- React re-render: <2 ms (pure SVG transform)
- **No measurable frame-time impact on the 50 ms chase-cam cadence**

## 7. Risks & Mitigations

| Risk | Severity | Mitigation |
|---|---|---|
| Quaternion → HeadingPitchRoll throws on zero quaternion | Med | try/catch, default bank = 0; tested |
| `flights.getTrackedInfo()` returns null intermittently | Low | P9 already handles; pitch/bank/VSI default to 0 |
| VSI flickers when altitude history window slides | Low | 4-sample window + 50 ms cadence = visible smoothing; tested |
| Pitch ladder rotates wildly when bankRad changes fast | Low | Vendor does same; cockpit context caps bank rate at 60°/s by SVS convention |
| HUD timer leaks if cleanup not called | Med | `mountCockpitHudTick` returns handle; `cleanup.push(stop)` in GlobeV2; tested |
| Cesium `HeadingPitchRoll.fromQuaternion` is heavy | Low | Called once per 50 ms tick (chase-cam), not per render |
| Bank indicator overlaps heading tape visually | Med | CSS-grid z-order + pointer-events: none; tested |
| SVG re-renders entire HUD on every 100 ms tick | Low | React 18 batches setState; no perceptible jank |

## 8. Open Questions / Deferred

1. **HUD customization** (P17) — user can show/hide individual elements
2. **SVS / TCAS / replay** — §6.3 in P15 spec, separate sub-project
3. **Bank rate limiter** — SVS convention caps bank at 60°/s; do we
   enforce? P17 if needed.

## 9. References

- P15 spec §6.2 (this spec's parent intent):
  `docs/superpowers/specs/2026-09-21-gev-p15-cockpit-mouse-look-design.md`
  lines 461–466
- Vendor reference:
  `/Volumes/TBU/Github/gods-eye-view/src/ui/cockpitInstruments.js`
  (260 LoC; speed/altitude/heading only — pitch/bank/VSI are net-new)
- Vendor math port (existing P9):
  `/Volumes/TBU/Workspace/IntelHub/console/gev-engine/src/cockpitMath.js`
- Existing pure-computation adapter (existing P9):
  `/Volumes/TBU/Workspace/IntelHub/console/src/gev-visual/cockpit/instruments-mount.ts`
- Existing React component (P9, 212 LoC, heading only):
  `/Volumes/TBU/Workspace/IntelHub/console/src/globe-hud/HudCockpitInstruments.tsx`
- Vendor cadence constant:
  `/Volumes/TBU/Github/gods-eye-view/src/ui/cockpitPresentation.js:14`
  `export const COCKPIT_HUD_UPDATE_MS = 100;`
- Chase-cam (P14/P15, owner of `getResolvedState`):
  `/Volumes/TBU/Workspace/IntelHub/console/src/gev-visual/cockpit/chase-cam.ts`
- Style gate (P9 R5, not modified):
  `/Volumes/TBU/Workspace/IntelHub/console/src/gev-visual/cockpit/style-gate.ts`
