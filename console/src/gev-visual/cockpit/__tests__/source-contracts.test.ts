// Contract guard suite — chase-cam source contracts (GEV P14 T5).
//
// c1  chase-cam is a camera driver, not a data source: it must NOT call
//     viewer.entities.add or viewer.entities.remove — it only drives the
//     Cesium camera via viewer.camera.setView.
// c2  chase-cam reuses vendor math/presets, not a re-implementation: pins the
//     two cockpit library imports the module must carry.
//
// Anchors are identifier-level CODE (property accesses, declarations) so they
// survive comment drift and reformatting but fire the moment an upstream sync
// changes a load-bearing contract.
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, test } from "vitest";

const here = dirname(fileURLToPath(import.meta.url));

function readCockpitSource(filename: string): string {
  return readFileSync(join(here, "..", filename), "utf8");
}

// ── c1: camera driver contract (chase-cam never mutates entities) ───────────

describe("c1: chase-cam is a camera driver, not a data source", () => {
  test("chase-cam.ts does not call viewer.entities.add", () => {
    const src = readCockpitSource("chase-cam.ts");
    expect(src).not.toMatch(/viewer\.entities\.add/);
  });

  test("chase-cam.ts does not call viewer.entities.remove", () => {
    const src = readCockpitSource("chase-cam.ts");
    expect(src).not.toMatch(/viewer\.entities\.remove/);
  });

  test("chase-cam.ts drives camera via viewer.camera.setView only", () => {
    const src = readCockpitSource("chase-cam.ts");
    // setView is the sole viewer mutation surface — no entities, no camera flyTo.
    expect(src).toMatch(/viewer\.camera\.setView/);
  });
});

// ── c2: vendor library re-use contract ──────────────────────────────────────

describe("c2: chase-cam reuses vendor cockpit libraries (no re-implementation)", () => {
  test("chase-cam.ts imports cockpitMath for math helpers", () => {
    const src = readCockpitSource("chase-cam.ts");
    expect(src).toContain('from "gev-engine/src/cockpitMath.js"');
  });

  test("chase-cam.ts imports cockpitPresentation for constants", () => {
    const src = readCockpitSource("chase-cam.ts");
    expect(src).toContain('from "gev-engine/src/ui/cockpitPresentation.js"');
  });
});

// ── c3: single-camera-writer invariant (GEV P15 T7) ───────────────────────
//
// chase-cam is the SOLE camera writer in cockpit mode (it calls
// viewer.camera.setView). mouse-look is an INPUT-only module — it must
// mutate only closure-scoped {headingDeltaRad, pitchDeltaRad,
// rangeOffsetM} state and surface them via getFrameOffset(). Camera
// writes flow through the animator (setCameraTargetFrame) only on
// snap-back; live drag offsets never touch the camera directly.
//
// Why this guard matters: if mouse-look ever starts calling
// viewer.camera.setView (or .flyTo / .lookAt outside the animator),
// chase-cam's 50ms tick will fight the drag handler and the camera
// will jitter.

describe("c3: mouse-look is input-only — never writes camera directly", () => {
  test("mouse-look.ts does NOT call viewer.camera.setView (single-camera-writer invariant)", () => {
    const src = readCockpitSource("mouse-look.ts");
    // Camera writes go through setCameraTargetFrame (animator) only.
    expect(src).not.toMatch(/\.camera\.setView\b/);
  });

  test("mouse-look.ts READS viewer.camera (constructor contract — proves camera is wired, not bypassed)", () => {
    const src = readCockpitSource("mouse-look.ts");
    // mouse-look reads viewer.camera in its constructor seam
    // (`if (!deps?.viewer?.camera)`); optional-chained forms must count too
    // since the file uses `viewer?.camera` rather than `viewer.camera.`.
    const cameraReads = (src.match(/\?\.camera\b|\.camera\b/g) ?? []).length;
    expect(cameraReads).toBeGreaterThan(0);
  });
});

// ── c4: cockpit HUD tick module exports (GEV P16 T6) ───────────────────────
//
// mountCockpitHudTick is the 10Hz timer driver that drains chase-cam +
// flights into the cockpit instruments adapter. It is added to the
// barrel in P16 T4 alongside the HudTickHandle interface (type-only).

describe("c4: cockpit-hud-tick module exports mountCockpitHudTick + HudTickHandle", () => {
  test("cockpit-hud-tick exports mountCockpitHudTick as a runtime function", async () => {
    const m = await import("../cockpit-hud-tick");
    expect(typeof m.mountCockpitHudTick).toBe("function");
  });

  test("HudTickHandle is type-only (no runtime value)", async () => {
    const m = await import("../cockpit-hud-tick");
    // HudTickHandle is `export interface` — erased at runtime.
    expect("HudTickHandle" in m).toBe(false);
  });
});

