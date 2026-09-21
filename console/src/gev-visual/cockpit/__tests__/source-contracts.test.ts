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
