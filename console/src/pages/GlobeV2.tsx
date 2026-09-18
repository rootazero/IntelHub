// GlobeV2 — GEV-engine globe page (T8 skeleton: HUD frame + bootstrap only;
// T9 adds the layer rail against getComponents().data.dataManager).
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
import { useEffect, useMemo, useRef, useState } from "react";
import { getKey } from "../api";
import { makeApiFetch } from "../gev-adapters/http";
import { createIntelHubGlobe } from "../gev-boot/application";
import { HudBottomBar } from "../globe-hud/HudBottomBar";
import { HudFrame } from "../globe-hud/HudFrame";
import { HudDetailPanel } from "../globe-hud/HudDetailPanel";
import { HudLayerRail } from "../globe-hud/HudLayerRail";
import type { RailManager } from "../globe-hud/HudLayerRail";
import { HudTopBar } from "../globe-hud/HudTopBar";
import { HudDrawToolbar } from "../globe-hud/HudDrawToolbar";
import { HudAnnotationList } from "../globe-hud/HudAnnotationList";
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
import "../globe-hud/hud.css";

const booted = { current: false };

// Key names follow the P1 globe build-time chain, fed by
// scripts/build-console.sh (VITE_CESIUM_ION_KEY / VITE_GOOGLE_MAPS_KEY); the
// P1 reader (src/globe/basemap.ts) was retired in T16.
const GOOGLE_KEY =
  (import.meta.env.VITE_GOOGLE_MAPS_KEY as string | undefined) ?? "";
const CESIUM_KEY =
  (import.meta.env.VITE_CESIUM_ION_KEY as string | undefined) ?? "";

export default function GlobeV2() {
  const [error, setError] = useState<string | null>(null);
  // T9: the layer rail drives the data-phase LayerLifecycle. Surfaced via
  // state (not a ref) so the rail mounts after start() resolves; StrictMode's
  // double effect keeps state, and the module-level `booted` guard ensures
  // start() runs once.
  const [railManager, setRailManager] = useState<RailManager | null>(null);
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
  // T11: ONE page-level overview poll feeds both HUD bars (top: alerts;
  // bottom: collector health + counts) — see useOverview.
  const overviewState = useOverview();
  // P7: one hub transport shared by the engine bootstrap and the top-bar
  // location search. useMemo keeps the HudTopBar prop referentially stable —
  // its self-mount effect depends on it.
  const apiFetch = useMemo(() => makeApiFetch("", getKey() ?? ""), []);

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
      })
      .catch((e) => {
        if (cancelled) return;
        setError(String(e));
      });
    return () => {
      cancelled = true;
      // P8 cleanup order (annotation → follow → camera → visual-effects →
      // globe): the annotation engine clears its board + renderer FIRST while
      // the viewer is still alive; follow is pure bookkeeping (no destroy);
      // camera destroys its animator before the viewer dies; visual-effects
      // removes post-process stages while the viewer is still alive;
      // globe.destroy() tears down the viewer last.
      annotationEngineRef.current?.destroy();
      annotationEngineRef.current = null;
      annotationIdMapRef.current.clear();
      followRef.current = null;
      setFollow(null);
      cameraRef.current?.destroy();
      cameraRef.current = null;
      setCameraOrientation(null);
      // Destroy the visual-effects adapter BEFORE engine teardown: its
      // destroy() removes stages from the live postProcessStages and restores
      // the bloom snapshot — a dead viewer would throw on remove(). React runs
      // this same-effect cleanup as one unit, so this ordering is the
      // lifecycle guarantee.
      visualEffectsRef.current?.destroy();
      visualEffectsRef.current = null;
      setVisualEffects(null);
      // Reset synchronously so StrictMode's re-setup OR a real re-entry
      // (route back to /globe after navigation) can boot fresh.
      booted.current = false;
      // scene.js defers creditContainer.remove() and viewer.destroy() inside
      // the engine's STOP_ORDER cleanup, which app.destroy() runs in LIFO.
      // This is what removes the cesium credit icon from <body> on route-leave.
      globe.destroy().catch((e) => console.warn('[GlobeV2] destroy failed:', e));
    };
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

  if (error)
    return <div className="hud-fatal">Globe engine failed: {error}</div>;
  return (
    <HudFrame
      top={
        <HudTopBar
          overview={overviewState.overview}
          visualEffects={visualEffects}
          camera={cameraOrientation}
          apiFetch={apiFetch}
          viewer={sceneHandles?.viewer}
        />
      }
      left={
        railManager ? (
          <HudLayerRail
            manager={railManager}
            onToggleDraw={() => setDrawActive((value) => !value)}
            drawActive={drawActive}
          />
        ) : null
      }
      right={<HudDetailPanel follow={follow} camera={cameraOrientation} />}
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
    </HudFrame>
  );
}
