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
} from "../gev-visual/cockpit/instruments-mount";

const POLL_MS = 250; // 4 Hz

export interface HudCockpitInstrumentsProps {
  instruments: InstrumentsHandle | null;
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

export function HudCockpitInstruments({
  instruments,
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

  return (
    <div className="hud-cockpit-instruments" data-testid="hud-cockpit-instruments">
      <Compass frame={frame} />
      <Altimeter frame={frame} />
      <SpeedRuler frame={frame} />
    </div>
  );
}
