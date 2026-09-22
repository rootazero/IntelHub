# GEV §6.3 — SVS / TCAS / Replay (Spec, Decision Document)

> Deferred from P15 spec §6.3 + P16 spec §8. Per user direction
> (2026-09-23), this PR is **decision only — no code**. Each of the
> three features (SVS, TCAS, replay) is its own sub-project; this
> doc captures the open product decisions and lays the technical
> ground for whoever picks up the implementation.

## 1. Why this spec exists now

Three cockpit-adjacent features have been deferred across two PRs
because they require product-level decisions that don't fit the
"ship a concrete artifact" cadence:

- **SVS (Synthetic Vision System)** — terrain rendering on/around
  the pitch ladder
- **TCAS (Traffic Collision Avoidance System)** — proximity
  warnings against other aircraft
- **Replay** — flight playback system

Each is independent. Each is ~500-1500 LoC of console work +
potential hub-core additions. None can ship without product
clarification; this spec is the clarification.

## 2. Background & dependency check

### What's already in place (the cockpit we can extend)

- **chase-cam** (P14) — operator camera glued to tracked aircraft
- **mouse-look + wheel-zoom** (P15) — operator input
- **HUD avionics** (P16) — heading / altitude / speed / pitch /
  bank / VSI, all driven from chase-cam's resolved state at 10 Hz
- **element visibility** (P17) — user can hide individual HUD
  elements; state persists
- **instruments-mount** — pure-computation adapter wrapping vendor
  math into a `CockpitInstrumentFrame`
- **cockpit store** — small reducer for cockpit overlay lifecycle
  (active / trackedId / visionMode / briefingPaused / hidden /
  elementVisibility)
- **cockpit-input fleet** — local-only context (cockpit has no
  network I/O today)

### What's NOT in place

- **No terrain data** — Cesium has its own globe terrain but no
  cockpit-specific terrain cache, no obstacle database, no
  elevation-API call surface
- **No traffic/proximity data** — `flights` source adapter ships
  tracked aircraft (P9/P12) but the catalog has no
  neighborhood-of-tracked-aircraft view, no closure-rate
  computation, no proximity envelope
- **No replay infrastructure** — there is no state recorder, no
  capture buffer, no playback clock, no scrubber UI. The vendor
  engine has no replay module. Hub-core has no replay endpoint
- **No collision detection** — no pair-wise distance / closure
  computation; no SVS/TCAS alerts table; no audio subsystem in
  console (visual-only today)

### Vendor engine coverage

The vendored `gods-eye-view` engine has NO SVS/TCAS/replay
modules. Files surveyed (`cockpitMath.js`, `cockpitTracking.js`,
`cockpitUtilityLayout.js`, `cockpitVisionPolicy.js`,
`cockpitCloudEffects.js`, `cockpitSignalFocus.js`) cover math,
utility layout, vision policy, cloud effects, and signal focus —
none extend to terrain rendering or proximity warning.

This means §6.3 is a clean-sheet sub-project on the console side,
with potential hub-core additions for any data layer that
requires network calls.

## 3. SVS — Synthetic Vision System

### 3.1 What SVS is

In a real glass cockpit, SVS paints a 3D representation of
terrain + obstacles onto the PFD so the pilot can see "through"
clouds. Typically layered onto the pitch ladder area with a
synthetic horizon (green/brown ground vs sky), terrain contours,
and obstacle polygons.

### 3.2 Open product decisions

The following decisions gate the SVS scope. **No code should
until the user weighs in.**

#### D-SVS-1: Rendering approach

- **A. 2D HUD overlay** (terrain contour lines drawn on the
  existing pitch ladder SVG, like a "sketch" of the terrain).
  Pro: minimal LoC (~150), no new dependencies, reuses the
  existing P16 pitch ladder as the canvas. Con: low fidelity
  (lines, not real terrain).
- **B. 3D synthetic vision via Cesium primitives** (a separate
  WebGL canvas layered behind/over the HUD; uses Cesium
  `Globe.depthTestAgainstTerrain` + custom terrain provider).
  Pro: high fidelity (real terrain, real obstacles). Con:
  ~1500 LoC, dual-canvas complexity, Cesium-specific.
- **C. Cesium ion / open data terrain overlay** (subscribe to
  Cesium World Terrain or equivalent, paint 2D HUD on top).
  Pro: production-grade fidelity with minimal math. Con:
  external dependency (Cesium ion key), network cost.

**Recommendation**: **A** for v1 (minimal, fast, validates the
concept). **C** deferred to a future major if fidelity proves
insufficient. **B** rejected (over-engineered for the use case).

#### D-SVS-2: Terrain data source

