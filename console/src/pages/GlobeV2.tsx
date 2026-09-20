// GlobeV2 — GEV-engine globe page (T8 skeleton: HUD frame + bootstrap only;
// T9 adds the layer rail against getComponents().data.dataManager;
// T-P6 visual-effects, T-P7 camera/follow, T-P8 annotation, T-P9 cockpit;
// T3 (P10-T3) adds 9 tail adapters + 5 HUD components).
//
// Engine lifecycle: the engine owns cesium viewer creation AND DOM artifacts
// (`#cesium-credits` div on document.body, `#cesiumContainer` viewer target).
// On route-leave we MUST call globe.destroy() — otherwise the cesium credit
// icon leaks onto every other page (cesium ion attribution positioned at
// bottom-left per console/gev-engine/src/ui/styles/foundation.css). Scene
// teardown also releases the WebGL context and tears down data layers + the
// map stack controller.
//
// Module-level `booted` guard (not a ref): survives StrictMode's
// setup→cleanup→setup dev cycle so we don't boot the viewer twice. Reset
// synchronously in cleanup so a real route-leave (different component
// instance) boots fresh on re-entry. StrictMode dev pays a 2× boot cost; prod
// pays 1×.
//
// CLEANUP ORDER (P10-T3, mandatory):
//   tail (frame-rate + shortcuts + recording + scene-controls + share-restoration
//         + panel-drag + panel-layout + panel-disclosure + scene-sharing)
//   → cockpit (vision + briefing + instruments)
//   → annotation
//   → follow
//   → camera
//   → visual-effects
//   → globe.destroy()
//
// Tail adapters inserted FIRST; P6/P7/P8/P9 relative order untouched (per
// brief §"GlobeV2 cleanup order"). Within the tail block the order matters:
// panel-drag first (it owns the drag listeners), then panel-layout (it calls
// vendor layout fns during panel changes), then panel-disclosure (it binds
// the collapse buttons AFTER the panels exist), then shortcuts (last so it
// captures the toggle handlers wired by the HUD components above).
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { getKey } from "../api";
import { makeApiFetch } from "../gev-adapters/http";
import { createIntelHubGlobe } from "../gev-boot/application";
import { useGlobeSelection } from "../gev-boot/context-bridge";
import { HudBottomBar } from "../globe-hud/HudBottomBar";
import { HudFrame } from "../globe-hud/HudFrame";
import { HudDetailPanel } from "../globe-hud/HudDetailPanel";
import { HudLayerRail } from "../globe-hud/HudLayerRail";
import type { RailManager } from "../globe-hud/HudLayerRail";
import { HudTopBar } from "../globe-hud/HudTopBar";
import { HudDrawToolbar } from "../globe-hud/HudDrawToolbar";
import { HudAnnotationList } from "../globe-hud/HudAnnotationList";
import { HudPanelDragHandle } from "../globe-hud/HudPanelDragHandle";
import { HudScenePanel } from "../globe-hud/HudScenePanel";
import { HudRecordingControls } from "../globe-hud/HudRecordingControls";
import { HudShortcutCheatsheet } from "../globe-hud/HudShortcutCheatsheet";
import { HudFrameRateReadout } from "../globe-hud/HudFrameRateReadout";
import { useOverview } from "../globe-hud/useOverview";
import type { BasemapStack } from "../globe-hud/useActiveBasemap";
import { readPersistedStyle } from "../globe-hud/HudStyleSwitcher";
import {
  mountVisualEffects,
  type ViewerLike,
  type VisualEffectsHandle,
} from "../gev-visual/visual-effects";
import {
  mountCameraOrientation,
  type CameraOrientationHandle,
  type CameraViewerLike,
} from "../gev-visual/camera-orientation";
import {
  mountFollowController,
  type FollowHandle,
} from "../gev-visual/follow-controller";
import {
  createAnnotationStore,
  type AnnotationStore,
} from "../gev-visual/annotations/annotation-store";
import type { AnnotationEngineHandle } from "../gev-visual/annotations/annotation-engine-mount";
import type { AnnotationSpec } from "../gev-visual/annotations/draw-tool";
import {
  createCockpitStore,
  mountCockpitInstruments,
  mountCockpitBriefing,
  mountCockpitVision,
  gateStyleWhileCockpitActive,
  type CockpitTrackedInfo,
  type InstrumentsHandle,
  type BriefingHandle,
  type VisionMountHandle,
  type GatedStyleControl,
} from "../gev-visual/cockpit";
import { HudCockpitFrame, useCockpitStore } from "../globe-hud/HudCockpitFrame";
import { readPersistedVisionMode } from "../globe-hud/HudCockpitVisionSwitch";
// P10-T3: 9 tail adapters + URL hash share hook.
import {
  mountFrameRateMonitor,
  type FrameRateMonitorHandle,
} from "../gev-visual/tail/frame-rate-monitor";
import {
  bindShortcuts,
  type ShortcutsHandle,
  type ShortcutsActions,
} from "../gev-visual/tail/shortcuts";
import {
  mountRecordingControls,
  type RecordingControlsHandle,
  type RecordingHud,
} from "../gev-visual/tail/recording-controls";
import {
  mountSceneControls,
  type SceneControlsHandle,
  type SceneProjectState,
} from "../gev-visual/tail/scene-controls";
import {
  mountPanelDrag,
  type PanelDragHandle,
} from "../gev-visual/tail/panel-drag";
import {
  mountPanelLayout,
  layoutLeftPanelRail,
  layoutRightPanelRail,
  type PanelLayoutHandle,
  type PanelLayoutCallbacks,
} from "../gev-visual/tail/panel-layout";
import {
  bindPanelDisclosure,
  type PanelDisclosureHandle,
} from "../gev-visual/tail/panel-disclosure";
import {
  createSceneDialog,
  mountSceneSharing,
  type SceneDialogHandle,
  type SceneSharingMountHandle,
} from "../gev-visual/tail/scene-sharing";
import { useShareRestoration } from "../gev-visual/tail/use-share-restoration";
import { GLOBE_STYLES, type GlobeStyle } from "../gev-visual/visual-effects";
import "../globe-hud/hud.css";

