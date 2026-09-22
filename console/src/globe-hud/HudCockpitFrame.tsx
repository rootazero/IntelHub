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
import {
  mountCockpitViewportLock,
  type CockpitViewportLock,
  type CockpitViewportLockViewer,
} from "../gev-visual/cockpit/viewport-lock";
import { setAllPanelDragDisabled } from "../gev-visual/tail/panel-drag";
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
import { HudCockpitElementSwitch } from "./HudCockpitElementSwitch";
import { HudCockpitReplay } from "./HudCockpitReplay";
import {
  mountCockpitReplayRecorder,
  type ReplayRecorderHandle,
} from "../gev-visual/cockpit/replay-recorder";
import {
  mountCockpitReplayPlayer,
  type ReplayPlayerHandle,
} from "../gev-visual/cockpit/replay-player";

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

  // GEV §6.3: mount recorder + player inside the cockpit overlay.
  // Only active while cockpit is up; cleanup destroys both adapters
  // (interval + state release) on exit.
  const [recorder, setRecorder] = useState<ReplayRecorderHandle | null>(null);
  const [player, setPlayer] = useState<ReplayPlayerHandle | null>(null);
  useEffect(() => {
    if (!state.active) return;
    const rec = mountCockpitReplayRecorder({
      getFrame: () => {
        const info = getTrackedInfo?.();
        if (!info) return null;
        return {
          heading: info.track ?? 0,
          pitchRad: 0,
          bankRad: 0,
          altitudeFt: info.altitudeM != null ? info.altitudeM * 3.28084 : null,
          speedKt: info.velocityMps != null ? info.velocityMps * 1.94384 : null,
          vsiMps: 0,
          callsign: info.callsign ?? "",
        };
      },
    });
    const ply = mountCockpitReplayPlayer({});
    setRecorder(rec);
    setPlayer(ply);
    return () => {
      rec.destroy();
      ply.destroy();
      setRecorder(null);
      setPlayer(null);
    };
  }, [state.active, getTrackedInfo]);

  useCockpitShortcuts({
    store,
    briefing: briefing ?? null,
    vision: vision ?? null,
    onToggleHidden: store.toggleHidden,
    onNextTab,
    onPrevTab,
  });

  // P12 T7: freeze the globe while the cockpit is live — camera inputs off,
  // click/double-click/right-click selection swallowed, cursor hidden. We mount
  // only while active and let destroy() (which auto-unlocks) restore the
  // canvas on exit / viewer swap. A half-built viewer throws inside mount and
  // must not take the HUD down (P2 lesson), so we warn and degrade.
  useEffect(() => {
    if (!viewer || !state.active) return;
    let viewportLock: CockpitViewportLock;
    try {
      viewportLock = mountCockpitViewportLock(
        viewer as CockpitViewportLockViewer,
      );
    } catch (e) {
      console.warn("[HudCockpitFrame] viewport lock mount failed:", e);
      return;
    }
    viewportLock.lock();
    return () => viewportLock.destroy();
  }, [viewer, state.active]);

  // P12 T7 / R7: panel dragging under the cockpit HUD would move the detail
  // panel while the operator is flying. The adapter instances live in
  // GlobeV2's ref, so we flip the whole registry instead of one handle.
  useEffect(() => {
    setAllPanelDragDisabled(state.active);
    return () => setAllPanelDragDisabled(false);
  }, [state.active]);

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
          <HudCockpitInstruments
            instruments={instruments}
            visibility={state.elementVisibility}
          />
          <HudCockpitBriefingPanel
            briefing={briefing}
            getTrackedInfo={getTrackedInfo}
            paused={state.briefingPaused}
            onPause={store.pauseBriefing}
            onResume={store.resumeBriefing}
            tab={briefingTab}
            onTabChange={setBriefingTab}
          />
          <HudCockpitElementSwitch store={store} />
          <HudCockpitReplay store={store} recorder={recorder} player={player} />
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
