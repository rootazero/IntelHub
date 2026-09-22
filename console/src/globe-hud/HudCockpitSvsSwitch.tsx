// GEV P19 SVS — cockpit HUD toggle for the Synthetic Vision System.
//
// Per §6.3 spec (D-SVS-3=C): manual toggle. Renders a `◇ SVS` button
// in the cockpit chrome that opens a popover with an on/off checkbox.
// Mirrors the P17 HudCockpitElementSwitch pattern: click-outside via
// `pointerdown`, subscribe to cockpit store, write localStorage
// round-trip.
//
// Persistence key: `intelhub.cockpit.svsEnabled`.

import { useEffect, useRef, useState } from "react";
import type {
  CockpitStore,
  CockpitStoreState,
} from "../gev-visual/cockpit/cockpit-store";
import { persistSvsEnabled, readPersistedSvsEnabled } from "../gev-visual/cockpit/svs-storage";

export function HudCockpitSvsSwitch({ store }: { store: CockpitStore }) {
  const [open, setOpen] = useState(false);
  const [state, setState] = useState<CockpitStoreState>(() => store.getState());
  const containerRef = useRef<HTMLDivElement>(null);

  useEffect(() => store.subscribe(setState), [store]);

  // Hydrate from localStorage on mount.
  useEffect(() => {
    const persisted = readPersistedSvsEnabled();
    if (persisted !== null && persisted !== state.svsEnabled) {
      store.setSvsEnabled(persisted);
    }
    // Run once on mount.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Round-trip localStorage on changes (post-reduce).
  useEffect(() => {
    persistSvsEnabled(state.svsEnabled);
  }, [state.svsEnabled]);

  // Click-outside via pointerdown.
  useEffect(() => {
    if (!open) return;
    function onPointerDown(ev: PointerEvent) {
      if (!containerRef.current) return;
      if (containerRef.current.contains(ev.target as Node)) return;
      setOpen(false);
    }
    window.addEventListener("pointerdown", onPointerDown);
    return () => window.removeEventListener("pointerdown", onPointerDown);
  }, [open]);

  const enabled = state.svsEnabled;

  return (
    <div className="hud-cockpit-svs" ref={containerRef}>
      <button
        type="button"
        className={`hud-cockpit-svs-toggle${enabled ? " hot" : ""}`}
        data-testid="hud-cockpit-svs-switch"
        onClick={() => setOpen((v) => !v)}
        aria-expanded={open}
        aria-haspopup="dialog"
      >
        ◇ SVS
      </button>
      {open && (
        <div
          role="dialog"
          className="hud-cockpit-svs-menu"
          data-testid="hud-cockpit-svs-menu"
        >
          <label className="hud-cockpit-svs-row">
            <input
              type="checkbox"
              data-testid="hud-cockpit-svs-checkbox"
              checked={enabled}
              onChange={(ev) => store.setSvsEnabled(ev.target.checked)}
            />
            <span>Wireframe terrain overlay</span>
          </label>
          <p className="hud-cockpit-svs-hint">
            Reads elevation from the active globe terrain (Cesium ion when
            available). Adds a 9×5 wireframe grid on the pitch ladder.
          </p>
        </div>
      )}
    </div>
  );
}