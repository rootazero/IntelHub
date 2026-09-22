// GEV P19 SVS — Cesium-native terrain sampler.
//
// Per §6.3 spec (D-SVS-1=A, D-SVS-2=B): reads from the runtime globe's
// terrain provider (whatever Cesium ion supplies via the engine's globe
// setup) and samples N points in a grid around the agent's projected
// ground position. Returns relative elevations vs the agent altitude AGL
// so the SVS overlay can draw wireframe terrain on the pitch ladder.
//
// Pure adapter (mount + sample + destroy). No DOM, no Cesium scene
// mutation. The SVS toggle button on the HUD decides whether to call
// `sample()` — when SVS is off, no work happens.
//
// Geometry:
//   - Sample a 9×5 grid in front of the agent (heading=0°).
//   - Width: 6 cells × 40 m = 240 m  (per side, total 480 m wide).
//   - Depth: 9 cells × 50 m = 450 m  (in front of agent).
//   - Each cell samples one Cartographic height at the agent's
//     projected position; rotated by heading so the grid faces forward.
//   - All math is local ENU around the agent — no Cesium.Transforms.

import * as Cesium from "cesium";

export interface SvsSamplePoint {
  /** Distance forward from agent (metres). Negative = behind. */
  forwardM: number;
  /** Distance right of agent (metres). Negative = left. */
  rightM: number;
  /** Terrain elevation at this point (metres above ellipsoid). */
  elevationM: number;
}

export interface MountCockpitTerrainSamplerDeps {
  /** Globe scene to sample from (viewer.scene.globe). */
  scene: { globe: { terrainProvider: Cesium.TerrainProvider } };
  /** Latitude/longitude of the agent's projected position (radians). */
  agentLat: number;
  /** Longitude of the agent's projected position (radians). */
  agentLng: number;
  /** Agent heading (radians, clockwise from north). */
  agentHeadingRad: number;
  /** Optional override for grid dimensions (default 9 × 5). */
  rows?: number;
  cols?: number;
  /** Forward step between rows in metres (default 50). */
  stepForwardM?: number;
  /** Lateral step between cols in metres (default 40). */
  stepRightM?: number;
}

export interface CockpitTerrainSamplerHandle {
  /** Sample the terrain grid. Cheap: just queries the provider. */
  sample(): Promise<SvsSamplePoint[]>;
  /** Update the agent pose for the next sample. */
  setAgentPose(lat: number, lng: number, headingRad: number): void;
  destroy(): void;
}

/** Convert metres offset (forward + right) into lat/lng delta. */
function metersToRadians(
  fwdM: number,
  rightM: number,
  latRad: number,
): { dLat: number; dLng: number } {
  // 1° lat ≈ 111 320 m. 1° lng ≈ 111 320 m × cos(lat).
  const cosLat = Math.cos(latRad);
  return {
    dLat: fwdM / 111_320,
    dLng: rightM / (111_320 * (cosLat > 0.0001 ? cosLat : 0.0001)),
  };
}

export function mountCockpitTerrainSampler(
  deps: MountCockpitTerrainSamplerDeps,
): CockpitTerrainSamplerHandle {
  const rows = deps.rows ?? 9;
  const cols = deps.cols ?? 5;
  const stepForwardM = deps.stepForwardM ?? 50;
  const stepRightM = deps.stepRightM ?? 40;
  let agentLat = deps.agentLat;
  let agentLng = deps.agentLng;
  let agentHeadingRad = deps.agentHeadingRad;
  let destroyed = false;

  async function sample(): Promise<SvsSamplePoint[]> {
    if (destroyed) return [];
    const provider = deps.scene.globe.terrainProvider;
    if (!provider) return [];
    // Heading-rotated unit basis: forward (heading), right (heading + 90°).
    const sin = Math.sin(agentHeadingRad);
    const cos = Math.cos(agentHeadingRad);
    // forward = (sin, cos); right = (cos, -sin)
    const colHalf = Math.floor(cols / 2);
    const points: SvsSamplePoint[] = [];
    // Build lon/lat pairs up-front; terrainProvider.tilesOnly or
    // sampleTerrainMostDetailed requires an array of Cartographic.
    const cartesians: Cesium.Cartographic[] = [];
    const locals: Array<{ fwd: number; right: number }> = [];
    for (let r = 0; r < rows; r++) {
      const fwdM = (r + 1) * stepForwardM;
      for (let c = 0; c < cols; c++) {
        const rightM = (c - colHalf) * stepRightM;
        // Rotate (rightM, fwdM) into the heading frame.
        const rotFwd = rightM * sin + fwdM * cos;
        const rotRight = rightM * cos - fwdM * sin;
        const { dLat, dLng } = metersToRadians(rotFwd, rotRight, agentLat);
        cartesians.push(
          Cesium.Cartographic.fromRadians(
            agentLng + dLng,
            agentLat + dLat,
            0,
          ),
        );
        locals.push({ fwd: rotFwd, right: rotRight });
      }
    }
    let sampled: Cesium.Cartographic[];
    try {
      sampled = await Cesium.sampleTerrainMostDetailed(provider, cartesians);
    } catch {
      return [];
    }
    for (let i = 0; i < sampled.length; i++) {
      points.push({
        forwardM: locals[i].fwd,
        rightM: locals[i].right,
        elevationM: sampled[i].height,
      });
    }
    return points;
  }

  function setAgentPose(lat: number, lng: number, headingRad: number) {
    agentLat = lat;
    agentLng = lng;
    agentHeadingRad = headingRad;
  }

  function destroy() {
    destroyed = true;
  }

  return { sample, setAgentPose, destroy };
}