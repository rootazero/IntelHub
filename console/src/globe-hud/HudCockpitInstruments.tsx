// GEV P9 cockpit instrument cluster — SVG compass + altitude tape + speed
// tape, driven by a 4 Hz requestAnimationFrame poll of the instruments adapter
// (plan Task 4 step 2). All data comes from the adapter frame
// (CockpitInstrumentFrame); the component owns no Cesium/engine imports.
//
// The RAF loop is the ONLY effect here and it never touches the camera (D4:
// overlay, not globe replacement) — it only reads `instruments.update()`. The
// P8 skipFirst lesson applies to view-mutating effects; a data read is safe on
// the first frame, but we still guard the loop against double-scheduling under
// StrictMode via the cleanup.
import { useEffect, useRef, useState } from "react";
import type {
  CockpitInstrumentFrame,
  InstrumentsHandle,
  RulerTick,
} from "../gev-visual/cockpit/instruments-mount";
import {
  DEFAULT_ELEMENT_VISIBILITY,
  type ElementVisibility,
} from "../gev-visual/cockpit/element-visibility";
import type { SvsSamplePoint } from "../gev-visual/cockpit/svs-terrain-sampler";
import { HudCockpitSvsOverlay } from "./HudCockpitSvsOverlay";

const POLL_MS = 250; // 4 Hz

export interface HudCockpitInstrumentsProps {
  instruments: InstrumentsHandle | null;
  /** GEV P17: per-element visibility map. Optional — defaults to
   *  DEFAULT_ELEMENT_VISIBILITY (all visible) so existing P16 tests
   *  and callers don't break. */
  visibility?: ElementVisibility;
  /** GEV P19 SVS: terrain samples drawn inside the pitch ladder.
   *  Optional — when undefined or empty, no overlay renders. */
  svsSamples?: SvsSamplePoint[];
  /** GEV P19 SVS: agent altitude in metres AGL. Used to compute
   *  elevation delta vs sample elevation. Required when svsSamples
   *  is provided. */
  svsAgentAltitudeM?: number | null;
}

/** Compass: a ring of 7 divisions centered on the heading + center readout. */
function Compass({ frame }: { frame: CockpitInstrumentFrame | null }) {
  const heading = frame?.heading ?? 0;
  const divisions = frame?.compass ?? [];
  return (
    <div className="hud-cockpit-gauge" data-testid="hud-cockpit-compass">
      <svg viewBox="0 0 160 160" className="hud-cockpit-compass-ring" aria-hidden>
        <circle cx="80" cy="80" r="70" className="hud-cockpit-ring" />
        {divisions.map((d) => {
          const angle = ((d.division - heading) * Math.PI) / 180;
          const x = 80 + Math.sin(angle) * 56;
          const y = 80 - Math.cos(angle) * 56;
          return (
            <g key={d.slot}>
              <line
                x1="80"
                y1="16"
                x2="80"
                y2="28"
                className={
                  d.active
                    ? "hud-cockpit-tick active"
                    : "hud-cockpit-tick"
                }
                transform={`rotate(${d.division - heading} 80 80)`}
              />
              <text
                x={x}
                y={y}
                textAnchor="middle"
                dominantBaseline="middle"
                className={
                  d.active ? "hud-cockpit-div active" : "hud-cockpit-div"
                }
              >
                {d.label}
              </text>
            </g>
          );
        })}
      </svg>
      <div className="hud-cockpit-heading" aria-live="off">
        {frame?.headingLabel ?? "---"}
      </div>
    </div>
  );
}

/** Vertical altitude tape: 9 ticks spaced by slot, center pointer at slot 0. */
function Altimeter({ frame }: { frame: CockpitInstrumentFrame | null }) {
  const ticks = frame?.altitudeTicks ?? [];
  const SLOT_PX = 22;
  return (
    <div className="hud-cockpit-gauge" data-testid="hud-cockpit-altimeter">
      <svg
        viewBox="0 0 120 200"
        className="hud-cockpit-tape"
        aria-hidden
        preserveAspectRatio="xMidYMid meet"
      >
        <rect
          x="52"
          y="6"
          width="16"
          height="188"
          className="hud-cockpit-tape-frame"
        />
        {ticks.map((t) => {
          const y = 100 + t.slot * SLOT_PX;
          return (
            <g key={`${t.value}-${t.slot}`}>
              <line
                x1={t.major ? "40" : "46"}
                x2="52"
                y1={y}
                y2={y}
                className="hud-cockpit-tick"
              />
              <text
                x="36"
                y={y + 3}
                textAnchor="end"
                className={t.major ? "hud-cockpit-div major" : "hud-cockpit-div"}
              >
                {t.label}
              </text>
            </g>
          );
        })}
        <line x1="34" y1="100" x2="68" y2="100" className="hud-cockpit-pointer" />
      </svg>
      <div className="hud-cockpit-readout" aria-live="off">
        {frame?.altitudeLabel ?? "-----"} FT
      </div>
    </div>
  );
}

