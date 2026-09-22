# GEV P18 — Cockpit SVG Positioning (Spec)

> Follows P17 (element visibility, merged + pushed) on a new
> branch. P18 is the long-deferred CSS positioning polish —
> originally deferred from P16 spec §8. **Orthogonal to P17**:
> P17 was about *whether* an element renders; P18 is about
> *where* it renders when present.

## 1. Background

P16 spec §4.3.1 envisioned a specific layout for the 8 HUD
elements (4 groups of 2-3 elements each, arranged in a
CSS-grid with overlap between pitch ladder + bank arc). What
landed in P16 + P17 was simpler: a single `.hud-cockpit-
instruments` flex container with `gap: 18px`, centered on the
viewport, with all 8 SVGs in document flow.

The simplification works — the layout degrades gracefully when
P17 hides elements — but it doesn't match the spec sketch:
- All elements are side-by-side at the same vertical center
- No overlap between pitch ladder and bank indicator
- Altitude ladder doesn't sit at the left edge with speed tape
  at the right edge (they're inline like the rest)
- P9 elements (Compass/Altimeter/SpeedRuler) have a gauge
  wrapper with background+border; P16 elements (AltitudeLadder/
  SpeedTape/PitchLadder/BankIndicator/VsiChevron) do not,
  giving the row an inconsistent visual rhythm

This PR closes that gap.

## 2. Goals & Non-Goals

### Goals

- Match the P16 spec §4.3.1 layout sketch (3-column grid +
  bottom row for compass + VSI chevron)
- Visual consistency: P16 elements get a subtle gauge wrapper
  matching P9 (lighter than P9 to avoid clutter — no border,
  just background + slight padding)
- P17 visibility stays interaction: hiding an element still
  removes it cleanly; remaining elements shift to occupy the
  freed space (no layout holes)
- Responsive: layout holds on viewports from 1280×720
  (desktop minimum) to 2560×1440 (test VM)
- Pointer-events: SVG groups remain non-interactive (cockpit
  state is read-only on the HUD)
- No regressions: 16 P16 + 17 P17 console tests still pass

### Non-Goals

- SVS / TCAS / replay (§6.3) — separate sub-project
- Per-element resize / drag — UX feature, deferred
- Per-element theme customization — out of scope
- Mobile / touch layouts — cockpit is desktop-only

## 3. Architecture

### 3.1 Layout grid

Two-row CSS grid, columns flex-based:

```
.hud-cockpit-instruments {
  display: grid;
  grid-template-areas:
    "altitudeLadder   pitchBank   speedTape"
    "compass          compass      vsiChevron";
  grid-template-columns: 100px 1fr 100px;
  grid-template-rows: 200px 120px;
  gap: 12px;
  position: absolute;
  top: 50%;
  left: 50%;
  transform: translate(-50%, -50%);
  pointer-events: none;
}
```

Each element is placed in its named grid area:

```css
.hud-cockpit-pitch-ladder    { grid-area: pitchBank; }
.hud-cockpit-bank-indicator  { grid-area: pitchBank; }
.hud-cockpit-altitude-ladder { grid-area: altitudeLadder; }
.hud-cockpit-speed-tape      { grid-area: speedTape; }
.hud-cockpit-compass         { grid-area: compass; }
.hud-cockpit-altimeter       { grid-area: pitchBank; }  /* over pitch ladder center */
.hud-cockpit-speed           { grid-area: pitchBank; }  /* over pitch ladder right */
.hud-cockpit-vsi-chevron     { grid-area: vsiChevron; }
```

The grid-area assignment is the key insight: by stacking
multiple elements in the same grid area (PitchLadder +
BankIndicator + Altimeter + Speed all share `pitchBank`), they
overlap rather than compete for horizontal space. CSS
`grid-area` with the same name = overlap; z-index then decides
rendering order.

### 3.2 Z-index layering (back to front)

```
z-index 0:  .hud-cockpit-pitch-ladder    (the deepest layer)
z-index 1:  .hud-cockpit-bank-indicator  (sits over pitch top)
z-index 2:  .hud-cockpit-altitude-ladder (left edge)
z-index 2:  .hud-cockpit-speed-tape      (right edge)
z-index 3:  .hud-cockpit-altimeter       (over pitch left)
z-index 3:  .hud-cockpit-speed           (over pitch right)
z-index 4:  .hud-cockpit-vsi-chevron     (right edge, top)
z-index 5:  .hud-cockpit-compass          (bottom center, top)
```

The Compass (bottom-center heading tape) sits highest so it's
always legible; the pitch ladder is the deepest because it's
the largest and most informational.

### 3.3 Visual consistency for P16 elements

P9 elements have a `.hud-cockpit-gauge` wrapper class with:
- background: `rgba(3, 5, 9, 0.55)`
- backdrop-filter: `blur(4px)`
- border: `1px solid rgba(154, 164, 178, 0.25)`
- border-radius: `10px`
- padding: `10px`

P16 elements get a lighter `.hud-cockpit-gauge-light` wrapper:
- background: `rgba(3, 5, 9, 0.35)`  (half opacity)
- backdrop-filter: `blur(3px)`
- border: `none`  (no border, to avoid clutter)
- border-radius: `8px`
- padding: `6px`

The half-opacity + no-border keeps P16 elements visually
present but quieter than P9 — preserves the "P9 is canonical,
P16 are additions" hierarchy.

### 3.4 P17 visibility interaction

The grid areas are preserved even when the element inside is
hidden (P17). The grid cell collapses to zero size when empty:

```css
.hud-cockpit-instruments:has(> :empty) {
  /* not used — we don't render empty wrappers */
}
```

Concrete: when an element is hidden via P17, we render `null`
in the JSX (per P17 T3 design). The grid cell shrinks to 0
because it has no children; adjacent cells stay anchored. No
layout gymnastics needed.

For Altimeter + SpeedRuler (which currently share `pitchBank`
with PitchLadder + BankIndicator), if both are hidden, the
`pitchBank` cell is still occupied by PitchLadder. If only
Altimeter is hidden, SpeedRuler + PitchLadder + BankIndicator
all stack in the same area. No issue.

### 3.5 Responsive scaling

The grid uses fixed pixels for column widths (100px each side
column + 1fr center), and fixed row heights (200px + 120px).
On smaller viewports, the center `1fr` column absorbs the
compression; the side columns stay readable.

```css
@media (max-height: 800px) {
  .hud-cockpit-instruments {
    grid-template-rows: 160px 100px;
    transform: translate(-50%, -50%) scale(0.85);
  }
}
```

The `transform: scale()` shrinks the entire cluster uniformly on
short viewports — preserves layout math in CSS.

## 4. Behavior

### 4.1 Default state

The 8 elements are arranged per the grid above. The cluster
sits at viewport center. Side columns (altitude + speed + VSI)
flank the center; bottom row has compass + VSI chevron.

### 4.2 With P17 visibility

When the user hides elements via `◇ ELEMENTS`:

- Hiding `compass` → bottom-center cell collapses; remaining
  elements stay in their cells
- Hiding `pitchLadder` → center cell still has bank indicator
  (still readable)
- Hiding `altitudeLadder` → left column collapses to 0 width
  (center + right stay balanced via 1fr)
- Hiding everything → cluster collapses to a 0×0 dot (cockpit
  chrome around it stays functional)

### 4.3 Edge cases

- **All hidden**: cluster collapses; P18 doesn't add a "show
  all" button (that's P17's `◇ ELEMENTS` responsibility)
- **Viewport too small** (< 800px height): scale(0.85) kicks
  in. Below 600px height is unsupported (cockpit mode itself
  is desktop-only per the existing convention)
- **High-DPI displays**: SVG `viewBox` ensures scaling is
  crisp at any DPI

## 5. Source contracts

Add 3 new contracts in `source-contracts.test.ts`:

- **c10**: `.hud-cockpit-instruments` uses `display: grid` (not
  the current flex). Catches accidental reversion to the
  P16/P17 layout if a future refactor pulls in the old CSS
- **c11**: `.hud-cockpit-pitch-ladder` and `.hud-cockpit-bank-
  indicator` share the `pitchBank` grid area. Catches accidental
  split into separate areas (would break the spec sketch)
- **c12**: P16 elements have a `.hud-cockpit-gauge-light`
  wrapper class. Catches accidental omission of visual
  consistency treatment

## 6. Acceptance

### sp8 (console)

Add 4 new bundle checks:

- `hud-cockpit-instruments` grid layout in dist (look for
  `grid-template-areas` literal)
- `pitchBank` grid area name in dist
- `.hud-cockpit-gauge-light` class in dist
- z-index layering sequence in source (1-5)

### console tests (vitest)

No new tests needed if grid layout is purely CSS. If we add
testid changes (e.g., wrapping each P16 element in a div
with the light class), the existing `HudCockpitInstruments`
tests still pass because they query by `data-testid` on the
inner element, not the wrapper.

## 7. Implementation plan

~150 LoC, 1 task:

- **T1**: Update `hud.css` with the grid layout, gauge-light
  styles, z-index layering, responsive scaling. Modify
  `HudCockpitInstruments.tsx` to wrap each P16 SVG in a
  `.hud-cockpit-gauge-light` div (preserves testids on inner
  SVGs). Add 3 source contracts. Add 4 sp8 bundle checks.
  Verify P16 + P17 console tests still pass.

No new modules, no new dependencies.

## 8. Deferred (still)

- §6.3 SVS / TCAS / replay — separate sub-project, product
  decision still required (see companion spec
  `2026-09-23-gev-section-6-3-svs-tcas-replay-design.md`)
- Per-element resize / drag
- Per-element theme customization
- Mobile / touch layouts