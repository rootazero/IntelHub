import { useEffect, useRef, useState } from "react";
import * as Cesium from "cesium";
// NOTE: widgets.css is injected into index.html by vite-plugin-cesium
// (transformIndexHtml always adds the link, even with rebuildCesium: true) —
// do not import it here or it loads twice.
import { applyBasemap, globeBase, type GlobeBase } from "../globe/basemap";
import { ErrorBoundary } from "../components/ErrorBoundary";

function GlobeInner() {
  const ref = useRef<HTMLDivElement>(null);
  const [base, setBase] = useState<GlobeBase | null>(null);

  useEffect(() => {
    if (!ref.current) return;
    const viewer = new Cesium.Viewer(ref.current, {
      animation: false,
      timeline: false,
      baseLayerPicker: false,
      geocoder: false,
      homeButton: false,
      sceneModePicker: false,
      navigationHelpButton: false,
      fullscreenButton: false,
      infoBox: false,
      selectionIndicator: false,
      // No Ion token: the default Ion basemap would 401. applyBasemap()
      // adds the real imagery layer right after construction.
      baseLayer: false,
    });
    // Initial camera once, synchronously — Cesium setView is safe at init
    // (unlike Leaflet's async-fitBounds pitfall, 2026-09-13 lesson).
    viewer.camera.setView({
      destination: Cesium.Cartesian3.fromDegrees(105.0, 30.0, 22_000_000),
    });
    let dead = false;
    void applyBasemap(viewer).then((b) => {
      if (!dead) setBase(b ?? globeBase());
    });
    return () => {
      dead = true;
      viewer.destroy();
    };
  }, []);

  return (
    <div className="relative h-full w-full">
      <div ref={ref} className="absolute inset-0" />
      <div className="absolute left-2 top-2 rounded border border-edge bg-panel/85 px-2 py-1 text-[11px] text-dim">
        GLOBE · basemap: {base ?? "loading…"}
      </div>
    </div>
  );
}

export default function Globe() {
  return (
    <ErrorBoundary>
      <GlobeInner />
    </ErrorBoundary>
  );
}
