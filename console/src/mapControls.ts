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
  | "northAmerica"
  | "southAmerica"
  | "europe"
  | "middleEast"
  | "eastAsiaPacific"
  | "southAsia"
  | "africa";

export const REGIONS: Record<RegionKey, LatLngBoundsExpression> = {
  // Frame↔map contract: longitude spans the full 359° so no repeated
  // continent copies can ever render; only uninhabited polar/pacific
  // fringes are trimmed; every continent (incl. East Asia) stays whole.
  // Region taxonomy (2026-09-15 split): "Asia Pacific" was too big
  // and "Americas" was too coarse. Split into OSINT-meaningful units —
  //   * americas → northAmerica (NAFTA/USMCA + Caribbean) +
  //                 southAmerica (Mercosur + Andean)
  //   * asiaPacific → eastAsiaPacific (APEC/印太 core: Japan, China,
  //                   Korea, Taiwan, Mongolia, ASEAN, ANZ, Pacific) +
  //                   southAsia (SAARC: India, Pakistan, Bangladesh, …)
  // The two new Asia buttons each fit comfortably in a single button
  // row, and the splits mirror the actual political/economic blocs.
  world: [[-58, -179], [76, 180]],
  // North America: contiguous US + Canada + Mexico + Central America +
  // Caribbean. Lng -170 covers Alaska (Aleutian arc) so single-button
  // nav reaches the entire US theater. South edge 10°N keeps Cuba,
  // Hispaniola, Jamaica in frame; northern 75°N reaches Hudson Bay.
  northAmerica: [[10, -170], [75, -50]],
  // South America: Panama (north ~13°N) south to Tierra del Fuego
  // (~-55°S); Lng -85 (Pacific coast of Ecuador/Peru) to -30 (Brazilian
  // bulge — NE coast including Recife and Fortaleza).
  southAmerica: [[-55, -85], [13, -30]],
  // Europe: unchanged.
  europe: [[35, -25], [70, 60]],
  // Middle East: east edge shrunk 75→62 so the Iran-Pakistan border
  // boundary cleanly hands off to southAsia at lng 60.
  middleEast: [[10, 25], [45, 62]],
  // East Asia & Pacific: lng 65 (just east of the Iran/Afghanistan/
  // Pakistan handover to southAsia) east through 180 (date line). Lat
  // -45 (southern New Zealand) to 55 (Russian Far East, north of
  // Mongolia). Excludes India, Pakistan, Bangladesh (in southAsia).
  eastAsiaPacific: [[-45, 65], [55, 180]],
  // South Asia: lng 60 (Pakistan-Iran border) east to 95 (Bangladesh,
  // Bhutan, Myanmar west edge). Lat 5 (southern Sri Lanka) to 40
  // (northern Pakistan/Kashmir). Includes India, Pakistan, Bangladesh,
  // Nepal, Bhutan, Sri Lanka, Maldives.
  southAsia: [[5, 60], [40, 95]],
  // Africa: unchanged.
  africa: [[-35, -20], [38, 55]],
};

export const REGION_KEYS: RegionKey[] = [
  "world",
  "northAmerica",
  "southAmerica",
  "europe",
  "middleEast",
  "eastAsiaPacific",
  "southAsia",
  "africa",
];

// Default zoom delta for the +/- buttons. Matches the map's zoomSnap 0.25
// so two clicks land on the next integer level.
export const ZOOM_STEP = 0.5;