/** Horizontal speed tape: 9 ticks spaced by slot, center pointer at slot 0. */
function SpeedRuler({ frame }: { frame: CockpitInstrumentFrame | null }) {
  const ticks = frame?.speedTicks ?? [];
  const SLOT_PX = 22;
  return (
    <div className="hud-cockpit-gauge" data-testid="hud-cockpit-speed">
      <svg
        viewBox="0 0 200 60"
        className="hud-cockpit-tape"
        aria-hidden
        preserveAspectRatio="xMidYMid meet"
      >
        <rect
          x="6"
          y="22"
          width="188"
          height="16"
          className="hud-cockpit-tape-frame"
        />
        {ticks.map((t) => {
          const x = 100 + t.slot * SLOT_PX;
          return (
            <g key={`${t.value}-${t.slot}`}>
              <line
                x1={x}
                x2={x}
                y1={t.major ? "12" : "18"}
                y2="22"
                className="hud-cockpit-tick"
              />
              <text
                x={x}
                y="48"
                textAnchor="middle"
                className={t.major ? "hud-cockpit-div major" : "hud-cockpit-div"}
              >
                {t.label}
              </text>
            </g>
          );
        })}
        <line x1="100" y1="6" x2="100" y2="38" className="hud-cockpit-pointer" />
      </svg>
      <div className="hud-cockpit-readout" aria-live="off">
        {frame?.speedLabel ?? "---"} KT
      </div>
    </div>
  );
}

