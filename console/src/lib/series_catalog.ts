// Typed wrapper over `series_catalog.json` (generated at build time from
// `core/hub dump-series-catalog`). Provides compile-time-safe lookups for
// HUD gauges, charts, and any other component that needs to reference a
// signal series by ID.
//
// Build pipeline:
//   1. `core/hub dump-series-catalog > console/src/lib/series_catalog.json`
//      (called by scripts/build-console.sh before npm run build)
//   2. Vite inlines the JSON at compile time
//
// Usage:
//   import CATALOG from "./series_catalog.json";
//   import { findByNormalizedId, findByDisplayName } from "./series_catalog";
//
//   const dgs10 = findByNormalizedId("fred:DGS10_PCT");
//   if (!dgs10) throw new Error("catalog missing DGS10_PCT");
//   // dgs10.normalized_id, dgs10.unit, dgs10.display_name are all typed
//
// If the catalog changes (e.g. a series is renamed), TypeScript build will
// catch any consumer that referenced the old name directly.

import catalogData from "./series_catalog.json";

export type Unit =
  | "Percent"
  | "Usd"
  | "Bbl"
  | "K"
  | "YoyPct"
  | "SpotUsdBbl"
  | "AnomC"
  | "Ppm"
  | "Index"
  | "Count"
  | "Symbol";

export type Source =
  | "fred"
  | "eia"
  | "treasury"
  | "comtrade"
  | "gscpi"
  | "nasa"
  | "noaa"
  | "quote"
  | "sentiment";

export interface SeriesDescriptor {
  source: Source;
  upstream_id: string;
  normalized_id: string;
  display_name: string;
  unit: Unit;
  description: string;
}

interface Catalog {
  count: number;
  series: SeriesDescriptor[];
}

export const CATALOG: Readonly<Catalog> = catalogData as Catalog;

/** Linear scan; O(n) over ~25 entries. Cache if hot. */
export function findByNormalizedId(id: string): SeriesDescriptor | undefined {
  return CATALOG.series.find((s) => s.normalized_id === id);
}

/** Linear scan; O(n). */
export function findByDisplayName(name: string): SeriesDescriptor | undefined {
  return CATALOG.series.find((s) => s.display_name === name);
}

/** Filter by source. */
export function forSource(src: Source): SeriesDescriptor[] {
  return CATALOG.series.filter((s) => s.source === src);
}

/**
 * Throwing lookup — use this when the ID is REQUIRED for the page to work
 * (e.g. a HUD gauge that has no fallback). Misnaming a series then fails
 * the build instead of silently rendering empty.
 */
export function requireByNormalizedId(id: string): SeriesDescriptor {
  const d = findByNormalizedId(id);
  if (!d) {
    throw new Error(`series_catalog: required series not found: ${id}`);
  }
  return d;
}