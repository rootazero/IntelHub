// Basemap chain — single source of truth for Radar and the command-deck
// MonitorMap. Both pages import from here so the chain order, provider
// URLs, and tile keys stay in lockstep.
//
// Post-2026-09-15: migrated from Leaflet to MapLibre GL JS. The shape
// changed from a flat list of (url, options) to a STYLE JSON factory
// because MapLibre needs sources + layers in a single StyleSpecification.
// The chain order, key handling, and failover logic are the same as
// before; only the rendering primitive changed.
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

import type { StyleSpecification } from "maplibre-gl";

export type TileProvider = "stadia" | "carto" | "esri";
export type TilesMode = TileProvider | "offline";

export const STADIA_KEY = (import.meta.env.VITE_STADIA_KEY as string | undefined) ?? "";
export const CARTO_KEY = (import.meta.env.VITE_CARTO_KEY as string | undefined) ?? "";

const ATTRIBUTION = {
  osm: '&copy; <a href="https://www.openstreetmap.org/copyright">OpenStreetMap</a> ',
  carto: '&copy; <a href="https://carto.com/attributions">CARTO</a>',
  stadia: '&copy; <a href="https://stadiamaps.com/">Stadia Maps</a>',
  esri: '&copy; <a href="https://www.esri.com/">Esri</a> &mdash; Esri, DeLorme, NAVTEQ',
};

// Single raster source per provider. Each style JSON references the
// matching source. (Using a single source per style keeps failover
// cheap — we just swap the whole style, no need to add/remove layers.)
const SOURCES = {
  stadia: () => ({
    type: "raster" as const,
    tiles: [
      `https://tiles.stadiamaps.com/tiles/alidade_smooth_dark/{z}/{x}/{y}{r}.png?api_key=${STADIA_KEY}`,
    ],
    tileSize: 256,
    attribution: `${ATTRIBUTION.osm} ${ATTRIBUTION.stadia}`,
    maxzoom: 20,
  }),
  carto: () => ({
    type: "raster" as const,
    tiles: [
      `https://a.basemaps.cartocdn.com/rastertiles/dark_all/{z}/{x}/{y}.png?key=${CARTO_KEY}`,
      `https://b.basemaps.cartocdn.com/rastertiles/dark_all/{z}/{x}/{y}.png?key=${CARTO_KEY}`,
      `https://c.basemaps.cartocdn.com/rastertiles/dark_all/{z}/{x}/{y}.png?key=${CARTO_KEY}`,
      `https://d.basemaps.cartocdn.com/rastertiles/dark_all/{z}/{x}/{y}.png?key=${CARTO_KEY}`,
    ],
    tileSize: 256,
    attribution: `${ATTRIBUTION.osm} ${ATTRIBUTION.carto}`,
    maxzoom: 20,
  }),
  esri: () => ({
    type: "raster" as const,
    tiles: [
      "https://server.arcgisonline.com/ArcGIS/rest/services/Canvas/World_Dark_Gray_Base/MapServer/tile/{z}/{y}/{x}",
    ],
    tileSize: 256,
    attribution: ATTRIBUTION.esri,
    maxzoom: 16,
  }),
};

function buildRasterStyle(provider: TileProvider): StyleSpecification {
  return {
    version: 8,
    sources: {
      basemap: SOURCES[provider](),
    },
    layers: [
      {
        id: "basemap",
        type: "raster",
        source: "basemap",
        minzoom: 0,
        maxzoom: 22,
      },
    ],
  };
}

/**
 * Offline fallback: uses the bundled world-110m.geo.json as a GeoJSON
 * source + fill layer. No external tiles needed (good for air-gapped /
 * blocked-egress environments).
 */
export function buildOfflineStyle(): StyleSpecification {
  return {
    version: 8,
    sources: {
      world: {
        type: "geojson",
        data: "/world-110m.geo.json",
      },
    },
    layers: [
      {
        id: "world-fill",
        type: "fill",
        source: "world",
        paint: {
          "fill-color": "#1f2937",
          "fill-outline-color": "#4b5563",
        },
      },
      {
        id: "world-line",
        type: "line",
        source: "world",
        paint: {
          "line-color": "#6b7280",
          "line-width": 0.5,
        },
      },
    ],
  };
}

/**
 * Build the full MapLibre style for a provider, or the offline style
 * when the page has been advanced to the "offline" tier (page-level
 * concern, not the chain's).
 */
export function buildStyle(provider: TileProvider | "offline"): StyleSpecification {
  if (provider === "offline") return buildOfflineStyle();
  return buildRasterStyle(provider);
}

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
