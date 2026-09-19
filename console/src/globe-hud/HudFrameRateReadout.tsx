// GEV P10 T3 — frame-rate readout HUD.
//
// Wraps the frame-rate-monitor tail adapter in a small top-right readout
// chip. The vendor's `frameRateMonitor.js` self-displays "FPS {n}" into a
// .frame-rate-readout child of #title-bar; the IntelHub HUD re-uses the
// vendor's DOM injection (no duplicate subscription) and just renders a
// thinner chip on the top-right HUD rail.
//
// testid (brief §testids):
//   - `hud-frame-rate-readout` — outer span
//
// Throttling: 4 Hz (per plan §plan-deferred ruling #6 — same cadence as the
// P9 instrument chips). The vendor's postRender fires at the GPU's full
// rate; we throttle our React update so the chip text doesn't flicker.
import { useEffect, useState } from "react";

/** 4 Hz throttle — 250 ms is the max-age for the displayed FPS value. */
const FPS_THROTTLE_MS = 250;

export interface HudFrameRateReadoutProps {
  /**
   * Latest FPS reading from the vendor. Null = monitor not mounted yet
   * (chip renders "FPS —" until the first postRender fires).
   */
  fps: number | null;
  /** Throttle interval (default 250 ms / 4 Hz per brief). */
  throttleMs?: number;
}

export function HudFrameRateReadout({
  fps,
  throttleMs = FPS_THROTTLE_MS,
}: HudFrameRateReadoutProps) {
  // Throttle: snap to the latest value at most once per interval. The
  // vendor pushes every postRender (~60 Hz); we forward to the DOM at
  // 4 Hz so the chip stays legible and avoids React reconciliation churn.
  const [displayFps, setDisplayFps] = useState<number | null>(fps);
  useEffect(() => {
    if (fps === null) {
      setDisplayFps(null);
      return;
    }
    const timer = setTimeout(() => setDisplayFps(fps), throttleMs);
    return () => clearTimeout(timer);
  }, [fps, throttleMs]);

  const text = displayFps === null ? "—" : `${displayFps}`;

  return (
    <span
      className="hud-frame-rate-readout"
      data-testid="hud-frame-rate-readout"
      aria-label={`frame rate ${displayFps ?? "unknown"} fps`}
      title={`frame rate ${displayFps ?? "unknown"} fps`}
    >
      <span className="hud-frame-rate-readout-label">FPS</span>
      <span className="hud-frame-rate-readout-value">{text}</span>
    </span>
  );
}