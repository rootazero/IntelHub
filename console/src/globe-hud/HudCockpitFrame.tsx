// GEV P9 cockpit frame — full-screen HUD overlay (D4: overlay, NOT globe
// replacement; no circular view mask per spec §0). Renders ONLY while
// `cockpitStore.active` is true and composes the four sub-components:
//   - HudCockpitContext      top-left readout (callsign / ICAO / alt / hdg / spd)
//   - HudCockpitInstruments  center SVG cluster (compass / altimeter / speed)
//   - HudCockpitBriefingPanel right panel (weather / summary + auto-rotate)
//   - HudCockpitVisionSwitch bottom 5-mode selector
// The exit button leaves P7 follow active (D5: 「退出驾驶舱（继续跟随）」).
import { useEffect, useState } from "react";
import type {
  CockpitStore,
  CockpitStoreState,
} from "../gev-visual/cockpit/cockpit-store";
import type {
  CockpitTrackedInfo,
  InstrumentsHandle,
} from "../gev-visual/cockpit/instruments-mount";
import type { BriefingHandle } from "../gev-visual/cockpit/briefing-mount";
import type { VisionMountHandle } from "../gev-visual/cockpit/vision-mount";
import { HudCockpitContext } from "./HudCockpitContext";
import { HudCockpitInstruments } from "./HudCockpitInstruments";
import { HudCockpitBriefingPanel } from "./HudCockpitBriefingPanel";
import { HudCockpitVisionSwitch } from "./HudCockpitVisionSwitch";

/** Subscribe a component to the cockpit store (shared by GlobeV2 + the frame). */
export function useCockpitStore(store: CockpitStore): CockpitStoreState {
  const [state, setState] = useState<CockpitStoreState>(() => store.getState());
  useEffect(() => store.subscribe(setState), [store]);
  return state;
}

export interface HudCockpitFrameProps {
  store: CockpitStore;
  /** flights.getTrackedInfo() seam for the context + briefing children. */
  getTrackedInfo?: () => CockpitTrackedInfo | null;
  instruments: InstrumentsHandle | null;
  briefing: BriefingHandle | null;
  vision: VisionMountHandle | null;
}

export function HudCockpitFrame({
  store,
  getTrackedInfo,
  instruments,
  briefing,
  vision,
}: HudCockpitFrameProps) {
  const state = useCockpitStore(store);

  if (!state.active) return null;

  return (
    <div className="hud-cockpit-frame" data-testid="hud-cockpit-frame">
      <HudCockpitContext getTrackedInfo={getTrackedInfo} />
      <HudCockpitInstruments instruments={instruments} />
      <HudCockpitBriefingPanel
        briefing={briefing}
        getTrackedInfo={getTrackedInfo}
        paused={state.briefingPaused}
        onPause={store.pauseBriefing}
        onResume={store.resumeBriefing}
      />
      <HudCockpitVisionSwitch
        vision={vision}
        mode={state.visionMode}
        onSelect={store.setVisionMode}
      />
      <button
        type="button"
        className="hud-cockpit-exit"
        data-testid="hud-cockpit-exit"
        title="退出驾驶舱（继续跟随）"
        onClick={store.exit}
      >
        退出驾驶舱（继续跟随）
      </button>
    </div>
  );
}
