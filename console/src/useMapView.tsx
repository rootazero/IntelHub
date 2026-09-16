// Cross-page view-state — region + zoom + reset, shared by Radar and
// MonitorMap so the same preset/zoom is shown on both pages. Backed by
// React Context (one Provider in App.tsx) — set Asia on Radar, land on
// Monitor already at Asia. State is ephemeral (not persisted); both maps
// re-fitBounds on mount using current region, which is the correct
// startup behavior.
//
// The hook owns the Leaflet handle via `attach(m)` so the zoom +/-
// helpers can drive whichever map mounted. Pages keep their own local
// mapRef for non-control concerns (tile layer, marker group).
import { createContext, useCallback, useContext, useMemo, useRef, useState } from "react";
import type { Map as LMap } from "leaflet";
import { REGIONS, ZOOM_STEP, type RegionKey } from "./mapControls";

type MapViewCtx = {
  region: RegionKey;
  setRegion: (r: RegionKey) => void;
  attach: (m: LMap | null) => void;
  zoomIn: () => void;
  zoomOut: () => void;
  reset: () => void;
  /** Imperatively fly to a region by key. We use this instead of a
   *  useEffect on `region` because Leaflet has a 'frozen viewBox'
   *  bug where the SVG viewBox stops updating after 4-5 fitBounds/
   *  setView calls in quick succession. Calling setView directly from
   *  the button click (rather than from a useEffect re-run) sidesteps
   *  the cache. */
  flyToRegion: (r: RegionKey) => void;
  /** Pages register a callback to run when the ⌂ button is clicked —
   *  e.g. Radar closes its right drawer here so a 'back to global' click
   *  also clears the focused event's detail panel. */
  registerOnReset: (cb: () => void) => () => void;
};

const Ctx = createContext<MapViewCtx | null>(null);

export function MapViewProvider({ children }: { children: React.ReactNode }) {
  const [region, setRegion] = useState<RegionKey>("world");
  const mapRef = useRef<LMap | null>(null);
  const onResetRef = useRef<(() => void) | null>(null);

  const attach = useCallback((m: LMap | null) => {
    mapRef.current = m;
  }, []);
  const zoomIn = useCallback(() => {
    const m = mapRef.current;
    if (!m) return;
    m.setZoom(m.getZoom() + ZOOM_STEP);
  }, []);
  const zoomOut = useCallback(() => {
    const m = mapRef.current;
    if (!m) return;
    m.setZoom(m.getZoom() - ZOOM_STEP);
  }, []);
  // Reset is unconditional: after focusOnEvent() zooms the map to a
  // continent but leaves the `region` state as "world", a plain
  // setRegion("world") would be a no-op (state already === "world").
  // We setView directly so the click always returns the user to world
  // view, regardless of whether the page's region state agrees.
  // animate:false because Leaflet's flyTo/flyToBounds/animate:true
  // silently no-ops when called after another animation (the well-
  // known "second animation no-op" Leaflet bug). The ⌂ button is an
  // explicit "give me world back" gesture; instant snap is the right
  // verb.
  const reset = useCallback(() => {
    const m = mapRef.current;
    if (m) m.setView([25, 10], 2, { animate: false });
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
  // other consumer (e.g. the page-level region indicator). The setView
  // happens here directly so it doesn't go through the page's
  // useEffect on `region` (which has the Leaflet 'frozen viewBox' bug).
  const flyToRegion = useCallback((r: RegionKey) => {
    const m = mapRef.current;
    if (!m) return;
    const bounds = REGIONS[r] as [[number, number], [number, number]];
    const [[sLat, wLng], [nLat, eLng]] = bounds;
    const center: L.LatLngExpression = [(sLat + nLat) / 2, (wLng + eLng) / 2];
    // Use a fixed zoom of 2 for every region. The computed "best fit"
    // zoom (log2 of map size / bounds) was producing zoom 2 for the
    // large regions (ASIA PAC, SOUTH ASIA, AFRICA) — same as the
    // initial world zoom — and Leaflet's setView then no-op'd because
    // "we're already at zoom 2". A uniform zoom 2 + center+pan gives
    // a quick overview that matches the visual size of the "world"
    // view; users zoom in with the + button when they want detail.
    m.stop();
    m.setView(center, 2, { animate: false });
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
