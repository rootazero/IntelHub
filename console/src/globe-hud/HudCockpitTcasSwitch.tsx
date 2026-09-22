// GEV P20 TCAS — cockpit HUD toggle for the Traffic Collision Avoidance
// System. Per §6.3 spec (D-TCAS-3=C): manual toggle. Renders a
// `◇ TCAS` button in the cockpit chrome that opens a popover with an
// on/off checkbox. Mirrors P17 HudCockpitElementSwitch pattern.
// Persistence key: `intelhub.cockpit.tcasEnabled`.

import { useEffect, useRef, useState } from "react";
import type {
  CockpitStore,
  CockpitStoreState,
} from "../gev-visual/cockpit/cockpit-store";
import {
  persistTcasEnabled,
  readPersistedTcasEnabled,
} from "../gev-visual/cockpit/tcas-storage";

export function HudCockpitTcasSwitch({ store }: { store: CockpitStore }) {
  const [open, setOpen] = useState(false);
  const [state, setState] = useState<CockpitStoreState>(() => store.getState());
  const containerRef = useRef<HTMLDivElement>(null);

  useEffect(() => store.subscribe(setState), [store]);

  // Hydrate from localStorage on mount.
  useEffect(() => {
    const persisted = readPersistedTcasEnabled();
    if (persisted !== null && persisted !== state.tcasEnabled) {
      store.setTcasEnabled(persisted);
    }
    // Run once on mount.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Round-trip localStorage on changes (post-reduce).
  useEffect(() => {
    persistTcasEnabled(state.tcasEnabled);
  }, [state.tcasEnabled]);

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

  const enabled = state.tcasEnabled;

  return (
    <div className="hud-cockpit-tcas-switch" ref={containerRef}>
      <button
        type="button"
        className={`hud-cockpit-tcas-toggle${enabled ? " hot" : ""}`}
        data-testid="hud-cockpit-tcas-switch"
        onClick={() => setOpen((v) => !v)}
        aria-expanded={open}
        aria-haspopup="dialog"
      >
        ◇ TCAS
      </button>
      {open && (
        <div
          role="dialog"
          className="hud-cockpit-tcas-menu"
          data-testid="hud-cockpit-tcas-menu"
        >
          <label className="hud-cockpit-tcas-row">
            <input
              type="checkbox"
              data-testid="hud-cockpit-tcas-checkbox"
              checked={enabled}
              onChange={(ev) => store.setTcasEnabled(ev.target.checked)}
            />
            <span>Traffic proximity warnings</span>
          </label>
          <p className="hud-cockpit-tcas-hint">
            Queries <code>/api/v1/flights/near</code> every 1 s at 5 nm
            radius. Color-coded diamonds: white = monitor, amber =
            caution (≤2 nm, ≥100 kt closure), red = warning (≤1 nm,
            ≥250 kt closure).
          </p>
        </div>
      )}
    </div>
  );
}