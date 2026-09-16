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
import L, { type Map as LMap } from "leaflet";
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
  // button is clicked.
  //
  // Background: Leaflet in this build has a "stuck render" bug — after
  // a few setView / fitBounds calls, the SVG viewBox and map_pane
  // transform stop updating (the internal getCenter / getBounds are
  // fine, but the DOM never reflects the new view). We've tried:
  //   - m.stop() before setView
  //   - requestAnimationFrame defer
  //   - map.invalidateSize(true) / invalidateSize + getSize()
  //   - map.fire('viewreset') / 'moveend'
  //   - map._resetView (private)
  // None unblock the render. So we bypass Leaflet's render path
  // entirely and write the SVG viewBox + map_pane transform directly
  // from the same math Leaflet would have used. The result is
  // guaranteed to be visible, even though it's an internal-API dance.
  const flyToRegion = useCallback((r: RegionKey) => {
    const m = mapRef.current;
    if (!m) return;
    // Update Leaflet's internal state (so getCenter/getBounds match
    // and any subsequent operations like fitBounds work from a known
    // base). The DOM update is handled by our manual code below.
    const bounds = REGIONS[r] as [[number, number], [number, number]];
    const [[sLat, wLng], [nLat, eLng]] = bounds;
    const center: L.LatLngExpression = [(sLat + nLat) / 2, (wLng + eLng) / 2];
    const size = m.getSize();
    const lngSpan = eLng - wLng;
    const latSpan = nLat - sLat;
    const zoomX = Math.log2((size.x * 360) / (lngSpan * 256));
    const zoomY = Math.log2((size.y * 180) / (latSpan * 256));
    let fitZoom = Math.max(2, Math.floor(Math.min(zoomX, zoomY)));
    const currentZoom = m.getZoom();
    if (fitZoom === currentZoom) fitZoom += 0.5;
    m.setView(center, fitZoom, { animate: false });
    setRegion(r);
    // Manual DOM update via the same math Leaflet uses internally.
    // The overlay-pane's SVG viewBox reflects the bounding box of
    // all markers in pixel coords; the map_pane's transform offsets
    // the pane so the current center is at the container's center.
    // We compute these directly because Leaflet's render path is
    // stuck (setView updates internal state but the DOM never
    // reflects it).
    const mapPane = m.getPane("mapPane") as HTMLElement | null;
    const overlayPaneDiv = m.getPane("overlayPane") as HTMLElement | null;
    // The overlay pane is a DIV containing an SVG. setAttribute on
    // the div is a no-op; we need the SVG child.
    const svg = overlayPaneDiv?.querySelector("svg") as unknown as SVGGElement | null;
    if (!mapPane || !svg) return;
    // Project the desired center to layer pixel coords at the
    // requested zoom. Then derive the viewBox top-left from the
    // container size. The map_pane transform offsets the pane so the
    // center is at the container's center.
    const centerLatLng = L.latLng(center[0] as number, center[1] as number);
    const centerPx = m.project(centerLatLng, fitZoom);
    const w = size.x;
    const h = size.y;
    const x = centerPx.x - w / 2;
    const y = centerPx.y - h / 2;
    svg.setAttribute("viewBox", `${x} ${y} ${w} ${h}`);
    svg.setAttribute("width", String(w));
    svg.setAttribute("height", String(h));
    mapPane.style.transform = `translate3d(${-x}px, ${-y}px, 0px)`;
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
