import { useEffect, useRef, useState } from "react";
import * as Cesium from "cesium";
// NOTE: widgets.css is injected into index.html by vite-plugin-cesium
// (transformIndexHtml always adds the link, even with rebuildCesium: true) —
// do not import it here or it loads twice.
import { applyBasemap, globeBase, type GlobeBase } from "../globe/basemap";
import { useAircraft, deadReckon, type AircraftPoint } from "../globe/aircraft";
import { buildSatRecs, loadSatellites, propagateAll, type SatRec } from "../globe/satellites";
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

  const SAT_CATS = ["stations", "visual", "weather", "gnss", "military"] as const;
  const [satCats, setSatCats] = useState<Set<string>>(new Set(SAT_CATS));
  const [satRecs, setSatRecs] = useState<SatRec[]>([]);
  const [satError, setSatError] = useState(false);
  const satCollection = useRef<Cesium.PointPrimitiveCollection | null>(null);
  const [selected, setSelected] = useState<GlobePick | null>(null);

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
    // Pick panel: primitive ids are GlobePick payloads (ac Task 10, sat Task 11).
    viewer.screenSpaceEventHandler.setInputAction((e: Cesium.ScreenSpaceEventHandler.PositionedEvent) => {
      const picked = viewer.scene.pick(e.position);
      const id = (picked?.id ?? null) as GlobePick | null;
      setSelected(id && (id.kind === "ac" || id.kind === "sat") ? id : null);
    }, Cesium.ScreenSpaceEventType.LEFT_CLICK);
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

  // TLE catalog load (localStorage-cached 1h in satellites.ts).
  useEffect(() => {
    let dead = false;
    loadSatellites()
      .then((tles) => {
        if (!dead) setSatRecs(buildSatRecs(tles));
      })
      .catch(() => {
        if (!dead) setSatError(true);
      });
    return () => {
      dead = true;
    };
  }, []);

  // Satellite layer: 1s SGP4 tick, rebuild collection — 500 objects is cheap.
  useEffect(() => {
    const viewer = viewerRef.current;
    if (!viewer || satRecs.length === 0) return;
    const coll = new Cesium.PointPrimitiveCollection();
    viewer.scene.primitives.add(coll);
    satCollection.current = coll;
    const SAT_COLORS: Record<string, Cesium.Color> = {
      stations: Cesium.Color.fromCssColorString("#ffd60a"),
      visual: Cesium.Color.fromCssColorString("#4fc3f7"),
      weather: Cesium.Color.fromCssColorString("#7cfc00"),
      gnss: Cesium.Color.fromCssColorString("#c792ea"),
      military: Cesium.Color.fromCssColorString("#ff3355"),
    };
    const FALLBACK = Cesium.Color.fromCssColorString("#9aa4b2");
    const t = setInterval(() => {
      if (viewer.isDestroyed()) return;
      coll.removeAll();
      const active = satRecs.filter((r) => satCats.has(r.tle.category));
      for (const p of propagateAll(active, new Date())) {
        coll.add({
          position: Cesium.Cartesian3.fromDegrees(p.lon, p.lat, p.altKm * 1000),
          pixelSize: p.category === "stations" ? 6 : 2.5,
          color: SAT_COLORS[p.category] ?? FALLBACK,
          id: { kind: "sat", name: p.name, category: p.category, noradId: p.norad_id } satisfies GlobePick,
        });
      }
    }, 1000);
    return () => {
      clearInterval(t);
      if (!viewer.isDestroyed()) viewer.scene.primitives.remove(coll);
      satCollection.current = null;
    };
  }, [satRecs, satCats]);

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
        <div className="flex flex-wrap items-center gap-1">
          {SAT_CATS.map((c) => (
            <label key={c} className="flex items-center gap-0.5">
              <input
                type="checkbox"
                checked={satCats.has(c)}
                onChange={(e) => {
                  const next = new Set(satCats);
                  if (e.target.checked) next.add(c);
                  else next.delete(c);
                  setSatCats(next);
                }}
              />
              {c}
            </label>
          ))}
          <span data-probe="sat-count">{satRecs.length}</span>
        </div>
        {satError && <div className="text-[#ff9500]">SAT CATALOG UNAVAILABLE</div>}
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
      {selected && (
        <div className="absolute right-2 top-2 w-56 rounded border border-edge bg-panel/90 p-2 text-[11px] text-ink">
          {selected.kind === "ac" ? (
            <>
              <div className="mb-1 font-semibold">{selected.ac.flight ?? selected.ac.hex}</div>
              <div className="text-dim">hex {selected.ac.hex}</div>
              <div className="text-dim">alt {Math.round(selected.ac.alt_m)} m · gs {selected.ac.gs ?? "—"} kt</div>
              <div className="text-dim">track {selected.ac.track ?? "—"}° · squawk {selected.ac.squawk ?? "—"}</div>
              {selected.ac.mil && <div className="text-[#ff9500]">MILITARY</div>}
            </>
          ) : (
            <>
              <div className="mb-1 font-semibold">{selected.name}</div>
              <div className="text-dim">NORAD {selected.noradId}</div>
              <div className="text-dim">category {selected.category}</div>
            </>
          )}
          <button className="mt-2 rounded border border-edge px-1.5 py-0.5 text-dim hover:bg-edge" onClick={() => setSelected(null)}>
            close
          </button>
        </div>
      )}
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
