// GEV P10 T3 — recording-mode toggle HUD.
//
// Wraps the recording-controls tail adapter in a single-button HUD that:
//   - shows the current recording state (active vs. inactive)
//   - toggles body.recording-mode (the sp8 check_44 acceptance hook)
//   - emits a mode-active badge when active so the cockpit / scene playback
//     can know to hide other UI (per vendor recording.css body selector).
//
// testids (brief §testids):
//   - `hud-recording-toggle`      — the toggle button
//   - `hud-recording-mode-active` — badge visible when active
//
// The vendor's `RecordingControls` reads `this.hud.{getMode, getVariant,
// setMode, setVariant, visible}` — this component is the source of truth for
// those getters. GlobeV2 wiring passes the adapter's setRecordingMode() call
// here, then the parent observes the state change and re-renders.
import { useCallback } from "react";

export interface HudRecordingControlsProps {
  /** Whether recording mode is currently active. */
  active: boolean;
  /** Called when the operator toggles the button. */
  onToggle?: (nextActive: boolean) => void;
  /** Localized label override (rare). */
  activeLabel?: string;
  inactiveLabel?: string;
  /** Override the toggle glyph (rare; default ⏺ for record). */
  activeGlyph?: string;
  inactiveGlyph?: string;
}

export function HudRecordingControls({
  active,
  onToggle,
  activeLabel = "录制中 · REC",
  inactiveLabel = "录制模式",
  activeGlyph = "⏺",
  inactiveGlyph = "○",
}: HudRecordingControlsProps) {
  const handleClick = useCallback(() => {
    onToggle?.(!active);
  }, [active, onToggle]);

  return (
    <div
      className={`hud-recording-controls${active ? " active" : ""}`}
      data-active={active}
    >
      <button
        type="button"
        className={`hud-recording-toggle${active ? " active" : ""}`}
        data-testid="hud-recording-toggle"
        aria-pressed={active}
        aria-label={active ? activeLabel : inactiveLabel}
        title={active ? activeLabel : inactiveLabel}
        onClick={handleClick}
      >
        <span aria-hidden>{active ? activeGlyph : inactiveGlyph}</span>
        <span className="hud-recording-toggle-label">
          {active ? activeLabel : inactiveLabel}
        </span>
      </button>
      {active && (
        <span
          className="hud-recording-mode-active"
          data-testid="hud-recording-mode-active"
          aria-label="recording mode active"
        >
          ●
        </span>
      )}
    </div>
  );
}