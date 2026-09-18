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
import { useEffect, useRef, useState } from "react";
import { getKey } from "../api";
import { makeApiFetch } from "../gev-adapters/http";
import { createIntelHubGlobe } from "../gev-boot/application";
import { HudBottomBar } from "../globe-hud/HudBottomBar";
import { HudFrame } from "../globe-hud/HudFrame";
import { HudDetailPanel } from "../globe-hud/HudDetailPanel";
import { HudLayerRail } from "../globe-hud/HudLayerRail";
import type { RailManager } from "../globe-hud/HudLayerRail";
import { HudTopBar } from "../globe-hud/HudTopBar";
import { useOverview } from "../globe-hud/useOverview";
import type { BasemapStack } from "../globe-hud/useActiveBasemap";
import { readPersistedStyle } from "../globe-hud/HudStyleSwitcher";
import {
  mountVisualEffects,
  type ViewerLike,
  type VisualEffectsHandle,
} from "../gev-visual/visual-effects";
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
  // T11: ONE page-level overview poll feeds both HUD bars (top: alerts;
  // bottom: collector health + counts) — see useOverview.
  const overviewState = useOverview();

  useEffect(() => {
    if (booted.current) return;
    booted.current = true;
    const globe = createIntelHubGlobe({
      // Same-origin hub API; bearer key reuses the console auth store (api.ts).
      apiFetch: makeApiFetch("", getKey() ?? ""),
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
      })
      .catch((e) => {
        if (cancelled) return;
        setError(String(e));
      });
    return () => {
      cancelled = true;
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

  if (error)
    return <div className="hud-fatal">Globe engine failed: {error}</div>;
  return (
    <HudFrame
      top={
        <HudTopBar overview={overviewState.overview} visualEffects={visualEffects} />
      }
      left={railManager ? <HudLayerRail manager={railManager} /> : null}
      right={<HudDetailPanel />}
      bottom={
        <HudBottomBar
          overview={overviewState.overview}
          manager={railManager}
          error={overviewState.error}
          viewer={sceneHandles?.viewer}
          mapStack={sceneHandles?.mapStack}
        />
      }
    />
  );
}
