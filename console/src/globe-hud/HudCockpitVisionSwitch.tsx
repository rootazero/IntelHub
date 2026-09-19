// GEV P9 cockpit vision switch — 5-mode selector, styled like the P6 globe
// style switcher. Each click drives vision.setMode() (which bridges the stage
// intensities through the P6 visual-effects handle), records the user pick in
// the cockpit store, and persists to localStorage so a reload restores it
// (spec §3.2: localStorage `intelhub.cockpit.visionMode`).
import { useEffect } from "react";
import {
  VISION_MODES,
  isVisionMode,
  type VisionMode,
  type VisionMountHandle,
} from "../gev-visual/cockpit/vision-mount";

const STORAGE_KEY = "intelhub.cockpit.visionMode";

export const VISION_LABELS: Record<VisionMode, string> = {
  optical: "OPTICAL",
  crt: "CRT",
  nvg: "NVG",
  thermal: "FLIR",
  noir: "NOIR",
};

export function readPersistedVisionMode(): VisionMode {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    return isVisionMode(raw) ? raw : "optical";
  } catch {
    return "optical";
  }
}

export interface HudCockpitVisionSwitchProps {
  vision: VisionMountHandle | null;
  /** Current mode from the cockpit store (drives the active highlight). */
  mode: VisionMode;
  /** Persist the pick into the cockpit store. */
  onSelect: (mode: VisionMode) => void;
}

export function HudCockpitVisionSwitch({
  vision,
  mode,
  onSelect,
}: HudCockpitVisionSwitchProps) {
  // D2 mount sync: the store's enter() already replays the persisted mode
  // onto the vision handle, but if this switch mounts before that replay ran
  // (e.g. a null vision handle at enter time), apply it here so the live
  // effect matches the highlighted mode. optical is a no-op.
  useEffect(() => {
    if (mode !== "optical") {
      vision?.setMode(mode);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps -- mount-only sync
  }, []);

  const select = (next: VisionMode) => {
    vision?.setMode(next);
    onSelect(next);
    try {
      localStorage.setItem(STORAGE_KEY, next);
    } catch {
      /* private mode: persistence is best-effort */
    }
  };

  return (
    <div
      className="hud-cockpit-vision"
      data-testid="hud-cockpit-vision-switch"
      role="group"
      aria-label="视觉模式 / Vision mode"
    >
      {VISION_MODES.map((m) => (
        <button
          key={m}
          type="button"
          data-testid={`hud-cockpit-vision-${m}`}
          className={`hud-cockpit-vision-option${m === mode ? " active" : ""}`}
          aria-pressed={m === mode}
          onClick={() => select(m)}
        >
          {VISION_LABELS[m]}
        </button>
      ))}
    </div>
  );
}