Where does the elevation data come from?

- **A. Hub-core terrain API** — new `/api/v1/tiles/{z}/{x}/{y}`
  endpoint backed by a terrain raster (SRTM, GMTED, etc.).
- **B. Cesium-native terrain** — use the engine's own globe
  terrain (whatever Cesium ion provides); SVS reads from
  `viewer.scene.globe.terrainProvider`.
- **C. Open-data CDN** — direct fetch from AWS Terrain Tiles
  or similar public sources, cached in browser IndexedDB.

**Recommendation**: **B** (Cesium-native, no extra hub-core work,
no API key required). Fall back to **A** if Cesium ion proves
unreliable.

#### D-SVS-3: Activation trigger

When does SVS turn on?

- **A. Always** — SVS renders whenever cockpit is active.
- **B. Below 3000 ft AGL** — only on approach / low altitude.
- **C. Manual toggle** — `◇ SVS` button in cockpit chrome
  (mirrors `◇ ELEMENTS` from P17).
- **D. Vision mode coupling** — only when vision mode is `nvg`
  or `thermal` (when "seeing through clouds" is most useful).

**Recommendation**: **C** + **D** (manual toggle + NVG/FLIR
default-on). Mirrors existing cockpit conventions; doesn't
clutter the HUD when not in use.

#### D-SVS-4: Visual style

- **A. Wireframe terrain** (sketch-style contour lines on
  pitch ladder).
- **B. Solid terrain polygon** (filled ground/sky colors,
  real obstacle shading).
- **C. Hybrid** (wireframe + critical obstacles solid).

**Recommendation**: **A** for v1 (matches "minimal LoC" of
D-SVS-1=A). Style notes in vendor's `cockpitVisionPolicy.js`
show NVG uses inverse colors; SVS could mirror.

### 3.3 Effort estimate

Assuming recommendations A/B/C/A:

