// GEV P9 cockpit frame — full-screen HUD overlay (D4: overlay, NOT globe
// replacement; no circular view mask per spec §0). Renders ONLY while
// `cockpitStore.active` is true and composes the four sub-components:
//   - HudCockpitContext      top-left readout (callsign / ICAO / alt / hdg / spd)
//   - HudCockpitInstruments  center SVG cluster (compass / altimeter / speed)
//   - HudCockpitBriefingPanel right panel (weather / summary + auto-rotate)
//   - HudCockpitVisionSwitch bottom 5-mode selector
// The exit button leaves P7 follow active (D5: 「退出驾驶舱（继续跟随）」).
import { useCallback, useEffect, useState } from "react";
import type {
  CockpitStore,
  CockpitStoreState,
} from "../gev-visual/cockpit/cockpit-store";
import {
  mountCockpitCameraTransition,
  type CockpitCameraTransition,
  type CockpitCameraTransitionDeps,
} from "../gev-visual/cockpit/camera-transition";
import type {
  CockpitTrackedInfo,
  InstrumentsHandle,
} from "../gev-visual/cockpit/instruments-mount";
import type { BriefingHandle } from "../gev-visual/cockpit/briefing-mount";
import type { VisionMountHandle } from "../gev-visual/cockpit/vision-mount";
import { useCockpitShortcuts } from "../gev-visual/cockpit/shortcuts";
import { useT } from "../i18n";
import { HudCockpitContext } from "./HudCockpitContext";
import { HudCockpitInstruments } from "./HudCockpitInstruments";
import {
  HudCockpitBriefingPanel,
  type CockpitBriefingTab,
} from "./HudCockpitBriefingPanel";
import { HudCockpitVisionSwitch } from "./HudCockpitVisionSwitch";

/** Subscribe a component to the cockpit store (shared by GlobeV2 + the frame). */
export function useCockpitStore(store: CockpitStore): CockpitStoreState {
  const [state, setState] = useState<CockpitStoreState>(() => store.getState());
  useEffect(() => store.subscribe(setState), [store]);
  return state;
}

export interface HudCockpitFrameProps {
  store: CockpitStore;
  /** Engine Cesium viewer — drives the enter/exit camera transition (T6).
   *  Surfaced by the page only after start() resolves (P2 lesson), so it may
   *  be null on the first render. */
  viewer?: unknown;
  /** flights.getTrackedInfo() seam for the context + briefing children. */
  getTrackedInfo?: () => CockpitTrackedInfo | null;
  instruments: InstrumentsHandle | null;
  briefing: BriefingHandle | null;
  vision: VisionMountHandle | null;
}

export function HudCockpitFrame({
  store,
  viewer,
  getTrackedInfo,
  instruments,
  briefing,
  vision,
}: HudCockpitFrameProps) {
  const state = useCockpitStore(store);
  const { t } = useT();
  // T6 camera transition: mounted once the viewer exists and torn down when it
  // goes away. Held in state so the enter/exit effect re-runs if the viewer
  // arrives after the cockpit did.
  const [cameraTransition, setCameraTransition] =
    useState<CockpitCameraTransition | null>(null);
  // Tab / Shift+Tab drive the briefing tab. The panel stays uncontrolled when
  // no tab prop is passed, so this is additive.
  const [briefingTab, setBriefingTab] = useState<CockpitBriefingTab>("weather");
  const onNextTab = useCallback(
    () => setBriefingTab((tab) => (tab === "weather" ? "summary" : "weather")),
    [],
  );
  const onPrevTab = useCallback(
    () => setBriefingTab((tab) => (tab === "summary" ? "weather" : "summary")),
    [],
  );

  useCockpitShortcuts({
    store,
    briefing: briefing ?? null,
    vision: vision ?? null,
    onToggleHidden: store.toggleHidden,
    onNextTab,
    onPrevTab,
  });

  useEffect(() => {
    if (!viewer) return;
    let transition: CockpitCameraTransition;
    try {
      transition = mountCockpitCameraTransition({
        viewer: viewer as CockpitCameraTransitionDeps["viewer"],
      });
    } catch (e) {
      // A half-built viewer (mock / pre-start frame) must not take the HUD
      // down — the P2 Leaflet class of failure is exactly this.
      console.warn("[HudCockpitFrame] camera transition mount failed:", e);
      return;
    }
    setCameraTransition(transition);
    return () => {
      transition.destroy();
      setCameraTransition(null);
    };
  }, [viewer]);

  // Enter: fly to the tracked aircraft one frame AFTER the store flips active,
  // so the vendor follow controller has already applied its camera frame and
  // our fly is a short delta on top of a settled pose (spec R4). Exit: fly back
  // to the pre-cockpit pose — decoupled from store.exit() itself (which the
  // keyboard handler and the exit button drive).
  useEffect(() => {
    if (!cameraTransition) return;
    if (state.active && state.trackedId) {
      const raf = requestAnimationFrame(() => {
        const info = getTrackedInfo?.();
        if (
          info?.longitude != null &&
          info?.latitude != null &&
          info?.altitudeM != null
        ) {
          void cameraTransition.flyToTracked({
            longitude: info.longitude,
            latitude: info.latitude,
            altitude: info.altitudeM,
          });
        }
      });
      return () => cancelAnimationFrame(raf);
    }
    if (!state.active) {
      void cameraTransition.flyBackToBaseline();
    }
  }, [state.active, state.trackedId, cameraTransition, getTrackedInfo]);

  if (!state.active) return null;

  return (
    <div
      className="hud-cockpit-frame"
      data-testid="hud-cockpit-frame"
      data-hidden={state.hidden ? "true" : undefined}
    >
      {state.hidden ? null : (
        <>
          <HudCockpitContext getTrackedInfo={getTrackedInfo} />
          <HudCockpitInstruments instruments={instruments} />
          <HudCockpitBriefingPanel
            briefing={briefing}
            getTrackedInfo={getTrackedInfo}
            paused={state.briefingPaused}
            onPause={store.pauseBriefing}
            onResume={store.resumeBriefing}
            tab={briefingTab}
            onTabChange={setBriefingTab}
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
        </>
      )}
      <div
        className="hud-cockpit-shortcut-hint"
        data-testid="hud-cockpit-shortcut-hint"
      >
        {state.hidden
          ? t("cockpit.shortcut.hintHidden")
          : t("cockpit.shortcut.hint")}
      </div>
    </div>
  );
}
