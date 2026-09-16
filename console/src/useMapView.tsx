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

  const value = useMemo<MapViewCtx>(
    () => ({ region, setRegion, attach, zoomIn, zoomOut, reset, registerOnReset }),
    [region, attach, zoomIn, zoomOut, reset, registerOnReset],
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