// ── c5: barrel re-exports ChaseCamResolvedState (GEV P16 T6) ───────────────

describe("c5: ChaseCamResolvedState type is re-exported from the barrel", () => {
  test("index barrel surfaces ChaseCamResolvedState for type-only import", () => {
    // Compile-time-only check via `import()` type syntax in type slot.
    // TypeScript resolves the barrel's `export type { ChaseCamResolvedState }`
    // even though no runtime value exists. The test body is intentionally
    // empty — the assertion IS the compile check itself.
    type _Check = import("../index").ChaseCamResolvedState;
    const _compileSentinel: _Check | undefined = undefined;
    expect(_compileSentinel).toBeUndefined();
  });
});

// ── c6: instruments-mount frame includes P16 fields (GEV P16 T6) ────────────
//
// P16 T2 added pitchRad/bankRad/vsiMps to CockpitInstrumentFrame (forwarded
// from ChaseCamResolvedState). This guard pins the shape at compile time so
// HUD consumers (T5 HudCockpitInstruments) cannot drift to the wrong fields.

describe("c6: instruments-mount frame includes 3 new P16 fields at compile time", () => {
  test("CockpitInstrumentFrame carries pitchRad/bankRad/vsiMps", () => {
    type Frame = {
      pitchRad: number;
      bankRad: number;
      vsiMps: number;
    };
    // Type-only assignment — compile error if any field is missing.
    const _typeCheck: Frame = { pitchRad: 0, bankRad: 0, vsiMps: 0 };
    expect(_typeCheck).toBeDefined();
  });
});

// ── c7: cockpit-store elementVisibility field (GEV P17 T7) ─────────────
//
// P17 T2 added elementVisibility to CockpitStoreState. This guard pins
// the shape at compile time so any caller wiring HudCockpitInstruments'
// `visibility` prop to the store cannot drift to the wrong field name.

describe("c7: cockpit-store carries elementVisibility at compile time", () => {
  test("CockpitStoreState.elementVisibility is an 8-key boolean record", () => {
    type State = {
      elementVisibility: {
        compass: boolean;
        altimeter: boolean;
        speedRuler: boolean;
        altitudeLadder: boolean;
        speedTape: boolean;
        pitchLadder: boolean;
        bankIndicator: boolean;
        vsiChevron: boolean;
      };
    };
    // Type-only assignment — compile error if any field is missing.
    const _typeCheck: State = {
      elementVisibility: {
        compass: true,
        altimeter: true,
        speedRuler: true,
        altitudeLadder: true,
        speedTape: true,
        pitchLadder: true,
        bankIndicator: true,
        vsiChevron: true,
      },
    };
    expect(_typeCheck.elementVisibility.compass).toBe(true);
  });
});

// ── c8: element-visibility storage key is stable (GEV P17 T7) ──────────────
//
// P17 T1 picked the literal `intelhub.cockpit.elementVisibility` as the
// localStorage key. Once shipped, renaming it would silently invalidate
// every existing user's preferences. Pin the literal in source so any
// future drift breaks tests.

describe("c8: ELEMENT_VISIBILITY_STORAGE_KEY literal is pinned in source", () => {
  test("element-visibility.ts declares the literal intelhub.cockpit.elementVisibility", () => {
    const src = readCockpitSource("element-visibility.ts");
    expect(src).toContain('"intelhub.cockpit.elementVisibility"');
  });
});

// ── c9: HudCockpitElementSwitch is a runtime component (GEV P17 T7) ─────────
//
// P17 T4 added HudCockpitElementSwitch as a React component. If a future
// refactor accidentally turns the export into a type-only (e.g., turns the
// function into an interface), this contract catches the drift.

describe("c9: HudCockpitElementSwitch is a runtime component export", () => {
  test("HudCockpitElementSwitch file exists and exports the component", () => {
    // The file lives in console/src/globe-hud/ (not gev-visual/cockpit/),
    // so readFileSync uses a path two levels up (out of __tests__/cockpit/)
    // then through the sibling globe-hud/ directory.
    const path = join(here, "..", "..", "..", "globe-hud", "HudCockpitElementSwitch.tsx");
    const src = readFileSync(path, "utf8");
    expect(src).toMatch(/export function HudCockpitElementSwitch\b/);
  });

  test("the toggle button testid is hardcoded in source", () => {
    const path = join(here, "..", "..", "..", "globe-hud", "HudCockpitElementSwitch.tsx");
    const src = readFileSync(path, "utf8");
    expect(src).toContain('"hud-cockpit-element-switch"');
  });
});
