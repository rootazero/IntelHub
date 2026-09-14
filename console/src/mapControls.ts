// Map view-controls — single source of truth for region POV presets.
// Parallel to ./basemap.ts (which holds tile-provider config); both Radar
// and the command-deck MonitorMap import from here so the regions stay in
// lockstep. Page-specific behavior (fly animation, fitBounds on mount) is
// applied via useMapView() in each page.
//
// Region latitudes are slightly trimmed so polar cap labels don't crowd;
// longitudes centered on each region so the default "world" view stays
// intact when the user clicks WORLD again. Fractional Leaflet zoomSnap
// 0.25 lets fitBounds fill any panel aspect precisely.
import type { LatLngBoundsExpression } from "leaflet";

export type RegionKey =
  | "world"
  | "americas"
  | "europe"
  | "middleEast"
  | "asiaPacific"
  | "africa";

export const REGIONS: Record<RegionKey, LatLngBoundsExpression> = {
  // Frame↔map contract: longitude spans the full 359° so no repeated
  // continent copies can ever render; only uninhabited polar/pacific
  // fringes are trimmed; every continent (incl. East Asia) stays whole.
  world: [[-58, -179], [76, 180]],
  americas: [[-55, -170], [70, -30]],
  europe: [[35, -25], [70, 60]],
  middleEast: [[10, 25], [45, 75]],
  asiaPacific: [[-15, 65], [55, 180]],
  africa: [[-35, -20], [38, 55]],
};

export const REGION_KEYS: RegionKey[] = [
  "world",
  "americas",
  "europe",
  "middleEast",
  "asiaPacific",
  "africa",
];

// Default zoom delta for the +/- buttons. Matches the map's zoomSnap 0.25
// so two clicks land on the next integer level.
export const ZOOM_STEP = 0.5;
