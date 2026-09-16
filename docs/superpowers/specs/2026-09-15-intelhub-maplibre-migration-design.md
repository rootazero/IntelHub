# MapLibre GL JS Migration

**Date**: 2026-09-15
**Status**: Active
**Author**: Claude (post-2026-09-15 Leaflet stuck-render incident)
**Scope**: IntelHub console map (Radar + Monitor Command Deck)

## 1. Problem

Leaflet 1.9.4 has a "stuck render" bug that surfaced across 4 separate
attempts to fix region focus / click focus / deep-link / ⌂ reset:

  1. **Deep-link → click another event**: `flyTo` no-ops when called
     after a prior `flyTo` from the same page state.
  2. **⌂ reset after deep-link**: `flyToBounds` silently no-ops.
  3. **Region button click 4+**: `fitBounds`/`setView` stops updating
     the SVG viewBox and map_pane transform.
  4. **Region fit-zoom**: Even with the manual DOM workaround
     (writing the SVG viewBox directly), the rendered view doesn't
     match the requested region — the manual viewBox computation
     is also "stuck" to the first region's geometry.

User verified the 4th attempt is still broken. Root cause confirmed:
**Leaflet's render layer is unreliable in this build, and we have
exhausted the workarounds**.

The user approved the fundamental fix: migrate the map to a
WebGL-based library without these render bugs.

## 2. Library choice

**MapLibre GL JS** (open-source Mapbox GL v1 fork).

Why not alternatives:
- **deck.gl**: visualization only; we'd still need a basemap.
- **Mapbox GL JS**: proprietary; requires API key for tiles.
- **OpenLayers**: GIS-y, has its own quirks; not WebGL.
- **Cesium**: 3D, overkill; bundle ~2MB.

MapLibre GL JS:
- WebGL-based, GPU-accelerated — no stuck-render class of bugs
- Open source (BSD-3), free, no API key needed for raster basemaps
- Same API as Mapbox GL JS (familiar)
- Vector + raster tile support
- Bundle size: ~250KB gzipped (Leaflet is ~150KB gzipped; the
  manual DOM workaround code can be deleted, netting +50KB)
- Mature, used in many OSINT tools
- Native [lng, lat] coordinate order (different from Leaflet)

## 3. API mapping (Leaflet → MapLibre GL JS)