const booted = { current: false };

// Key names follow the P1 globe build-time chain, fed by
// scripts/build-console.sh (VITE_CESIUM_ION_KEY / VITE_GOOGLE_MAPS_KEY); the
// P1 reader (src/globe/basemap.ts) was retired in T16.
const GOOGLE_KEY =
  (import.meta.env.VITE_GOOGLE_MAPS_KEY as string | undefined) ?? "";
const CESIUM_KEY =
  (import.meta.env.VITE_CESIUM_ION_KEY as string | undefined) ?? "";

/** FPS polling cadence for the HUD readout chip (4 Hz per P10 plan §6.7). */
const FPS_POLL_MS = 250;

/** Convenience: look up a DOM element by id, or null. */
function lookupId(id: string): HTMLElement | null {
  return typeof document !== "undefined"
    ? document.getElementById(id)
    : null;
}

export default function GlobeV2() {
  const [error, setError] = useState<string | null>(null);
  // T9: the layer rail drives the data-phase LayerLifecycle. Surfaced via
  // state (not a ref) so the rail mounts after start() resolves; StrictMode's
  // double effect keeps state, and the module-level `booted` guard ensures
  // start() runs once.
  const [railManager, setRailManager] = useState<RailManager | null>(null);
  // cctv-vendor-wire-up: shortcuts handlers (toggleCctv) run from keyboard
  // events outside the React render cycle — a stale useState closure would
  // call setEnabled on the previous dataManager. Mirror the state into a ref
  // so toggleCctv reads the live dataManager at call time.
  const railManagerRef = useRef<RailManager | null>(null);
  useEffect(() => {
    railManagerRef.current = railManager;
  }, [railManager]);
  // T14: scene handles for the live bottom-bar lanes (cursor pick + basemap
  // stack). Surfaced via state so the bars mount their Cesium wiring only
  // AFTER start() resolves — registering a ScreenSpaceEventHandler against a
  // half-built viewer is the exact failure the P2 Leaflet lesson warns about.
  const [sceneHandles, setSceneHandles] = useState<{
    viewer: unknown;
    mapStack: BasemapStack | null;
  } | null>(null);
  // T-P6: visual-effects adapter handle — surfaced via state so HudTopBar's
  // style switcher mounts only after start() resolves. A plain ref holds the
  // same handle for the SAME effect's cleanup, which must destroy it BEFORE
  // globe.destroy() (viewer dies first → postProcessStages.remove throws).
  const [visualEffects, setVisualEffects] = useState<VisualEffectsHandle | null>(null);
  const visualEffectsRef = useRef<VisualEffectsHandle | null>(null);
  // P7: camera-orientation + follow-controller handles. Same pattern as the
  // P6 visual-effects adapter — a ref holds the handle for THIS effect's
  // cleanup (camera destroy must run BEFORE globe.destroy(); follow is pure
  // bookkeeping with no engine resources), and state surfaces it to the HUD
  // so the detail/top-bar buttons mount only after start() resolves.
  const [cameraOrientation, setCameraOrientation] =
    useState<CameraOrientationHandle | null>(null);
  const cameraRef = useRef<CameraOrientationHandle | null>(null);
  const [follow, setFollow] = useState<FollowHandle | null>(null);
  const followRef = useRef<FollowHandle | null>(null);
  // P8: annotation board + store. Handles live in refs (for THIS effect's
  // cleanup + the mount/load path); the list surfaces via state so it
  // re-renders. mount() returns an ADAPTER-LOCAL id (p8-N), NOT spec.id — the
  // id map tracks spec.id → adapter id so unmount-by-spec works.
  const [drawActive, setDrawActive] = useState(false);
  const [annotations, setAnnotations] = useState<AnnotationSpec[]>([]);
  const annotationEngineRef = useRef<AnnotationEngineHandle | null>(null);
  const annotationStoreRef = useRef<AnnotationStore | null>(null);
  const annotationIdMapRef = useRef(new Map<string, string>());
  // P9: cockpit adapters + store. Handles live in refs (for THIS effect's
  // cleanup); the frame + rail consume them via state so they mount only after
  // start() resolves (same pattern as P6/P7/P8). The store is component-local
  // and seeded with the persisted vision mode (spec §3.2); it holds a lazy
  // getter for the vision handle (mounted async in start().then()) so enter()
  // can replay the persisted mode onto it (D2 re-entry fix).
  const cockpitVisionRef = useRef<VisionMountHandle | null>(null);
  const cockpitStore = useMemo(
    () =>
      createCockpitStore(
        { visionMode: readPersistedVisionMode() },
        { getVision: () => cockpitVisionRef.current },
      ),
    [],
  );
  const cockpitState = useCockpitStore(cockpitStore);
  const cockpitSelection = useGlobeSelection();
  const [cockpitInstruments, setCockpitInstruments] =
    useState<InstrumentsHandle | null>(null);
  const cockpitInstrumentsRef = useRef<InstrumentsHandle | null>(null);
  const [cockpitBriefing, setCockpitBriefing] =
    useState<BriefingHandle | null>(null);
  const cockpitBriefingRef = useRef<BriefingHandle | null>(null);
  const [cockpitVision, setCockpitVision] =
    useState<VisionMountHandle | null>(null);
  const flightsRef = useRef<(() => CockpitTrackedInfo | null) | undefined>(
    undefined,
  );
  // R5: gate the globe style picker while cockpit vision is active. The gated
  // control is passed to HudTopBar (NOT the raw visual-effects handle), so the
  // picker records the user's pick but no-ops on the live stages until exit.
  const gatedStyle = useMemo(
    () =>
      visualEffects
        ? gateStyleWhileCockpitActive(visualEffects, cockpitStore)
        : null,
    [visualEffects, cockpitStore],
  );
  const gatedStyleRef = useRef<GatedStyleControl | null>(null);
  gatedStyleRef.current = gatedStyle;
  // T11: ONE page-level overview poll feeds both HUD bars (top: alerts;
  // bottom: collector health + counts) — see useOverview.
  const overviewState = useOverview();
  // P7: one hub transport shared by the engine bootstrap and the top-bar
  // location search. useMemo keeps the HudTopBar prop referentially stable —
  // its self-mount effect depends on it.
  const apiFetch = useMemo(() => makeApiFetch("", getKey() ?? ""), []);

  // ===== P10-T3: 9 tail adapter refs + HUD state ==========================
  // Each adapter owns ONE handle in a ref; the refs are populated by the
  // start().then() callback (after engine boot) and consumed in the cleanup
  // closure (FIRST in the cleanup order, BEFORE cockpit).
  const frameRateRef = useRef<FrameRateMonitorHandle | null>(null);
  const shortcutsRef = useRef<ShortcutsHandle | null>(null);
  const recordingRef = useRef<RecordingControlsHandle | null>(null);
  const sceneControlsRef = useRef<SceneControlsHandle | null>(null);
  const panelDragRef = useRef<PanelDragHandle | null>(null);
  const panelLayoutRef = useRef<PanelLayoutHandle | null>(null);
  const panelDisclosureRef = useRef<PanelDisclosureHandle | null>(null);
  const sceneSharingRef = useRef<SceneSharingMountHandle | null>(null);
  const sceneDialogRef = useRef<SceneDialogHandle | null>(null);
  // Recording mode state — the adapter's HUD contract reads this via a
  // stable closure (recorded on mount).
  const recordingModeRef = useRef<"active" | "inactive">("inactive");
  const recordingVariantRef = useRef<"minimal" | "tactical">("minimal");
  const [recordingMode, setRecordingMode] = useState<"active" | "inactive">(
    "inactive",
  );
  // FPS polled from the vendor's #title-bar readout (4 Hz cadence).
  const [fps, setFps] = useState<number | null>(null);
  const [fpsReadoutOpen, setFpsReadoutOpen] = useState(false);
  // Scene panel + state machine (re-exported from the scene-controls
  // adapter). The panel is hidden until the operator clicks the rail icon.
  const [scenePanelOpen, setScenePanelOpen] = useState(false);
  const [sceneProject, setSceneProject] = useState<SceneProjectState | null>(
    null,
  );
  // Cheatsheet visibility — `?` toggles via shortcuts.toggleCheatsheet().
  const [shortcutsOpen, setShortcutsOpen] = useState(false);
  // URL share state (decodeShareHash on mount + on hashchange).
  const shareRestoration = useShareRestoration();
  // Stable action map for the shortcuts adapter — passed through refs so the
  // `?` pop-key can flip the cheatsheet state from outside React state
  // callbacks. The R5 gate (already wired in T2) reads cockpitStore directly;
  // we pass it through for the gate to consult.
  const styleSetterRef = useRef<(style: string) => void>(null);
  const shortcutsActionsRef = useRef<ShortcutsActions>({
    setStyle: (style: string) => styleSetterRef.current?.(style),
    dismissSearch: () => {
      // The location search lives in HudTopBar; emit a synthetic Escape
      // keyboard event so the existing onKeyDown path closes the result
      // list. Cheaper than refactoring the search handle.
      if (typeof document === "undefined") return;
      document.dispatchEvent(
        new KeyboardEvent("keydown", { key: "Escape", bubbles: true }),
      );
    },
    toggleHud: () => {
      // Cycle the HUD rail collapsed state via the engine's hud-toggle button
      // (if present). Falls back to a no-op when the engine shell isn't booted.
      lookupId("hud-toggle")?.dispatchEvent(
        new MouseEvent("click", { bubbles: true }),
      );
    },
    toggleOrbit: () => {
      // No IntelHub equivalent (vendor Cesium orbit tracker). Console.warn
      // so the operator sees the no-op rather than silent failure.
      console.warn("[shortcuts] toggleOrbit: no IntelHub equivalent");
    },
    toggleCleanView: () => {
      console.warn("[shortcuts] toggleCleanView: no IntelHub equivalent");
    },
    toggleLayers: () => {
      // Toggle the dataManager's enabled-state mass toggle (best-effort: click
      // the engine's clear-selected-layers if present).
      lookupId("clear-selected-layers")?.dispatchEvent(
        new MouseEvent("click", { bubbles: true }),
      );
    },
    cycleDetection: () => {
      console.warn("[shortcuts] cycleDetection: not wired in P9");
    },
    toggleCctv: () => {
      // cctv-vendor-wire-up: vendor's cctv-enable-btn is no longer the path
      // (our index.html is React-only and the engine binds that click via
      // cctvBindings.js to actions.toggleEnabled() which is engine-chrome
      // we replaced). Toggle the dataManager's cctv layer directly — the
      // bridge (gev-boot/cctv-bridge.ts) wraps setActiveCamera so a 3D
      // billboard click writes to contextStore and triggers the T14 popout.
      const manager = railManagerRef.current;
      if (!manager) {
        console.warn("[shortcuts] toggleCctv: dataManager not ready");
        return;
      }
      const next = !manager.isEffectivelyEnabled("cctv");
      void manager
        .setEnabled("cctv", next, { origin: "user" })
        .catch((e) => console.warn(`[shortcuts] toggleCctv failed:`, e));
    },
    toggleCheatsheet: () => setShortcutsOpen((v) => !v),
  });

  useEffect(() => {
    if (booted.current) return;
    booted.current = true;
    const globe = createIntelHubGlobe({
      // Same-origin hub API; bearer key reuses the console auth store (api.ts).
      apiFetch,
      googleApiKey: GOOGLE_KEY,
      cesiumToken: CESIUM_KEY,
    });
    // `cancelled` guards setState after unmount: globe.start() resolves
    // asynchronously, and cleanup may have run before .then fires.
    let cancelled = false;
    globe
      .start()
      .then(() => {
        if (cancelled) return;
        const components = globe.getComponents() as {
          data?: { dataManager?: RailManager };
          scene?: { viewer?: unknown; mapStackController?: BasemapStack };
        };
        setRailManager(components?.data?.dataManager ?? null);
        setSceneHandles({
          viewer: components?.scene?.viewer ?? null,
          mapStack: components?.scene?.mapStackController ?? null,
        });
        // T-P6: mount the visual-effects adapter against the live viewer.
        // PostProcessStage needs a fully-built viewer (the P2 Leaflet class
        // of failure). Guarded so an absent/mock viewer (jsdom tests, no
        // WebGL) degrades to "no switcher" instead of erroring the page.
        const viewer = components?.scene?.viewer;
        if (viewer) {
          try {
            const fx = mountVisualEffects(
              viewer as ViewerLike,
              {},
              readPersistedStyle(),
            );
            visualEffectsRef.current = fx;
            setVisualEffects(fx);
            styleSetterRef.current = (style: string) => {
              if ((GLOBE_STYLES as readonly string[]).includes(style)) {
                fx.setStyle(style as GlobeStyle);
              }
            };
          } catch (e) {
            console.warn("[GlobeV2] visual effects disabled:", e);
          }
        }
        // P7: camera-orientation adapter (tilt toggle + reset north). Reads
        // viewer.camera.heading + trackedEntity — mounted against the live
        // viewer, guarded like visual-effects for mock/partial boots.
        if (viewer) {
          try {
            const cam = mountCameraOrientation(viewer as CameraViewerLike);
            cameraRef.current = cam;
            setCameraOrientation(cam);
          } catch (e) {
            console.warn("[GlobeV2] camera orientation disabled:", e);
          }
        }
        // P7: follow controller — routes flight/satellite tracking through
        // the engine data manager's per-layer trackById APIs. Pure bookkeeping
        // (no viewer resources), but guarded for an absent dataManager
        // (mock/partial boot) so the detail panel degrades to no buttons.
        const dataManager = components?.data?.dataManager;
        if (dataManager) {
          try {
            const fc = mountFollowController(
              dataManager as unknown as Parameters<
                typeof mountFollowController
              >[0],
            );
            followRef.current = fc;
            setFollow(fc);
          } catch (e) {
            console.warn("[GlobeV2] follow controller disabled:", e);
          }
        }
        // P9: cockpit adapters. instruments needs viewer.scene.canvas + the
        // flights layer's getTrackedInfo seam; briefing needs only the hub
        // transport; vision bridges the P6 visual-effects handle. Guarded for
        // mock/partial boots (jsdom tests, no WebGL) so the cockpit degrades
        // to "no overlay" instead of erroring the page.
        const flightsLayer = (
          dataManager as unknown as {
            layers?: Map<string, { module?: { getTrackedInfo?: () => unknown } }>;
          }
        )?.layers?.get("flights")?.module;
        const getTrackedInfo = flightsLayer?.getTrackedInfo
          ? () =>
              flightsLayer.getTrackedInfo!() as CockpitTrackedInfo | null
          : undefined;
        flightsRef.current = getTrackedInfo;
        if (viewer && getTrackedInfo) {
          try {
            const ins = mountCockpitInstruments(
              viewer as { scene: { canvas?: unknown } },
              { getTrackedInfo },
            );
            cockpitInstrumentsRef.current = ins;
            setCockpitInstruments(ins);
          } catch (e) {
            console.warn("[GlobeV2] cockpit instruments disabled:", e);
          }
        }
        if (getTrackedInfo) {
          try {
            const br = mountCockpitBriefing(apiFetch);
            cockpitBriefingRef.current = br;
            setCockpitBriefing(br);
          } catch (e) {
            console.warn("[GlobeV2] cockpit briefing disabled:", e);
          }
        }
        if (visualEffectsRef.current) {
          try {
            const vs = mountCockpitVision(visualEffectsRef.current);
            cockpitVisionRef.current = vs;
            setCockpitVision(vs);
          } catch (e) {
            console.warn("[GlobeV2] cockpit vision disabled:", e);
          }
        }
        // P8: annotation engine + store + load existing. mountAnnotationEngine
        // is loaded LAZILY: its vendored graph (annotationEngine.js → resolver
        // → neighborhoodPolygons.js → an un-vendored JSON data pack) must not
        // sit in GlobeV2's static module graph, or the mocked-bootstrap test
        // (hud-frame.test.tsx) fails to transform the missing JSON import.
        // Guarded on scene.canvas (the adapter's own mount-time guard) so a
        // mock/partial boot degrades to "no annotation board" and the heavy
        // import never fires against a fake viewer.
        if (
          viewer &&
          typeof (viewer as { scene?: { canvas?: unknown } }).scene?.canvas !==
            "undefined"
        ) {
          import("../gev-visual/annotations/annotation-engine-mount")
            .then(({ mountAnnotationEngine }) => {
              if (cancelled) return;
              const engine = mountAnnotationEngine(viewer);
              annotationEngineRef.current = engine;
              const store = createAnnotationStore(apiFetch);
              annotationStoreRef.current = store;
              store
                .list()
                .then((list) => {
                  if (cancelled) return;
                  setAnnotations(list);
                  list.forEach((spec) => {
                    const aid = engine.mount(spec);
                    annotationIdMapRef.current.set(spec.id, aid);
                  });
                })
                .catch((e) =>
                  console.warn("[GlobeV2] annotation list failed:", e),
                );
            })
            .catch((e) =>
              console.warn("[GlobeV2] annotation engine disabled:", e),
            );
        }

        // ===== P10-T3: 9 tail adapters =================================
        // Mounted in the order they appear in the cleanup chain (tail first,
        // reverse for setup so first-mounted = last-destroyed; cleanup order
        // mirrors this reverse). Engine shell elements are looked up at this
        // point (after globe.start() resolves); some vendor adapters will
        // gracefully no-op if their host DOM is missing.
        try {
          // 1. frame-rate-monitor (vendor self-injects .frame-rate-readout
          // into #title-bar; no DOM contract for T3 chip).
          frameRateRef.current = mountFrameRateMonitor({
            viewer: viewer
              ? (viewer as Parameters<typeof mountFrameRateMonitor>[0]["viewer"])
              : null,
          });
        } catch (e) {
          console.warn("[GlobeV2] frame-rate monitor disabled:", e);
        }
        try {
          // 2. shortcuts (global keyboard listener; needs R5 gate via
          // cockpitStore). Mounted AFTER the recording HUD contract so
          // the recording `setMode` is wired by the time a keypress fires.
          shortcutsRef.current = bindShortcuts({
            actions: shortcutsActionsRef.current,
            cockpitStore,
          });
        } catch (e) {
          console.warn("[GlobeV2] shortcuts disabled:", e);
        }
        try {
          // 3. recording-controls (DOM injection + HUD contract). The HUD
          // contract reads/writes our refs; we mutate recordingModeRef on
          // setMode so the vendor can read the latest value.
          const recordingHud: RecordingHud = {
            getMode: () => recordingModeRef.current,
            getVariant: () => recordingVariantRef.current,
            setMode: (mode: string) => {
              const next =
                mode === "active" ? "active" : "inactive";
              recordingModeRef.current = next;
              setRecordingMode(next);
              // Toggle the body class for the sp8 check_44 acceptance hook
              // AND for the vendor's CSS selectors (recording.css).
              if (typeof document !== "undefined") {
                document.body.classList.toggle(
                  "recording-mode",
                  next === "active",
                );
              }
            },
            setVariant: (variant: string) => {
              const next =
                variant === "tactical" ? "tactical" : "minimal";
              recordingVariantRef.current = next;
            },
            visible: true,
          };
          recordingRef.current = mountRecordingControls({
            hud: recordingHud,
            syncShareState: () => {
              // Persist the current share hash when recording mode toggles
              // (vendor's expected side-effect).
              if (shareRestoration.urlState) {
                shareRestoration.persist(shareRestoration.urlState);
              }
            },
          });
        } catch (e) {
          console.warn("[GlobeV2] recording controls disabled:", e);
        }
        try {
          // 4. scene-controls (project state machine; subscribe to project
          // changes for the HUD scene panel).
          sceneControlsRef.current = mountSceneControls({
            read: () => sceneProject ?? EMPTY_SCENE_STATE,
            actions: {
              selectScene: (id: string) => setSceneProject((p) =>
                p
                  ? { ...p, selectedSceneId: String(id), selectedShotId: null }
                  : null,
              ),
              capture: () => {
                setSceneProject((p) =>
                  p
                    ? {
                        ...p,
                        shots: p.selectedSceneId
                          ? p.scenes
                              .find((s) => s.id === p.selectedSceneId)
                              ?.shots.concat([])
                              ?.slice(-1) ?? []
                          : [],
                        progress: 0,
                        status: "capturing",
                        hasRun: true,
                      }
                    : null,
                );
              },
              update: () => {
                /* no-op: scene state already mirrors adapter */
              },
              next: () => {
                setSceneProject((p) => {
                  if (!p?.selectedSceneId || !p?.selectedShotId) return p;
                  const scene = p.scenes.find(
                    (s) => s.id === p.selectedSceneId,
                  );
                  if (!scene) return p;
                  const idx = scene.shots.findIndex(
                    (s) => s.id === p.selectedShotId,
                  );
                  const next = scene.shots[idx + 1];
                  return next
                    ? { ...p, selectedShotId: next.id }
                    : p;
                });
              },
              export: () => {
                console.warn("[scene-controls] export: not wired in T3");
              },
              download: () => {
                console.warn("[scene-controls] download: not wired in T3");
              },
              import: () => {
                console.warn("[scene-controls] import: not wired in T3");
              },
              start: () => {
                setSceneProject((p) =>
                  p ? { ...p, running: true, status: "running" } : null,
                );
              },
              stop: () => {
                setSceneProject((p) =>
                  p ? { ...p, running: false, status: "stopped" } : null,
                );
              },
            },
          });
          // Bridge scene-controls notifications to our scene-project state.
          sceneControlsRef.current.onChange((notification) => {
            if (notification?.state) {
              setSceneProject(notification.state);
            }
          });
        } catch (e) {
          console.warn("[GlobeV2] scene controls disabled:", e);
        }
        // 5. share-restoration: pure hook (no setup; already active via the
        // top-of-component useShareRestoration() call).
        try {
          // 6. panel-drag (4 callback deps from panel-layout + panel-
          // disclosure; vendor owns the localStorage namespace).
          panelDragRef.current = mountPanelDrag(null, {
            syncPanelCollapseButton: (panel) => {
              // Vendor-owned refresh: forward to the disclosure adapter if
              // mounted so the collapse button icon flips.
              if (
                panel instanceof HTMLElement &&
                panelDisclosureRef.current
              ) {
                // Best-effort: synthesize a click on the disclosure button.
                const btn = panel.querySelector(
                  "[data-panel-disclosure]",
                ) as HTMLElement | null;
                btn?.dispatchEvent(
                  new MouseEvent("click", { bubbles: true }),
                );
              }
            },
            layoutRightPanels: () => {
              // Trigger a right-rail layout pass on next frame so the
              // vendor's rectangle math sees the fresh DOM.
              requestAnimationFrame(() => {
                runPanelLayoutPass("right");
              });
            },
            syncCctvPanelViewport: () => {
              console.warn(
                "[panel-drag] syncCctvPanelViewport: not wired in T3",
              );
            },
            showToast: (msg: string) => {
              console.info("[panel-drag] toast:", msg);
            },
          });
        } catch (e) {
          console.warn("[GlobeV2] panel drag disabled:", e);
        }
        try {
          // 7. panel-layout (mount + obstacle mapping). The mount returns
          // a stub handle; we ALSO call the vendor layout fns directly via
          // the re-exports (per brief §"T2 carries").
          panelLayoutRef.current = mountPanelLayout({});
          // Initial layout pass: schedule on rAF so the engine shell
          // elements (#left-panel-stack, #right-context-rail) have time to
          // be created by applicationShell.js after globe.start().
          requestAnimationFrame(() => runPanelLayoutPass("both"));
        } catch (e) {
          console.warn("[GlobeV2] panel layout disabled:", e);
        }
        try {
          // 8. panel-disclosure (collapse-button binding). Vendor requires
          // both onChange + onEscape callbacks at call time.
          const disclosureButtons = Array.from(
            document.querySelectorAll<HTMLElement>(
              "[data-panel-disclosure]",
            ),
          );
          const disclosureHandles: PanelDisclosureHandle[] = [];
          for (const btn of disclosureButtons) {
            const panel = btn.closest<HTMLElement>("[data-panel-id]");
            if (!panel) continue;
            disclosureHandles.push(
              bindPanelDisclosure({
                panel,
                buttons: [btn],
                onChange: () => {
                  // After a disclosure flip, re-layout the affected rail.
                  requestAnimationFrame(() =>
                    runPanelLayoutPass("both"),
                  );
                },
                onEscape: () => {
                  // Vendor-routed Escape: collapse + then re-layout.
                  requestAnimationFrame(() =>
                    runPanelLayoutPass("both"),
                  );
                },
              }),
            );
          }
          // Wrap the array so cleanup can iterate. We attach a sentinel
          // destroy() that clears the array (no single-handle).
          panelDisclosureRef.current = {
            destroy: () => {
              for (const h of disclosureHandles) h.destroy();
              disclosureHandles.length = 0;
            },
          };
        } catch (e) {
          console.warn("[GlobeV2] panel disclosure disabled:", e);
        }
        try {
          // 9. scene-sharing (mount edit/share button bar into the scene
          // panel; createDialog is created lazily on demand).
          const scenePanelEl = lookupId("scene-panel");
          if (scenePanelEl) {
            sceneSharingRef.current = mountSceneSharing(scenePanelEl, {
              edit: () => {
                // Lazily create the dialog so SSR-safe code paths don't
                // touch document.body before mount.
                if (!sceneDialogRef.current) {
                  sceneDialogRef.current = createSceneDialog(
                    "Edit Scene",
                    () => {
                      sceneDialogRef.current?.dispose();
                      sceneDialogRef.current = null;
                    },
                  );
                }
              },
              share: () => {
                // Persist the current share state to URL hash and fire a
                // synthetic click on the share button if it exists.
                if (shareRestoration.urlState) {
                  shareRestoration.persist(shareRestoration.urlState);
                }
                lookupId("share-btn")?.dispatchEvent(
                  new MouseEvent("click", { bubbles: true }),
                );
              },
            });
          }
        } catch (e) {
          console.warn("[GlobeV2] scene sharing disabled:", e);
        }
      })
      .catch((e) => {
        if (cancelled) return;
        setError(String(e));
      });
    return () => {
      cancelled = true;
      // Cleanup order (P10-T3, mandatory):
      //   tail (frame-rate + shortcuts + recording + scene-controls +
      //         share-restoration + panel-drag + panel-layout +
      //         panel-disclosure + scene-sharing)
      //   → cockpit (vision + briefing + instruments)
      //   → annotation
      //   → follow
      //   → camera
      //   → visual-effects
      //   → globe.destroy()

      // 1. tail adapters (FIRST; reverse order of mount).
      // The HUD contract recording (5b) and scene dialog (9b) are torn down
      // BEFORE their owning adapter so the adapter doesn't see a dangling
      // listener.
      sceneDialogRef.current?.dispose();
      sceneDialogRef.current = null;
      sceneSharingRef.current?.destroy();
      sceneSharingRef.current = null;
      panelDisclosureRef.current?.destroy();
      panelDisclosureRef.current = null;
      panelLayoutRef.current?.destroy();
      panelLayoutRef.current = null;
      panelDragRef.current?.destroy();
      panelDragRef.current = null;
      // share-restoration: hook cleanup is internal (removeEventListener on
      // unmount). No explicit destroy needed.
      sceneControlsRef.current?.destroy();
      sceneControlsRef.current = null;
      recordingRef.current?.destroy();
      recordingRef.current = null;
      shortcutsRef.current?.destroy();
      shortcutsRef.current = null;
      frameRateRef.current?.destroy();
      frameRateRef.current = null;

      // 2. cockpit (P9; vision drops its captured baseline, briefing stops
      // its rotation timer, instruments drops its frame).
      cockpitVisionRef.current?.destroy();
      cockpitVisionRef.current = null;
      setCockpitVision(null);
      cockpitBriefingRef.current?.destroy();
      cockpitBriefingRef.current = null;
      setCockpitBriefing(null);
      cockpitInstrumentsRef.current?.destroy();
      cockpitInstrumentsRef.current = null;
      setCockpitInstruments(null);
      flightsRef.current = undefined;
      // 3. annotation (P8; clears its board + renderer while viewer alive).
      annotationEngineRef.current?.destroy();
      annotationEngineRef.current = null;
      annotationIdMapRef.current.clear();
      // 4. follow (P7; pure bookkeeping, no destroy).
      followRef.current = null;
      setFollow(null);
      // 5. camera (P7; destroys its animator before viewer dies).
      cameraRef.current?.destroy();
      cameraRef.current = null;
      setCameraOrientation(null);
      // 6. visual-effects (P6; removes post-process stages while viewer alive).
      visualEffectsRef.current?.destroy();
      visualEffectsRef.current = null;
      setVisualEffects(null);
      // Reset synchronously so StrictMode's re-setup OR a real re-entry
      // (route back to /globe after navigation) can boot fresh.
      booted.current = false;
      // 7. scene.js defers creditContainer.remove() and viewer.destroy()
      // inside the engine's STOP_ORDER cleanup, which app.destroy() runs
      // in LIFO. This is what removes the cesium credit icon from <body>
      // on route-leave.
      globe.destroy().catch((e) => console.warn('[GlobeV2] destroy failed:', e));
    };
  }, []);

  // P10-T3: FPS polling — read the vendor's #title-bar > .frame-rate-readout
  // textContent every FPS_POLL_MS. Vendor pushes updates at full GPU rate;
  // we throttle to 4 Hz to avoid React reconciliation churn.
  useEffect(() => {
    if (!fpsReadoutOpen) {
      setFps(null);
      return;
    }
    const tick = () => {
      const titleBar = lookupId("title-bar");
      const readout = titleBar?.querySelector<HTMLElement>(
        ".frame-rate-readout",
      );
      if (!readout) {
        setFps(null);
        return;
      }
      const match = /(\d+)/.exec(readout.textContent ?? "");
      setFps(match ? Number(match[1]) : null);
    };
    tick();
    const timer = setInterval(tick, FPS_POLL_MS);
    return () => clearInterval(timer);
  }, [fpsReadoutOpen]);

  // P10-T3: local helper — call the vendor panel-layout functions directly
  // with the full argument list at layout time (per brief §"T2 carries").
  // The mounted mountPanelLayout handle exposes no-op layoutLeft/Right stubs
  // (T2 carry: vendor fns are re-exported); we call the vendor functions
  // here to honor the contract.
  const runPanelLayoutPass = useCallback((side: "left" | "right" | "both") => {
    if (typeof window === "undefined") return;
    const win = window;
    const doc = win.document;
    const leftStack = doc.getElementById("left-panel-stack") as HTMLElement | null;
    const rightStack = doc.getElementById("right-context-rail") as HTMLElement | null;
    // No-op if the engine shell hasn't created the stacks yet (a mock/partial
    // boot). The brief says call directly; the vendor returns early on null
    // stack so this is safe.
    const layoutCallbacks: PanelLayoutCallbacks = {
      onCollapse: (panel: HTMLElement) => {
        panel.classList.add("collapsed");
      },
      onRetry: () => {
        requestAnimationFrame(() => runPanelLayoutPass(side));
      },
      onAligned: () => {
        // No-op; cosmetic hook for the vendor's layout alignment callback.
      },
    };
    const hudPresentation = { visible: true, variant: "default" };
    const collapsedHeights = new Map<string, number>();
    const leftPanelId = leftStack?.querySelector<HTMLElement>(
      "[data-panel-id]:not(.collapsed)",
    )?.dataset.panelId ?? null;
    if ((side === "left" || side === "both") && leftStack) {
      try {
        layoutLeftPanelRail({
          stack: leftStack,
          obstacles: "all",
          windowRef: win,
          hud: hudPresentation,
          preferredPanelId: leftPanelId,
          ...layoutCallbacks,
          collapsedHeights,
        });
      } catch (e) {
        console.warn("[GlobeV2] layoutLeftPanelRail failed:", e);
      }
    }
    if ((side === "right" || side === "both") && rightStack) {
      try {
        layoutRightPanelRail({
          stack: rightStack,
          obstacles: "all",
          windowRef: win,
          hud: hudPresentation,
          preferredPanelId: null,
          ...layoutCallbacks,
          leftStack,
          displayPanel: lookupId("display-panel"),
          readDisplayScrollTop: () => {
            const dp = lookupId("display-panel");
            return dp ? dp.scrollTop : 0;
          },
          documentRef: doc,
        });
      } catch (e) {
        console.warn("[GlobeV2] layoutRightPanelRail failed:", e);
      }
    }
  }, []);

  // P8: on-select flight — stub per the plan-deferred ruling (pin-only simple
  // camera.flyTo). Line/area centroid framing (bbox radius + pitch) is a
  // P9/P10 follow-up: cameraVerbs.js is a command-driven continuous-motion
  // system, not a one-shot flyTo, so wiring it here would be premature.
  const flyToAnnotation = (spec: AnnotationSpec) => {
    if (spec.shape !== "pin" || !spec.vertices[0]) return;
    const viewer = sceneHandles?.viewer as
      | { camera?: { flyTo?: (options: { destination: unknown }) => void } }
      | undefined;
    const Cesium = (
      window as unknown as {
        __CESIUM__?: {
          Cartesian3: {
            fromDegrees: (lon: number, lat: number, height: number) => unknown;
          };
        };
      }
    ).__CESIUM__;
    if (!viewer?.camera?.flyTo || !Cesium) return;
    const { lon, lat } = spec.vertices[0];
    try {
      viewer.camera.flyTo({
        destination: Cesium.Cartesian3.fromDegrees(lon, lat, 50000),
      });
    } catch (e) {
      console.warn("[GlobeV2] annotation flyTo failed:", e);
    }
  };

  // P9 cockpit toggle (rail 🎮). Enter auto-follows the flight (D5): prefer an
  // already-tracked flight, else the current flight selection. Exit keeps the
  // P7 follow active and restores the user-picked globe style (R5).
  const onToggleCockpit = useCallback(() => {
    if (cockpitStore.getState().active) {
      cockpitStore.exit();
      gatedStyleRef.current?.restorePicked();
    } else {
      const tracked = followRef.current?.trackedId() ?? null;
      let id = tracked;
      if (
        !id &&
        cockpitSelection.kind === "flight" &&
        cockpitSelection.data?.id != null
      ) {
        id = String(cockpitSelection.data.id);
      }
      if (id && followRef.current) {
        followRef.current.follow("flight", id);
      }
      cockpitStore.enter(id ?? "");
    }
  }, [cockpitStore, cockpitSelection]);

  if (error)
    return <div className="hud-fatal">Globe engine failed: {error}</div>;
  return (
    <HudFrame
      top={
        <HudTopBar
          overview={overviewState.overview}
          visualEffects={gatedStyle}
          camera={cameraOrientation}
          apiFetch={apiFetch}
          viewer={sceneHandles?.viewer}
        >
          {fpsReadoutOpen && (
            <HudFrameRateReadout fps={fps} />
          )}
        </HudTopBar>
      }
      left={
        railManager ? (
          <HudLayerRail
            manager={railManager}
            onToggleDraw={() => setDrawActive((value) => !value)}
            drawActive={drawActive}
            onToggleCockpit={onToggleCockpit}
            cockpitActive={cockpitState.active}
            onToggleScene={() => setScenePanelOpen((v) => !v)}
            sceneActive={scenePanelOpen}
            onToggleRecording={() => {
              // Flip the recording mode through the HUD contract; the
              // adapter observes the change via setMode.
              const next = recordingModeRef.current === "active"
                ? "inactive"
                : "active";
              recordingRef.current?.controls &&
                ((recordingRef.current.controls as unknown as {
                  hud: RecordingHud;
                }).hud.setMode(next));
            }}
            recordingActive={recordingMode === "active"}
            onToggleShortcuts={() => setShortcutsOpen((v) => !v)}
            shortcutsActive={shortcutsOpen}
            onToggleFps={() => setFpsReadoutOpen((v) => !v)}
            fpsActive={fpsReadoutOpen}
          />
        ) : null
      }
      right={
        <HudDetailPanel follow={follow} camera={cameraOrientation}>
          <HudPanelDragHandle
            panelId="detail-panel"
            title="拖拽详情面板 / Drag detail panel"
            onDragStart={(id, e) =>
              panelDragRef.current?.startDrag(
                id,
                e.nativeEvent as PointerEvent,
              )
            }
            onDragEnd={() => panelDragRef.current?.endDrag()}
          />
          <HudRecordingControls
            active={recordingMode === "active"}
            onToggle={(next) => {
              recordingRef.current?.controls &&
                ((recordingRef.current.controls as unknown as {
                  hud: RecordingHud;
                }).hud.setMode(next ? "active" : "inactive"));
            }}
          />
        </HudDetailPanel>
      }
      bottom={
        <HudBottomBar
          overview={overviewState.overview}
          manager={railManager}
          error={overviewState.error}
          viewer={sceneHandles?.viewer}
          mapStack={sceneHandles?.mapStack}
        />
      }
    >
      <HudDrawToolbar
        viewer={sceneHandles?.viewer}
        apiFetch={apiFetch}
        visible={drawActive}
        onClose={() => setDrawActive(false)}
        onCreated={(spec) => {
          const aid = annotationEngineRef.current?.mount(spec);
          if (aid) annotationIdMapRef.current.set(spec.id, aid);
          setAnnotations((prev) => [spec, ...prev]);
        }}
        onError={(msg) => console.warn("[hud-draw]", msg)}
      />
      <HudAnnotationList
        viewer={sceneHandles?.viewer}
        annotations={annotations}
        visible={annotations.length > 0}
        onSelect={flyToAnnotation}
        onDelete={(id) => {
          const aid = annotationIdMapRef.current.get(id);
          if (aid) {
            annotationEngineRef.current?.unmount(aid);
            annotationIdMapRef.current.delete(id);
          }
          setAnnotations((prev) => prev.filter((a) => a.id !== id));
          void annotationStoreRef.current
            ?.remove(id)
            .catch((e) =>
              console.warn("[GlobeV2] annotation delete failed:", e),
            );
        }}
      />
      <HudCockpitFrame
        store={cockpitStore}
        getTrackedInfo={flightsRef.current}
        instruments={cockpitInstruments}
        briefing={cockpitBriefing}
        vision={cockpitVision}
      />
      <HudScenePanel
        visible={scenePanelOpen}
        scenes={
          sceneProject?.scenes.map((s) => ({
            id: s.id,
            title: s.title,
            shots: s.shots.map((sh) => ({ id: sh.id, title: sh.title })),
          })) ?? []
        }
        selectedSceneId={sceneProject?.selectedSceneId ?? null}
        selectedShotId={sceneProject?.selectedShotId ?? null}
        capturing={sceneProject?.running ?? false}
        onCapture={() => sceneControlsRef.current?.controls &&
          ((sceneControlsRef.current.controls as unknown as {
            actions: { capture: () => void };
          }).actions.capture())}
        onShare={() => sceneControlsRef.current?.controls &&
          ((sceneControlsRef.current.controls as unknown as {
            actions: { export: () => void };
          }).actions.export())}
        onClose={() => setScenePanelOpen(false)}
        status={sceneProject?.status ?? null}
      />
      <HudShortcutCheatsheet
        visible={shortcutsOpen}
        onClose={() => setShortcutsOpen(false)}
      />
    </HudFrame>
  );
}

/** Initial empty project state for the scene-controls adapter (read()). */
const EMPTY_SCENE_STATE: SceneProjectState = {
  scenes: [],
  selectedSceneId: null,
  selectedShotId: null,
  running: false,
  hasRun: false,
  progress: 0,
  status: "idle",
  runtime: "0s",
  playbackActive: false,
  keyboardEnabled: false,
};