- T1: Cesium-native terrain sampler (reads from
  `viewer.scene.globe.terrainProvider`, samples N points around
  the agent's projected position). ~80 LoC.
- T2: Pitch-ladder SVG extension (overlay wireframe on existing
  pitch ladder, scaled by altitude AGL). ~120 LoC.
- T3: Cockpit store extension (SVS toggle, persistence,
  activation trigger). ~50 LoC.
- T4: Tests + acceptance. ~150 LoC.
- Total: **~400 LoC** in one PR, plus optional 2 follow-ups if
  fidelity proves insufficient.

## 4. TCAS — Traffic Collision Avoidance System

### 4.1 What TCAS is

In real aviation, TCAS watches other aircraft within ~40 nm and
emits visual + aural advisories ("TRAFFIC, TRAFFIC") when
separation shrinks below thresholds based on altitude.

### 4.2 Open product decisions

#### D-TCAS-1: Source of "other aircraft"

Where do the targets come from?

- **A. Hub-core flights catalog** — query
  `/api/v1/flights?near=icao24&radius_nm=N` from hub-core PG.
  Pro: already exists, real ADS-B data. Con: 1-2s latency.
- **B. Direct OpenSky/ADSBX feed** — bypass hub-core, fetch
  from upstream directly. Pro: lowest latency. Con: rate-limit
  hazard, breaks the existing "hub-core is the only egress"
  pattern.
- **C. Local neighborhood cache** — subscribe to hub-core's
  `geo_events` topic (where flights are emitted as they appear
  on the globe) and maintain a sliding window of nearby
  aircraft. Pro: sub-second freshness, reuses existing event
  stream. Con: only sees aircraft already on the globe (won't
  see off-screen targets until they enter the camera frustum).

**Recommendation**: **A** (hub-core PG, add a `near=`
parameter to the existing `/api/v1/flights` endpoint). The
`opensky_live` collector already populates the `geo_events`
table; a `/flights?near=` query is a 30-LoC hub-core addition.
**C** as a follow-up optimization.

#### D-TCAS-2: Detection radius

- **A. 5 nm** — short-range, advisory only.
- **B. 10 nm** — mid-range, the realistic TCAS TA (Traffic
  Advisory) envelope.
- **C. 40 nm** — full TCAS range, but high UI clutter.

**Recommendation**: **A** for v1 (5 nm). Pilot mental load
stays low; visual clutter minimal. **B** as a tunable knob.

#### D-TCAS-3: Visual representation

- **A. Target diamonds** — small filled squares in the HUD
  bearing the target, colored by separation (white=monitor,
  amber=caution, red=warning).
- **B. Vertical tape extension** — tiny altitude bars on the
  altimeter indicating the target's relative altitude.
- **C. Both** (diamonds + altitude bars, like real TCAS II).

**Recommendation**: **C** if scope allows, **A** otherwise.

#### D-TCAS-4: Aural warnings

- **A. None** (visual-only).
- **B. Web Audio API beeps** ("traffic, traffic" synthesized
  tones).
- **C. Pre-recorded audio** (real TCAS samples).

**Recommendation**: **A** for v1 (no audio deps). **B** as a
follow-up if users request.

#### D-TCAS-5: Threat criteria

What makes a target "advisory" vs "warning"?

- **A. Distance-only** (≤ 1 nm = red; ≤ 2 nm = amber).
- **B. Distance + closure rate** (account for closing speed;
  closing ≥ 500 fpm at ≤ 3 nm = red).
- **C. Distance + closure + altitude differential** (full TCAS
  logic; closing fast at co-altitude = red).

**Recommendation**: **B** (distance + closure rate). The math
is ~20 LoC and materially more useful than distance-only.
**C** is a complexity bump not justified by v1 scope.

### 4.3 Effort estimate

Assuming recommendations A/A/C/A/B:

- T1: Hub-core `?near=` parameter on `/api/v1/flights`. ~30
  LoC hub-core + ~30 LoC tests.
- T2: Console `mountCockpitTcas` adapter (subscribe to the
  near query at 1 Hz, compute distance + closure rate,
  classify threat). ~150 LoC.
- T3: HUD extension (target diamonds + altitude bars on
  altimeter). ~120 LoC.
- T4: Cockpit store extension (TCAS toggle, persistence). ~50
  LoC.
- T5: Tests + acceptance. ~200 LoC.
- Total: **~550 LoC** in one PR (50 hub-core + 500 console).

## 5. Replay — flight playback

### 5.1 What replay is

A timeline scrubber UI that lets the operator replay a past
flight segment: state at time T (position, altitude, speed,
heading, cockpit HUD state, surrounding geo_events). Useful for
post-flight debrief, incident review, training.

### 5.2 Open product decisions

#### D-REPLAY-1: What state to record

- **A. Just the cockpit frame** (heading / pitch / bank /
  altitude / speed / VSI / trackedId). Pro: tiny (a few KB per
  minute). Con: no context (no weather, no other aircraft at
  that moment).
- **B. Cockpit frame + nearby geo_events** (everything that
  appeared on the operator's screen at time T). Pro: full
  visual replay. Con: 100-500 KB per minute, harder to scrub.
- **C. Full hub-core snapshot** (every signal collection at T).
  Pro: research-grade replay. Con: GB per minute, not practical
  for browser.

**Recommendation**: **A** for v1 (cockpit frame only). The
cockpit IS the cockpit — replaying it without visual chrome
is a coherent minimum-viable product. **B** deferred.

#### D-REPLAY-2: Where to store

- **A. Browser IndexedDB** (local). Pro: no infra change,
  infinite storage. Con: per-user, no cross-device sync.
- **B. Hub-core PG** (server). Pro: cross-device, auditable.
  Con: hub-core storage grows continuously; archival policy
  needed.
- **C. Both** (local cache + server sync). Pro: best of both.
  Con: complexity.

**Recommendation**: **A** for v1. Local-only. Pilot reviews
their own debrief, doesn't need cross-device. **B** deferred.

#### D-REPLAY-3: Time range

- **A. Last session only** (~30 minutes default). Pro: small
  buffer. Con: lost on browser refresh.
- **B. Last 24 hours** (rolling). Pro: useful for next-day
  debrief. Con: ~50 MB IndexedDB footprint.
- **C. User-selected segments** (record-on-demand). Pro:
  bounded storage. Con: pilot must remember to record.

**Recommendation**: **C** (user-selected segments, default
~10 minutes, max ~2 hours). Mirrors the "dashcam" mental
model. **B** deferred.

#### D-REPLAY-4: Playback controls

- **A. Play / pause / speed** (0.5× / 1× / 2× / 4×).
- **B. Scrubber** (timeline drag to set time).
- **C. Loop** (repeat on reaching end).
- **D. Bookmarks** (named markers for "interesting moments").

**Recommendation**: **A** + **B** for v1. **C** + **D**
deferred.

#### D-REPLAY-5: UI placement

- **A. Bottom strip** (like a video player timeline).
- **B. Floating popover** (toggle from cockpit chrome).
- **C. Modal full-screen** (separate page route).

**Recommendation**: **B** (popover from a `◇ REPLAY` button in
cockpit chrome, mirroring `◇ ELEMENTS` from P17).

### 5.3 Effort estimate

Assuming recommendations A/A/C/A+B/B:

- T1: Recorder (cockpit frame → IndexedDB every 50 ms during
  recording). ~150 LoC.
- T2: Playback engine (read IndexedDB, interpolate, drive
  cockpit-store at scrub position). ~200 LoC.
- T3: Replay popover UI + scrubber. ~200 LoC.
- T4: Cockpit store extension (recording / playback state). ~50
  LoC.
- T5: Tests + acceptance. ~200 LoC.
- Total: **~800 LoC** in one PR (console-only).

## 6. Dependency graph

The three features are largely orthogonal:

```
SVS       ──> Cesium terrain API
              cockpit-store (visibility toggle)

TCAS      ──> hub-core /api/v1/flights?near=
              console-side distance + closure math
              cockpit-store (TCAS toggle)

Replay    ──> IndexedDB
              cockpit-store (recording/playback state)
              scratch layer on top of cockpit-store

No shared infrastructure.
```

A single PR can ship any one of the three independently. If all
three ship together, the cumulative LoC is ~1750 + 50 hub-core;
best done as a 3-PR sequence (one per feature) to keep each
reviewable in <800 LoC.

## 7. Recommended next step

Per user direction, **no code this PR**. The next action is a
**decision conversation** with the user covering:

1. Approve / reject each §3.2, §4.2, §5.2 recommendation
2. Lock the scope of the first feature to ship (SVS, TCAS, or
   replay — pick one to start)
3. Schedule the first feature's implementation PR

Once decisions are locked, the chosen feature's spec gets a
follow-up section appended here with the final decisions, and
implementation begins in a new worktree.

## 9. Deferred (still)

- The other two §6.3 features (whichever wasn't picked first)
- CSS positioning polish for SVG groups (separate spec — see
  follow-up doc)

---

## 10. Replay — Locked Decisions (2026-09-23)

User delegated selection based on "high cohesion, low coupling"
principle. Picked **Replay** for first §6.3 ship because:

- **Highest cohesion**: single bounded context — "cockpit frame
  lifecycle" (record → store → playback). All complexity
  contained in one domain.
- **Lowest coupling**: console-only, IndexedDB-native, only
  touches the existing cockpit-store seam. No new dependencies,
  no hub-core changes, no Cesium-specific leaking.
- **TCAS rejected for first ship**: cross-cuts hub-core +
  console (multi-layer coupling); broader scope dilutes
  cohesion. Better as a focused second PR.
- **SVS rejected for first ship**: high coupling to Cesium
  internals (terrain provider API leaks engine-specific
  concerns into HUD layer).

### 10.1 Locked recommendations (from §5.2)

- **D-REPLAY-1 → A**: just the cockpit frame (small payload,
  coherent MVP).
- **D-REPLAY-2 → A**: browser IndexedDB (local-only, no infra
  change).
- **D-REPLAY-3 → C**: user-selected segments (dashcam mental
  model, default ~10 min, max ~2 hours).
- **D-REPLAY-4 → A + B**: play / pause / speed + scrubber (the
  baseline; loop + bookmarks deferred).
- **D-REPLAY-5 → B**: popover from `◇ REPLAY` button in cockpit
  chrome (mirrors `◇ ELEMENTS` from P17).

### 10.2 Module breakdown (high cohesion, low coupling)

```
src/gev-visual/cockpit/
├── replay-recorder.ts    # Recorder adapter (mount + IndexedDB IO + sampling)
├── replay-player.ts      # Player adapter (mount + scrub + speed control)
└── replay-types.ts       # ReplaySegment, ReplayFrame types (no shadow)

src/gev-visual/cockpit/cockpit-store.ts
└── EXTEND: 4 new actions (start/stop recording, start/stop
   playback) + 2 new fields (recordingSegmentId, playbackState)

src/globe-hud/
└── HudCockpitReplay.tsx  # UI: popover with record/play/scrubber/speed
```

Boundary discipline:
- Recorder/Player are pure adapters (mount/destroy pattern, like
  `mountCockpitInstruments`). They never touch DOM.
- HUD reads cockpit-store via the same seam as P17's
  `HudCockpitElementSwitch` (subscribe + dispatch).
- No new dependencies — IndexedDB is browser-native.
- Tests use `fake-indexeddb` (vitest-only) so they run in jsdom.

### 10.3 Effort estimate (matches §5.3)

- T1: store extension (~50 LoC)
- T2: recorder (~250 LoC + ~6 tests)
- T3: player (~250 LoC + ~6 tests)
- T4: HUD popover (~200 LoC + ~8 tests)
- T5: source contracts + sp8 + frame wiring (~100 LoC + ~4 tests)
- Total: ~850 LoC in 5 commits.