/** GEV P16 T5: vertical altitude ladder — slot-spaced ticks centered on slot 0. */
function AltitudeLadder({ ticks }: { ticks: RulerTick[] }) {
  return (
    <div className="hud-cockpit-gauge-light">
    <svg
      data-testid="altitude-ladder"
      className="hud-cockpit-altitude-ladder"
      width="80"
      height="200"
      viewBox="0 0 80 200"
      aria-hidden
    >
      {ticks.map((tick) => (
        <g
          key={tick.slot}
          data-testid="altitude-tick"
          transform={`translate(0, ${100 + tick.slot * 20})`}
        >
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
    </div>
  );
}

/** GEV P16 T5: vertical speed tape — slot-spaced ticks, mirrored to right side. */
function SpeedTape({ ticks }: { ticks: RulerTick[] }) {
  return (
    <div className="hud-cockpit-gauge-light">
    <svg
      data-testid="speed-tape"
      className="hud-cockpit-speed-tape"
      width="80"
      height="200"
      viewBox="0 0 80 200"
      aria-hidden
    >
      {ticks.map((tick) => (
        <g
          key={tick.slot}
          data-testid="speed-tick"
          transform={`translate(50, ${100 + tick.slot * 20})`}
        >
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
    </div>
  );
}

/** GEV P16 T5: pitch ladder — horizon lines rotated by bank, shifted by pitch. */
function PitchLadder({
  pitchRad,
  bankRad,
  svsSamples,
  svsAgentAltitudeM,
}: {
  pitchRad: number;
  bankRad: number;
  svsSamples?: SvsSamplePoint[];
  svsAgentAltitudeM?: number | null;
}) {
  const PITCH_RANGE_DEG = [-30, -20, -10, 0, 10, 20, 30];
  const bankDeg = (bankRad * 180) / Math.PI;
  const pitchDeg = (pitchRad * 180) / Math.PI;
  return (
    <div className="hud-cockpit-gauge-light">
    <svg
      data-testid="pitch-ladder"
      className="hud-cockpit-pitch-ladder"
      width="200"
      height="300"
      viewBox="0 0 200 300"
      aria-hidden
    >
      <g transform={`translate(100, 150) rotate(${bankDeg})`}>
        {PITCH_RANGE_DEG.map((pitchLineDeg) => {
          const yOffset = (pitchLineDeg - pitchDeg) * 5;
          return (
            <g key={pitchLineDeg} transform={`translate(0, ${yOffset})`}>
              <line
                x1="-30"
                x2="30"
                y1="0"
                y2="0"
                stroke={
                  pitchLineDeg === 0 ? "currentColor" : "rgba(255,255,255,0.6)"
                }
                strokeWidth={pitchLineDeg === 0 ? "2" : "1"}
              />
              <text
                x="-35"
                y="4"
                fontSize="10"
                fill="currentColor"
                textAnchor="end"
              >
                {Math.abs(pitchLineDeg)}
              </text>
              <text x="35" y="4" fontSize="10" fill="currentColor">
                {Math.abs(pitchLineDeg)}
              </text>
            </g>
          );
        })}
        {/* GEV P19 SVS: wireframe terrain overlay on the pitch ladder.
            Sits inside the banked group so it rolls with the horizon. */}
        {svsSamples && svsSamples.length > 0 && svsAgentAltitudeM !== null && svsAgentAltitudeM !== undefined ? (
          <HudCockpitSvsOverlay
            samples={svsSamples}
            agentAltitudeM={svsAgentAltitudeM}
          />
        ) : null}
      </g>
    </svg>
    </div>
  );
}

/** GEV P16 T5: bank indicator — arc of |deg| ticks with cyan current-bank marker. */
function BankIndicator({ bankRad }: { bankRad: number }) {
  const BANK_RANGE_DEG = [-60, -45, -30, -15, 0, 15, 30, 45, 60];
  const bankDeg = (bankRad * 180) / Math.PI;
  const markerX = 100 + bankDeg * 1.5;
  return (
    <div className="hud-cockpit-gauge-light">
    <svg
      data-testid="bank-indicator"
      className="hud-cockpit-bank-indicator"
      width="200"
      height="60"
      viewBox="0 0 200 60"
      aria-hidden
    >
      {BANK_RANGE_DEG.map((deg) => (
        <g key={deg} transform={`translate(${100 + deg * 1.5}, 30)`}>
          {deg === 0 ? (
            <polygon points="0,-5 -4,5 4,5" fill="currentColor" />
          ) : (
            <text
              fontSize="10"
              fill="currentColor"
              textAnchor="middle"
              dominantBaseline="middle"
            >
              {Math.abs(deg)}
            </text>
          )}
        </g>
      ))}
      <polygon
        points={`${markerX},50 ${markerX - 5},60 ${markerX + 5},60`}
        fill="cyan"
      />
    </svg>
    </div>
  );
}

/** GEV P16 T5: VSI chevron — lime up / amber down + ft/min readout. */
function VsiChevron({ vsiMps }: { vsiMps: number }) {
  const MPS_TO_FTPM = 196.85;
  const ftpm = Math.abs(vsiMps * MPS_TO_FTPM);
  const isUp = vsiMps > 0;
  const isDown = vsiMps < 0;
  return (
    <div className="hud-cockpit-gauge-light">
    <svg
      data-testid="vsi-chevron"
      className="hud-cockpit-vsi-chevron"
      width="40"
      height="200"
      viewBox="0 0 40 200"
      aria-hidden
    >
      {isUp && (
        <polygon
          points="20,90 10,100 30,100"
          fill="lime"
          data-testid="vsi-up"
        />
      )}
      {isDown && (
        <polygon
          points="20,110 10,100 30,100"
          fill="amber"
          data-testid="vsi-down"
        />
      )}
      <text x="20" y="160" fontSize="10" fill="currentColor" textAnchor="middle">
        {vsiMps !== 0 ? ftpm.toFixed(0) : "0"}
      </text>
    </svg>
    </div>
  );
}

export function HudCockpitInstruments({
  instruments,
  visibility,
  svsSamples,
  svsAgentAltitudeM,
}: HudCockpitInstrumentsProps) {
  const [frame, setFrame] = useState<CockpitInstrumentFrame | null>(null);
  const rafRef = useRef<number | null>(null);
  const lastRef = useRef(0);
  const skipFirst = useRef(true);

  useEffect(() => {
    if (!instruments) return;
    // Immediate first read so the gauges never flash empty; the RAF loop then
    // refreshes at 4 Hz. skipFirst (P8 lesson) gates the RAF throttle baseline
    // so the loop never fires an update against a half-committed overlay.
    setFrame(instruments.update());
    const poll = (timestamp: number) => {
      if (skipFirst.current) {
        skipFirst.current = false;
        lastRef.current = timestamp;
      } else if (timestamp - lastRef.current >= POLL_MS) {
        lastRef.current = timestamp;
        setFrame(instruments.update());
      }
      rafRef.current = requestAnimationFrame(poll);
    };
    rafRef.current = requestAnimationFrame(poll);
    return () => {
      if (rafRef.current != null) cancelAnimationFrame(rafRef.current);
      rafRef.current = null;
      skipFirst.current = true;
    };
  }, [instruments]);

  if (!instruments) return null;

  // GEV P17: resolve visibility — undefined prop defaults to all-visible so
  // pre-P17 callers (16 P16 tests, GlobeV2 wiring) keep working unchanged.
  const v = visibility ?? DEFAULT_ELEMENT_VISIBILITY;
  const show = (key: keyof ElementVisibility) => v[key];

  return (
    <div className="hud-cockpit-instruments" data-testid="hud-cockpit-instruments">
      {show("compass") ? <Compass frame={frame} /> : null}
      {show("altimeter") ? <Altimeter frame={frame} /> : null}
      {show("speedRuler") ? <SpeedRuler frame={frame} /> : null}
      {frame && show("altitudeLadder") ? (
        <AltitudeLadder ticks={frame.altitudeTicks} />
      ) : null}
      {frame && show("speedTape") ? (
        <SpeedTape ticks={frame.speedTicks} />
      ) : null}
      {frame && show("pitchLadder") ? (
        <PitchLadder
          pitchRad={frame.pitchRad}
          bankRad={frame.bankRad}
          svsSamples={svsSamples}
          svsAgentAltitudeM={svsAgentAltitudeM}
        />
      ) : null}
      {frame && show("bankIndicator") ? (
        <BankIndicator bankRad={frame.bankRad} />
      ) : null}
      {frame && show("vsiChevron") ? (
        <VsiChevron vsiMps={frame.vsiMps} />
      ) : null}
    </div>
  );
}
