// GEV P9 cockpit context readout (top-left): callsign / ICAO / altitude /
// heading / speed, refreshed every 250 ms (4 Hz) from flights.getTrackedInfo().
//
// Data note (T1 finding #3): getTrackedInfo() returns both `altitudeM` (MSL,
// the readout datum) and `renderAltitudeM` (a render-only surface datum).
// Only `altitudeM` is read here — renderAltitudeM is deliberately excluded.
import { useEffect, useState } from "react";
import type { CockpitTrackedInfo } from "../gev-visual/cockpit/instruments-mount";

/** vendor cockpitInstruments.js updateHud m/s → knots conversion. */
const MPS_TO_KTS = 1.94384;

const CONTEXT_POLL_MS = 250;

function formatAltitudeM(meters: number | undefined): string {
  return meters === undefined || !Number.isFinite(meters)
    ? "-----"
    : `${Math.round(meters).toLocaleString("en-US")} M`;
}

function formatHeading(track: number | null | undefined): string {
  if (track == null || !Number.isFinite(track)) return "---°";
  return `${String(Math.round(((track % 360) + 360) % 360)).padStart(3, "0")}°`;
}

function formatSpeedKt(mps: number | null | undefined): string {
  return mps == null || !Number.isFinite(mps)
    ? "--- KT"
    : `${Math.round(mps * MPS_TO_KTS)} KT`;
}

export interface HudCockpitContextProps {
  /** flights.getTrackedInfo() seam (dead-reckoned tracked aircraft). */
  getTrackedInfo?: () => CockpitTrackedInfo | null;
}

export function HudCockpitContext({
  getTrackedInfo,
}: HudCockpitContextProps) {
  const [info, setInfo] = useState<CockpitTrackedInfo | null>(null);

  useEffect(() => {
    if (!getTrackedInfo) return;
    // First read is synchronous so the readout never flashes empty; the
    // interval then keeps it live at the 4 Hz cadence.
    setInfo(getTrackedInfo() ?? null);
    const timer = setInterval(
      () => setInfo(getTrackedInfo() ?? null),
      CONTEXT_POLL_MS,
    );
    return () => clearInterval(timer);
  }, [getTrackedInfo]);

  const callsign = info?.callsign || "AIRCRAFT";
  const icao = info?.icao24 || "-----";

  return (
    <div className="hud-cockpit-context" data-testid="hud-cockpit-context">
      <div className="hud-cockpit-context-callsign">{callsign}</div>
      <div className="hud-cockpit-context-icao">ICAO {icao}</div>
      <dl className="hud-cockpit-context-readout">
        <div className="hud-cockpit-context-field">
          <dt>ALT</dt>
          <dd>{formatAltitudeM(info?.altitudeM)}</dd>
        </div>
        <div className="hud-cockpit-context-field">
          <dt>HDG</dt>
          <dd>{formatHeading(info?.track)}</dd>
        </div>
        <div className="hud-cockpit-context-field">
          <dt>SPD</dt>
          <dd>{formatSpeedKt(info?.velocityMps)}</dd>
        </div>
      </dl>
    </div>
  );
}
