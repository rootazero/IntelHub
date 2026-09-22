// GEV P20 TCAS — proximity-target diamonds on the cockpit HUD.
//
// Per §6.3 spec (D-TCAS-3=C, D-TCAS-5=B): diamond markers positioned by
// bearing + distance from the agent, colored by threat level
// (white=monitor, amber=caution, red=warning). 5 nm radius = circle
// that fills the lower half of the viewport.
//
// Layout: SVG overlay sized 300×300 in the bottom-right of the cockpit
// chrome. Bearing 0° (north) at top, increasing clockwise. Distance
// rings at 1, 2, 5 nm. Each target rendered as a small diamond at
// (bearing, distance) → (x, y).
//
// Relative altitude ladder (D-TCAS-3 sub-option B): drawn as horizontal
// altitude bars next to each diamond, showing ±2.5k ft deviation from
// the agent altitude.

import type { TcasTarget } from "../gev-visual/cockpit/tcas-client";

const SIZE = 300;
const CENTER = SIZE / 2;
const RADIUS_MAX_NM = 5;
const PX_PER_NM = (SIZE / 2 - 12) / RADIUS_MAX_NM;

/** Project (bearing_deg, distance_nm) → (x, y) on the SVG canvas.
 *  Bearing 0° = up (north), increasing clockwise. */
function project(bearingDeg: number, distanceNm: number) {
  const r = distanceNm * PX_PER_NM;
  const theta = (bearingDeg - 90) * (Math.PI / 180); // 0° → up
  return {
    x: CENTER + r * Math.cos(theta),
    y: CENTER + r * Math.sin(theta),
  };
}

function threatColor(threat: TcasTarget["threat"]): string {
  switch (threat) {
    case "warning":
      return "#ff4757";
    case "caution":
      return "#ffa502";
    case "monitor":
      return "#9aa4b2";
    default:
      return "#6b7280";
  }
}

function threatLabel(threat: TcasTarget["threat"]): string {
  switch (threat) {
    case "warning":
      return "WARNING";
    case "caution":
      return "CAUTION";
    case "monitor":
      return "MONITOR";
    default:
      return "";
  }
}

export function HudCockpitTcasOverlay({
  targets,
  agentAltitudeM,
}: {
  targets: TcasTarget[];
  agentAltitudeM: number | null;
}) {
  const active = targets.filter(
    (t) => t.threat !== "none" && t.distance_nm <= RADIUS_MAX_NM,
  );
  const counts = {
    warning: active.filter((t) => t.threat === "warning").length,
    caution: active.filter((t) => t.threat === "caution").length,
    monitor: active.filter((t) => t.threat === "monitor").length,
  };

  return (
    <div
      className="hud-cockpit-tcas"
      data-testid="hud-cockpit-tcas-overlay"
    >
      <svg
        width={SIZE}
        height={SIZE}
        viewBox={`0 0 ${SIZE} ${SIZE}`}
        aria-hidden
      >
        {/* Distance rings (1, 2, 5 nm). */}
        {[1, 2, 5].map((nm) => (
          <circle
            key={nm}
            cx={CENTER}
            cy={CENTER}
            r={nm * PX_PER_NM}
            fill="none"
            stroke="rgba(154, 164, 178, 0.18)"
            strokeWidth={1}
            strokeDasharray="2 4"
          />
        ))}
        {/* Compass marks every 45°. */}
        {[0, 45, 90, 135, 180, 225, 270, 315].map((deg) => {
          const inner = project(deg, 0);
          const outer = project(deg, RADIUS_MAX_NM);
          return (
            <line
              key={deg}
              x1={inner.x}
              y1={inner.y}
              x2={outer.x}
              y2={outer.y}
              stroke="rgba(154, 164, 178, 0.25)"
              strokeWidth={1}
            />
          );
        })}
        {/* Agent triangle at the center (always pointing up = heading 0). */}
        <polygon
          points={`${CENTER},${CENTER - 6} ${CENTER - 5},${CENTER + 4} ${CENTER + 5},${CENTER + 4}`}
          fill="#7fd0ff"
        />
        {/* Target diamonds. */}
        {active.map((t) => {
          const { x, y } = project(t.bearing_deg, t.distance_nm);
          const color = threatColor(t.threat);
          const altDeltaFt = (t.alt_m - (agentAltitudeM ?? 0)) * 3.28084;
          return (
            <g key={t.hex}>
              <polygon
                points={`${x},${y - 4} ${x + 4},${y} ${x},${y + 4} ${x - 4},${y}`}
                fill={color}
                stroke="rgba(3, 5, 9, 0.8)"
                strokeWidth={0.5}
              />
              {/* Altitude ladder + relative-altitude readout (only when
                  the agent altitude is known). */}
              {agentAltitudeM !== null ? (
                <>
                  <line
                    x1={x + 6}
                    y1={y}
                    x2={x + 22}
                    y2={y}
                    stroke={color}
                    strokeWidth={2}
                    strokeDasharray={altDeltaFt > 0 ? "0" : "2 2"}
                  />
                  <text
                    x={x + 6}
                    y={y - 6}
                    fontSize={8}
                    fill={color}
                    fontFamily="monospace"
                  >
                    {altDeltaFt >= 0 ? "+" : ""}
                    {Math.round(altDeltaFt / 100) / 10}k
                  </text>
                </>
              ) : null}
            </g>
          );
        })}
      </svg>
      <div className="hud-cockpit-tcas-summary">
        <span className="hud-cockpit-tcas-warning">{counts.warning}</span>
        <span className="hud-cockpit-tcas-caution">{counts.caution}</span>
        <span className="hud-cockpit-tcas-monitor">{counts.monitor}</span>
      </div>
      <div className="hud-cockpit-tcas-label">TCAS</div>
      {counts.warning > 0 ? (
        <div className="hud-cockpit-tcas-threat-tag">WARNING</div>
      ) : counts.caution > 0 ? (
        <div className="hud-cockpit-tcas-threat-tag caution">CAUTION</div>
      ) : null}
    </div>
  );
}

export const TCAS_RADIUS_NM = RADIUS_MAX_NM;
export { threatColor, threatLabel };