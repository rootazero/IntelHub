// Cross-page view-state — region + zoom + reset, shared by Radar and
// MonitorMap so the same preset/zoom is shown on both pages. Backed by
// React Context (one Provider in App.tsx) — set Asia on Radar, land on
// Monitor already at Asia. State is ephemeral (not persisted); both maps
// re-fitBounds on mount using current region, which is the correct
// startup behavior.
//
// The hook owns the MapLibre handle via `attach(m)` so the zoom +/-
// helpers can drive whichever map mounted. Pages keep their own local
// mapRef for non-control concerns (marker layer, event focus).
//
// Post-2026-09-15: migrated from Leaflet to MapLibre GL JS. The
// biggest API change is that MapLibre takes `[lng, lat]` (not
// `[lat, lng]` like Leaflet) — every region preset already uses
// `[lat, lng]` in this file (the source format is geographic), so the
// conversion happens at the `flyToRegion` boundary.

import { createContext, useCallback, useContext, useMemo, useRef, useState } from "react";
import type { Map as MlMap, LngLatBoundsLike } from "maplibre-gl";
import { REGIONS, ZOOM_STEP, type RegionKey } from "./mapControls";

type MapViewCtx = {
  region: RegionKey;
  setRegion: (r: RegionKey) => void;
  attach: (m: MlMap | null) => void;
  zoomIn: () => void;
  zoomOut: () => void;
  reset: () => void;
  /** Imperative fly to a region by key. MapLibre's fitBounds is robust
   *  (no stuck-render class of bugs we hit with Leaflet) so this is
   *  the only path the page needs for region navigation. */
  flyToRegion: (r: RegionKey) => void;
  /** Pages register a callback to run when the ⌂ button is clicked —
   *  e.g. Radar closes its right drawer here so a 'back to global' click
   *  also clears the focused event's detail panel. */
  registerOnReset: (cb: () => void) => () => void;
};

const Ctx = createContext<MapViewCtx | null>(null);

export function MapViewProvider({ children }: { children: React.ReactNode }) {
  const [region, setRegion] = useState<RegionKey>("world");
  const mapRef = useRef<MlMap | null>(null);
  const onResetRef = useRef<(() => void) | null>(null);

  const attach = useCallback((m: MlMap | null) => {
    mapRef.current = m;
  }, []);

  const zoomIn = useCallback(() => {
    const m = mapRef.current;
    if (!m) return;
    m.zoomIn({ duration: 0 });
  }, []);
  const zoomOut = useCallback(() => {
    const m = mapRef.current;
    if (!m) return;
    m.zoomOut({ duration: 0 });
  }, []);

  // ⌂ reset: snap to world view. animate:false (instant) per
  // previous Leaflet fix — the ⌂ button is an explicit "give me
  // world back" gesture; animation would just slow the user down.
  // setRegion('world') also updates the context state for any
  // indicator (e.g. the "active" highlight on the WORLD button).
  const reset = useCallback(() => {
    const m = mapRef.current;
    if (m) m.fitBounds(toMlBounds(REGIONS.world), { animate: false, padding: 20 });
    onResetRef.current?.();
    setRegion("world");
  }, []);

  const registerOnReset = useCallback((cb: () => void) => {
    onResetRef.current = cb;
    return () => {
      if (onResetRef.current === cb) onResetRef.current = null;
    };
  }, []);

  // Imperative fly to region. Called by MapControls when a region
  // button is clicked. setRegion() updates the context state for any
  // other consumer (e.g. the page-level region indicator). The
  // fitBounds happens here directly so it doesn't go through the
  // page's useEffect on `region`. MapLibre's fitBounds is robust —
  // no stuck-render issues.
  const flyToRegion = useCallback((r: RegionKey) => {
    const m = mapRef.current;
    if (!m) return;
    m.fitBounds(toMlBounds(REGIONS[r]), { animate: false, padding: 20 });
    setRegion(r);
  }, []);

  const value = useMemo<MapViewCtx>(
    () => ({ region, setRegion, attach, zoomIn, zoomOut, reset, registerOnReset, flyToRegion }),
    [region, attach, zoomIn, zoomOut, reset, registerOnReset, flyToRegion],
  );

  return <Ctx.Provider value={value}>{children}</Ctx.Provider>;
}

export function useMapView(): MapViewCtx {
  const ctx = useContext(Ctx);
  if (!ctx) throw new Error("useMapView must be used inside <MapViewProvider>");
  return ctx;
}

// Re-export REGIONS here too so page code can `import { useMapView, REGIONS }`
// from one place rather than two imports.
export { REGIONS };

/**
 * Convert a [[sLat, wLng], [nLat, eLng]] (Leaflet-style lat/lng
 * pairs) to MapLibre's LngLatBoundsLike ([[wLng, sLat], [eLng, nLat]],
 * i.e. swap to [lng, lat] and reverse the pair order).
 */
export function toMlBounds(
  bounds: [[number, number], [number, number]],
): LngLatBoundsLike {
  const [[sLat, wLng], [nLat, eLng]] = bounds;
  return [
    [wLng, sLat],
    [eLng, nLat],
  ];
}
