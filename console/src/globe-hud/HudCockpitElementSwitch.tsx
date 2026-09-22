// GEV P17 cockpit HUD element switch — single toggle button + popover with 8
// checkboxes for per-element visibility. Mirrors HudStyleSwitcher (P6) for
// the click-outside pattern and HudCockpitVisionSwitch (P9) for persistence.
// Subscribes to the store so external changes (keyboard shortcuts, cross-tab
// sync, programmatic dispatch) update the checkboxes in real time.
import { useEffect, useRef, useState } from "react";
import type { CockpitStore } from "../gev-visual/cockpit/cockpit-store";
import {
  COCKPIT_ELEMENT_KEYS,
  persistElementVisibility,
  type CockpitElementKey,
  type ElementVisibility,
} from "../gev-visual/cockpit/element-visibility";

/** Display labels for the popover. Mixed case (matches the existing EN HUD
 *  strings — the vision switch uses all-caps "OPTICAL/CRT/...", but the
 *  checkboxes are labels next to controls, not status badges, so mixed case
 *  reads better here). i18n deferred — single-language MVP; can layer `t()`
 *  on top later without API changes. */
export const ELEMENT_LABELS: Record<CockpitElementKey, string> = {
  compass: "Heading",
  altimeter: "Altimeter",
  speedRuler: "Speed",
  altitudeLadder: "Altitude Ladder",
  speedTape: "Speed Tape",
  pitchLadder: "Pitch Ladder",
  bankIndicator: "Bank Indicator",
  vsiChevron: "VSI",
};

export interface HudCockpitElementSwitchProps {
  store: CockpitStore;
}

export function HudCockpitElementSwitch({
  store,
}: HudCockpitElementSwitchProps) {
  const [open, setOpen] = useState(false);
  const [visibility, setVisibility] = useState<ElementVisibility>(
    () => store.getState().elementVisibility,
  );
  const rootRef = useRef<HTMLDivElement | null>(null);

  // Subscribe so external store changes (keyboard shortcut, cross-tab sync,
  // programmatic dispatch) flow into the checkbox state. Cleanup unsubscribes
  // on unmount — no leaks under StrictMode double-mount.
  useEffect(() => {
    const off = store.subscribe((s) => setVisibility(s.elementVisibility));
    return () => {
      off();
    };
  }, [store]);

  // Close the popover on any click outside — matches HudStyleSwitcher (P6).
  // Using `pointerdown` rather than `click` so the close fires before the
  // outside click triggers anything else (consistent with the established
  // console convention).
  useEffect(() => {
    if (!open) return;
    const onPointerDown = (event: PointerEvent) => {
      if (rootRef.current && !rootRef.current.contains(event.target as Node)) {
        setOpen(false);
      }
    };
    document.addEventListener("pointerdown", onPointerDown);
    return () => document.removeEventListener("pointerdown", onPointerDown);
  }, [open]);

  const anyHidden = COCKPIT_ELEMENT_KEYS.some((k) => !visibility[k]);

  const toggle = (key: CockpitElementKey) => {
    store.toggleElement(key);
    // Persist AFTER the dispatch so the localStorage write matches the
    // post-reduce state (avoids a 1-tick window where they're out of sync).
    persistElementVisibility(store.getState().elementVisibility);
  };

  return (
    <div className="hud-cockpit-element" ref={rootRef}>
      <button
        type="button"
        className={`hud-bar-back hud-cockpit-element-toggle${anyHidden ? " hot" : ""}`}
        onClick={() => setOpen((v) => !v)}
        title="显示元素 / Elements"
        aria-label="HUD elements"
        aria-expanded={open}
        data-testid="hud-cockpit-element-switch"
      >
        ◇ ELEMENTS
      </button>
      {open ? (
        <div
          className="hud-cockpit-element-menu"
          role="menu"
          aria-label="HUD elements"
        >
          {COCKPIT_ELEMENT_KEYS.map((key) => (
            <label
              key={key}
              className="hud-cockpit-element-option"
              data-testid={`hud-cockpit-element-row-${key}`}
            >
              <input
                type="checkbox"
                checked={visibility[key]}
                onChange={() => toggle(key)}
                data-testid={`hud-cockpit-element-${key}`}
              />
              <span>{ELEMENT_LABELS[key]}</span>
            </label>
          ))}
        </div>
      ) : null}
    </div>
  );
}