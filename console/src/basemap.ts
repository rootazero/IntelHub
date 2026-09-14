// Basemap chain — single source of truth for Radar and the command-deck
// MonitorMap. Both pages import from here so the chain order, provider
// URLs, and tile keys stay in lockstep.
//
// Three providers ordered by preference (best → worst):
//   - CARTO dark_all: primary when a key is set. 5M tiles/month free for
//     non-commercial use, key at carto.com/basemaps/apikey.
//   - Esri Canvas/World_Dark_Gray_Base: always-available, no-auth second
//     fallback. Grey (not true black) but reliable.
//   - Stadia Maps (alidade_smooth_dark): last-resort third fallback when
//     a key is set. 200K credits/month, non-commercial use permitted at
//     stadiamaps.com/sign-up. Kept as final fallback only — its alidade
//     style renders worse for our OSINT overlay density than CARTO.
// Without either dark key the chain is just ["esri"] — grey only.
//
// Pages can choose to add a fourth tier (bundled offline GeoJSON) past
// the chain when both Esri and Stadia fail — that's a page-level
// concern, not a chain concern.

export type TileProvider = "stadia" | "carto" | "esri";
export type TilesMode = TileProvider | "offline";

export const STADIA_KEY = (import.meta.env.VITE_STADIA_KEY as string | undefined) ?? "";
export const CARTO_KEY = (import.meta.env.VITE_CARTO_KEY as string | undefined) ?? "";

export const PROVIDERS: Record<TileProvider, { url: string; options: L.TileLayerOptions }> = {
  stadia: {
    url: `https://tiles.stadiamaps.com/tiles/alidade_smooth_dark/{z}/{x}/{y}{r}.png?api_key=${STADIA_KEY}`,
    options: {
      attribution:
        '&copy; <a href="https://stadiamaps.com/">Stadia Maps</a> &copy; ' +
        '<a href="https://www.openstreetmap.org/copyright">OSM</a>',
      subdomains: "abcd",
      maxZoom: 20,
    },
  },
  carto: {
    url: `https://{s}.basemaps.cartocdn.com/rastertiles/dark_all/{z}/{x}/{y}.png?key=${CARTO_KEY}`,
    options: {
      attribution:
        '&copy; <a href="https://www.openstreetmap.org/copyright">OSM</a> ' +
        '&copy; <a href="https://carto.com/attributions">CARTO</a>',
      subdomains: "abcd",
      maxZoom: 20,
    },
  },
  esri: {
    url: "https://server.arcgisonline.com/ArcGIS/rest/services/Canvas/World_Dark_Gray_Base/MapServer/tile/{z}/{y}/{x}",
    options: {
      attribution:
        '&copy; <a href="https://www.esri.com/">Esri</a> &mdash; Esri, DeLorme, NAVTEQ',
      maxZoom: 16,
    },
  },
};

// Order: CARTO if keyed (best) → Esri (always, auth-free) → Stadia if keyed
// (last-resort fallback). Esri is always present as a tier-2 anchor even
// when both dark providers are keyed, so a CARTO outage can't strand us
// on Stadia's weaker rendering.
export const CHAIN: TileProvider[] = [
  ...(CARTO_KEY ? (["carto"] as const) : []),
  "esri",
  ...(STADIA_KEY ? (["stadia"] as const) : []),
];

export const PRIMARY: TileProvider = CHAIN[0];