| Concept | Leaflet 1.9.4 | MapLibre GL JS |
|---|---|---|
| Create map | `L.map(div, opts)` | `new maplibregl.Map({ container, style, center, zoom, ... })` |
| Coord order | `[lat, lng]` | `[lng, lat]` ⚠ |
| Set view | `map.setView([lat, lng], z)` | `map.jumpTo({ center: [lng, lat], zoom: z })` |
| Animated set | `map.flyTo([lat,lng], z, {duration})` | `map.flyTo({ center: [lng,lat], zoom: z, duration })` |
| Fit bounds | `map.fitBounds(bounds, {animate:false})` | `map.fitBounds([[lng,lat],[lng,lat]], {animate:false})` ⚠ lng/lat |
| Get center | `map.getCenter()` | `map.getCenter()` (returns `LngLat`) |
| Get zoom | `map.getZoom()` | `map.getZoom()` |
| Get bounds | `map.getBounds()` | `map.getBounds()` (returns `LngLatBounds`) |
| Get size | `map.getSize()` | `map.getSize()` (returns `{width, height}` — different from Leaflet's `{x, y}`!) |
| Resize | `map.invalidateSize()` | `map.resize()` |
| Events | `map.on('moveend', cb)` | `map.on('moveend', cb)` (same) |
| Add raster | `L.tileLayer(url, opts)` | raster source in `style.sources` + raster layer in `style.layers` |
| Add circle | `L.circleMarker([lat,lng], opts)` | HTML element + `new maplibregl.Marker({ element }).setLngLat([lng,lat]).addTo(map)` |
| Add tooltip | `m.bindTooltip(text, opts)` | custom HTML in marker element (MapLibre has `Popup` but no `bindTooltip` on markers) |
| Cluster | `L.markerClusterGroup()` (plugin) | GeoJSON source + cluster: true + circle layer with cluster paint |
| Attribution | `attributionControl: true` | built into style; CARTO/Stadia/Esri tiles require attribution in the style JSON |
| Zoom controls | `L.control.zoom()` or `zoomControl: false` + custom | built-in (`map.addControl(new maplibregl.NavigationControl()))` or custom |
| Popup | `L.popup()` | `new maplibregl.Popup()` |

**Coordinate-order is the #1 footgun**: every `[lat, lng]` literal
must become `[lng, lat]`. Add a `swapLatLng()` helper or just do
inline.

## 4. File-by-file change plan

### 4.1 `console/package.json`

Add `maplibre-gl` dependency. Remove `leaflet` (no longer used).
Keep `@types/leaflet` removal too (or just delete). Update version:

```json
"maplibre-gl": "^4.7.1"
```

### 4.2 `console/src/basemap.ts` (full rewrite)

The current file is 66 lines of Leaflet provider URLs + types. New
version returns a MapLibre **style JSON** instead.

```ts
export type BasemapProvider = "carto" | "esri" | "offline";

export const PROVIDERS: Record<BasemapProvider, {
  url: string;
  attribution: string;
  maxZoom: number;
}> = { ... };

export const CHAIN: BasemapProvider[] = ["carto", "esri", "offline"];

// Build a MapLibre style JSON for a given provider.
export function buildStyle(provider: BasemapProvider): maplibregl.StyleSpecification { ... }

// PR: build the offline style with the bundled world-110m.geo.json
// as a GeoJSON source + fill layer.
```

### 4.3 `console/src/useMapView.tsx` (major rewrite)

Context shape stays the same (region, setRegion, attach, zoomIn,
zoomOut, reset, flyToRegion, registerOnReset) — the **public API
is unchanged**, only the internal implementation changes from
Leaflet to MapLibre.

Changes:
- `mapRef` type changes from `L.Map | null` to `maplibregl.Map | null`
- `attach(m)` accepts a `maplibregl.Map`
- `flyToRegion(r)` uses `map.fitBounds(REGIONS[r], { padding: 20, animate: false })` — MapLibre's fitBounds is reliable
- `reset()` uses `map.fitBounds(REGIONS.world, ...)` + emits `onResetRef.current?.()` to close the drawer
- `zoomIn/zoomOut` uses `map.zoomIn()` / `map.zoomOut()`
- Size is `{ width, height }` not `{ x, y }` (update all `size.x` → `size.width`)

**Delete the manual DOM workaround code** (the SVG viewBox direct
write that was added in commit cd8df25). MapLibre's fitBounds is
reliable — no more Leaflet stuck-render class of bugs.

### 4.4 `console/src/components/MapControls.tsx` (no changes)

The component only uses `useMapView` for `region`, `flyToRegion`,
`zoomIn/zoomOut/reset`. Public API unchanged. The +/−/⌂ buttons are
just buttons; they don't talk to Leaflet/MapLible directly.

### 4.5 `console/src/components/hud/MonitorMap.tsx` (full rewrite)

Current: 340 lines of Leaflet code. New version:
- Replace `L.map()` with `new maplibregl.Map({ style: buildStyle("carto") })`
- Replace `L.circleMarker` with HTML element + `new maplibregl.Marker`
- Replace `L.markerClusterGroup` with MapLibre's GeoJSON source +
  cluster + circle layer
- Replace `m.flyTo` with `m.flyTo({ center, zoom, ... })` — note [lng, lat]
- `onSelect` prop receives the event when a circleMarker is clicked
- Detail card "open in Radar" button still works (no map changes)

### 4.6 `console/src/pages/Radar.tsx` (major rewrite)

Current: 444 lines. New version:
- Map init: `new maplibregl.Map(...)` instead of `L.map(...)`
- Markers: HTML circles + `new maplibregl.Marker` instead of `L.circleMarker`
- SSE: same `streamEvents` listener (unchanged)
- Right drawer: same `selected` state (unchanged)
- Deep-link effect: same `useEffect([deepLinkEventId, events.length])`
  but `focusOnEvent` now uses MapLibre's `flyTo`
- Click handler: same, but the Leaflet `m.on("click")` becomes the
  MapLibre marker's `addEventListener("click")` on the HTML element
- `map-init` `fit` becomes MapLibre's own fitBounds (no manual RAF + setTimeout)

### 4.7 `console/src/lib/eventFocus.ts` (no changes)

The utility takes a `map: LMap | null` and a marker event. The
MapLibre port of this utility:

```ts
import type { Map as MlMap } from "maplibre-gl";

export function focusOnEvent(
  map: MlMap | null | undefined,
  event: FocusableEvent,
  options: FocusOptions = {},
): void {
  if (options.claim) options.claim.current = true;
  options.setSelected?.(event);
  if (!map) return;
  map.flyTo({
    center: [event.lon, event.lat],  // [lng, lat] for MapLibre!
    zoom: EVENT_FOCUS_ZOOM,
    duration: EVENT_FOCUS_DURATION * 1000,  // MapLibre takes ms
  });
}
```

### 4.8 CSS (no changes)

The existing `hud.css` + `index.css` are agnostic of Leaflet vs
MapLibre — they style the page layout. MapLibre's default
attribution is styled automatically.

## 5. Risks

| Risk | Mitigation |
|---|---|
| Coordinate-order bugs ([lat,lng] vs [lng,lat]) | Code review with checklist; add `swapLatLng()` helper if many call sites |
| Bundle size growth (Leaflet 150KB → MapLibre 250KB) | Acceptable. Manual DOM workaround (4 hours of debugging) saved 50KB; net +50KB is a fair trade. |
| MapLibre style JSON typo → map blank | Add console.error on style load failure; add a basic "fallback" style (gray background) |
| WebGL not available in old browsers | Acceptable; IntelHub requires modern browser. If user reports issue, fall back to `forceCanvas: true` raster mode. |
| Cluster performance with 1000+ markers | GeoJSON source + cluster layer is the standard MapLibre pattern; performant up to ~100K points |
| Migration breaks accept-sp7 | Run after each major file change; final acceptance before merge |

## 6. Acceptance

- [ ] `cargo test --package hub-core --lib` (or equivalent) passes
  (no Rust changes; the hub-core backend is unaffected).
- [ ] `accept-sp7.py` runs 25/2/0 unchanged.
- [ ] E2E Playwright probe:
  - Click each of 8 region buttons → produces 8 distinct map views.
  - Click a circleMarker → drawer opens, map flies to event.
  - Click "open in Radar" from Monitor detail card → Radar focuses
    the event (drawer + map view).
  - Click ⌂ reset → world view, drawer closed.
  - Deep-link from Monitor → Radar lands at event with drawer.
  - All 5 above work in sequence (no "stuck render" after
    repeated navigation, the bug that triggered this migration).
- [ ] No console errors during the E2E probe.
- [ ] Bundle size: console/dist/assets/index-*.js is within 350KB
  gzipped (currently ~155KB Leaflet + 412KB app code).

## 7. Out of scope

- Migrate basemap attribution UI (MapLibre has a built-in
  `attributionControl`; we just need to ensure it shows).
- Re-tune the region preset bounds (current bounds are fine; not
  the bug we're fixing).
- Switch to vector tiles (CARTO basemap is still raster in the
  new build; can be a follow-up).
- Migrate the bunded offline `world-110m.geo.json` to MapLibre's
  GeoJSON source format (same data, just different config).

## 8. Estimated size

| File | Lines (estimate) |
|---|---|
| `console/package.json` | ±5 |
| `console/src/basemap.ts` | ~120 (full rewrite, +60) |
| `console/src/useMapView.tsx` | ~120 (rewrite, -40) |
| `console/src/components/hud/MonitorMap.tsx` | ~280 (rewrite, -60) |
| `console/src/pages/Radar.tsx` | ~360 (rewrite, -80) |
| `console/src/lib/eventFocus.ts` | ±5 (coord-order fix) |

Net: ~-100 LoC. The 50KB bundle size increase is the trade for
reliability.

## 9. Execution order

1. Add `maplibre-gl` to package.json + install + delete leaflet
2. Rewrite `basemap.ts` (style JSON factory)
3. Rewrite `useMapView.tsx` (Map type + methods)
4. Rewrite `eventFocus.ts` (coord order)
5. Rewrite `MonitorMap.tsx`
6. Rewrite `Radar.tsx`
7. Build, fix any TS errors
8. accept-sp7 + E2E probe
9. Push to origin
10. 415 verification (rebuild 415 from scratch to verify the new
    console build on a clean VM state)

The user has been clear: do the full migration now. No incremental
PRs. Land in one commit (or 2-3 if the build needs debugging).
