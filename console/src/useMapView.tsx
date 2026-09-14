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
};

const Ctx = createContext<MapViewCtx | null>(null);

export function MapViewProvider({ children }: { children: React.ReactNode }) {
  const [region, setRegion] = useState<RegionKey>("world");
  const mapRef = useRef<LMap | null>(null);

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
  const reset = useCallback(() => setRegion("world"), []);

  const value = useMemo<MapViewCtx>(
    () => ({ region, setRegion, attach, zoomIn, zoomOut, reset }),
    [region, attach, zoomIn, zoomOut, reset],
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
