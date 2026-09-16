import { useEffect, useRef, useState } from "react";
import * as Cesium from "cesium";
// NOTE: widgets.css is injected into index.html by vite-plugin-cesium
// (transformIndexHtml always adds the link, even with rebuildCesium: true) —
// do not import it here or it loads twice.
import { applyBasemap, globeBase, type GlobeBase } from "../globe/basemap";
import { useAircraft, deadReckon, type AircraftPoint } from "../globe/aircraft";
import { ErrorBoundary } from "../components/ErrorBoundary";

// Pick payloads carried on Cesium primitives; ScreenSpaceEventHandler picks
// these ids back out (Task 11 adds the sat variant for the satellite layer).
type GlobePick = { kind: "ac"; ac: AircraftPoint } | { kind: "sat"; name: string; category: string; noradId: number };

function GlobeInner() {
  const ref = useRef<HTMLDivElement>(null);
  const [base, setBase] = useState<GlobeBase | null>(null);
  const [showMilOnly, setShowMilOnly] = useState(false);
  const [showAircraft, setShowAircraft] = useState(true);
  const { snap, fetchedAt } = useAircraft();
  const acCollection = useRef<Cesium.PointPrimitiveCollection | null>(null);
  const viewerRef = useRef<Cesium.Viewer | null>(null);

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
    viewerRef.current = viewer;
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
      viewerRef.current = null;
      acCollection.current = null;
      viewer.destroy();
    };
  }, []);

  // Aircraft layer: rebuild points per snapshot; 1s dead-reckoning tick.
  useEffect(() => {
    const viewer = viewerRef.current;
    if (!viewer || !snap) return;
    if (acCollection.current) viewer.scene.primitives.remove(acCollection.current);
    acCollection.current = null;
    if (!showAircraft || snap.stale) return;
    const coll = new Cesium.PointPrimitiveCollection();
    const list = showMilOnly ? snap.aircraft.filter((a) => a.mil) : snap.aircraft;
    for (const ac of list) {
      coll.add({
        position: Cesium.Cartesian3.fromDegrees(ac.lon, ac.lat, ac.alt_m),
        pixelSize: ac.mil ? 5 : 3,
        color: ac.squawk === "7700"
          ? Cesium.Color.fromCssColorString("#ff3355")
          : ac.mil
            ? Cesium.Color.fromCssColorString("#ff9500")
            : Cesium.Color.fromCssColorString("#4fc3f7"),
        id: { kind: "ac", ac } satisfies GlobePick,
      });
    }
    viewer.scene.primitives.add(coll);
    acCollection.current = coll;
    return () => {
      if (!viewer.isDestroyed()) viewer.scene.primitives.remove(coll);
    };
  }, [snap, showAircraft, showMilOnly]);

  // 1s extrapolation between 15s polls (deadReckon caps at 240s, so an
  // idle tab never runs a track off the globe).
  useEffect(() => {
    const t = setInterval(() => {
      const coll = acCollection.current;
      if (!coll || !snap || snap.stale) return;
      const dt = (Date.now() - fetchedAt) / 1000;
      const list = showMilOnly ? snap.aircraft.filter((a) => a.mil) : snap.aircraft;
      for (let i = 0; i < coll.length && i < list.length; i++) {
        const [la, lo] = deadReckon(list[i], dt);
        coll.get(i).position = Cesium.Cartesian3.fromDegrees(lo, la, list[i].alt_m);
      }
    }, 1000);
    return () => clearInterval(t);
  }, [snap, fetchedAt, showMilOnly]);

  return (
    <div className="relative h-full w-full">
      <div ref={ref} className="absolute inset-0" />
      <div className="absolute left-2 top-2 space-y-1 rounded border border-edge bg-panel/85 px-2 py-1 text-[11px] text-dim">
        <div>GLOBE · basemap: {base ?? "loading…"}</div>
        <label className="flex items-center gap-1">
          <input type="checkbox" checked={showAircraft} onChange={(e) => setShowAircraft(e.target.checked)} />
          aircraft <span data-probe="aircraft-count">{snap?.stale ? 0 : (snap?.count ?? 0)}</span>
        </label>
        <label className="flex items-center gap-1">
          <input type="checkbox" checked={showMilOnly} onChange={(e) => setShowMilOnly(e.target.checked)} />
          military only
        </label>
        {snap?.stale && <div className="font-semibold text-[#ff9500]">ADS-B STALE</div>}
        {snap && !snap.stale && snap.last_tick && (
          <div>
            last: {snap.last_tick}
            {(() => {
              const t = snap.ts ? new Date(snap.ts).getTime() : NaN;
              return Number.isFinite(t)
                ? ` · ${Math.max(0, Math.round((Date.now() - t) / 1000))}s ago`
                : "";
            })()}
          </div>
        )}
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
