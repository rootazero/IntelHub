// GEV P19 SVS — wireframe terrain overlay on the pitch ladder.
//
// Per §6.3 spec (D-SVS-1=A 2D HUD overlay, D-SVS-4=A wireframe):
// draws the terrain sampler output as a 5×9 wireframe mesh inside
// the existing pitch ladder's banked group. The mesh scrolls
// vertically with pitch (forward = downward on the ladder) AND with
// terrain elevation (higher elevation relative to agent altitude AGL
// = downward shift = "obstacle ahead").
//
// All math is local-frame (forward / right metres) — no Cesium transforms
// in this component. The sampler has already rotated samples into the
// agent's heading frame.

import type { SvsSamplePoint } from "../gev-visual/cockpit/svs-terrain-sampler";

/** Pixels per metre on the pitch-ladder canvas. Tuned so the wireframe
 *  matches the existing pitch ladder's pitch scale (10° = 50 px ≈
 *  ~100 m at standard 100 m / 10° visual rate). */
const PX_PER_M_FORWARD = 0.5;
const PX_PER_M_ELEVATION = 0.25;

/** Project a sample into (x, y) on the ladder canvas. Returns null when
 *  the sample falls outside the lane box (off-screen L/R). */
function project(
  sample: SvsSamplePoint,
  agentAltitudeM: number,
): { x: number; y: number } | null {
  // The ladder is 200 px wide. Map right_m to a centered x.
  const x = sample.rightM * PX_PER_M_FORWARD;
  if (x < -100 || x > 100) return null;
  // Forward → downward (negative y in SVG), elevation delta → downward.
  const elevationDeltaM = sample.elevationM - agentAltitudeM;
  const y =
    sample.forwardM * PX_PER_M_FORWARD +
    elevationDeltaM * PX_PER_M_ELEVATION;
  // Skip off-screen (below the ladder).
  if (y < -150 || y > 200) return null;
  return { x, y };
}

export function HudCockpitSvsOverlay({
  samples,
  agentAltitudeM,
}: {
  samples: SvsSamplePoint[];
  agentAltitudeM: number;
}) {
  if (!samples.length) return null;
  const projected: Array<{
    x: number;
    y: number;
    raw: SvsSamplePoint;
  }> = [];
  for (const s of samples) {
    const p = project(s, agentAltitudeM);
    if (p !== null) {
      projected.push({ ...p, raw: s });
    }
  }
  if (!projected.length) return null;

  // Build row-pairs and col-pairs for wireframe polylines.
  // We don't store original row/col in samples; reconstruct by matching
  // forwardM rounded to the nearest step.
  const STEP_FORWARD = 50;
  const STEP_RIGHT = 40;
  const cols = new Map<number, SvsSamplePoint[]>();
  for (const s of samples) {
    const rightKey = Math.round(s.rightM / STEP_RIGHT);
    if (!cols.has(rightKey)) cols.set(rightKey, []);
    cols.get(rightKey)!.push(s);
  }
  const sortedCols = [...cols.keys()].sort((a, b) => a - b);
  const sortedRightM = sortedCols.map((k) => k * STEP_RIGHT);

  // Group projected points by rightKey.
  const projectedByCol = new Map<
    number,
    Array<{ x: number; y: number; raw: SvsSamplePoint }>
  >();
  for (const p of projected) {
    const rightKey = Math.round(p.raw.rightM / STEP_RIGHT);
    if (!projectedByCol.has(rightKey)) projectedByCol.set(rightKey, []);
    projectedByCol.get(rightKey)!.push(p);
  }
  // Sort each col by forward distance.
  for (const arr of projectedByCol.values()) {
    arr.sort((a, b) => a.raw.forwardM - b.raw.forwardM);
  }

  const rowLines: string[] = [];
  const colLines: string[] = [];

  // Row lines (constant forwardM): connect adjacent cols at the same row.
  // For each col, walk its sorted samples; for each row, find the matching
  // y in the next col and connect.
  const minRows = Math.min(
    ...sortedCols.map((k) => projectedByCol.get(k)?.length ?? 0),
  );
  for (let row = 0; row < minRows; row++) {
    for (let i = 0; i < sortedCols.length - 1; i++) {
      const a = projectedByCol.get(sortedCols[i])?.[row];
      const b = projectedByCol.get(sortedCols[i + 1])?.[row];
      if (a && b) {
        rowLines.push(`${a.x},${a.y} ${b.x},${b.y}`);
      }
    }
  }
  // Col lines (constant rightM): connect adjacent rows in each col.
  for (const k of sortedCols) {
    const arr = projectedByCol.get(k) ?? [];
    for (let i = 0; i < arr.length - 1; i++) {
      colLines.push(`${arr[i].x},${arr[i].y} ${arr[i + 1].x},${arr[i + 1].y}`);
    }
  }

  // Side refs for the testid so we can prove the component rendered.
  void sortedRightM;
  return (
    <g
      data-testid="hud-cockpit-svs-overlay"
      stroke="#7fd0ff"
      strokeWidth="1"
      fill="none"
      opacity="0.7"
    >
      {rowLines.map((pts, i) => (
        <polyline key={`r${i}`} points={pts} />
      ))}
      {colLines.map((pts, i) => (
        <polyline key={`c${i}`} points={pts} />
      ))}
    </g>
  );
}