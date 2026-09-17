// GlobeV2 — GEV-engine globe page (T8 skeleton: HUD frame + bootstrap only;
// T9 adds the layer rail against getComponents().data.dataManager).
//
// Engine singleton: Cesium viewers are heavyweight and the engine app owns
// teardown ordering, so exactly one instance is booted per page load. The
// module-level `booted` guard (not a ref) survives StrictMode's double
// mount/unmount, where a ref would be reset by the first unmount and boot a
// second viewer on the re-mount.
import { useEffect, useState } from "react";
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
import "../globe-hud/hud.css";

const booted = { current: false };

// Key names follow the P1 globe (src/globe/basemap.ts), fed at build time by
// scripts/build-console.sh (VITE_CESIUM_ION_KEY / VITE_GOOGLE_MAPS_KEY).
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
  // T11: ONE page-level overview poll feeds both HUD bars (top: alerts;
  // bottom: collector health + counts) — see useOverview.
  const overviewState = useOverview();

  useEffect(() => {
    if (booted.current) return; // StrictMode second mount skips
    booted.current = true;
    const globe = createIntelHubGlobe({
      // Same-origin hub API; bearer key reuses the console auth store (api.ts).
      apiFetch: makeApiFetch("", getKey() ?? ""),
      googleApiKey: GOOGLE_KEY,
      cesiumToken: CESIUM_KEY,
    });
    globe
      .start()
      .then(() => {
        const components = globe.getComponents() as {
          data?: { dataManager?: RailManager };
        };
        setRailManager(components?.data?.dataManager ?? null);
      })
      .catch((e) => setError(String(e)));
    // No destroy on unmount: the engine singleton outlives route switches;
    // route-leave teardown is a P3 decision.
  }, []);

  if (error)
    return <div className="hud-fatal">Globe engine failed: {error}</div>;
  return (
    <HudFrame
      top={<HudTopBar overview={overviewState.overview} />}
      left={railManager ? <HudLayerRail manager={railManager} /> : null}
      right={<HudDetailPanel />}
      bottom={
        <HudBottomBar
          overview={overviewState.overview}
          manager={railManager}
          error={overviewState.error}
        />
      }
    />
  );
}
