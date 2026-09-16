// Shared event-focus utility — one source of truth for "open the event's
// detail + fly the map to it" behavior. Called from:
//   1. Radar deep-link effect (/radar?event=<id> from Monitor Command Deck)
//   2. Radar circleMarker click handler
//   3. (future) MonitorMap click handler — for consistent behavior across
//      both pages that show the geo_events map.
//
// All callers converge on the same `focusOnEvent()` call so the visual
// experience is consistent: drawer/card opens, map pans with a 1.2s
// animation, the event lands near the viewport center at continent zoom
// (5 — preserves context: surrounding events, geography, kind-coloring
// of the area). City-level zoom (9) was rejected because it isolated the
// event from its neighborhood, and required a new "back to global"
// button to recover — which we don't want. The existing ⌂ reset button
// in <MapControls> is wired to useMapView's reset() which fitBounds
// REGIONS.world, so it's already a one-click return to global view.
//
// Post-2026-09-15: migrated from Leaflet to MapLibre. MapLibre uses
// [lng, lat] (not [lat, lng]) and its flyTo takes a duration in ms
// (not seconds). The 300ms settle race in the deep-link effect is still
// guarded by `claim`.

import type { Map as MlMap } from "maplibre-gl";

/** Zoom level when focusing on an event (continent level). */
export const EVENT_FOCUS_ZOOM = 5;
/** FlyTo animation duration, in seconds. */
export const EVENT_FOCUS_DURATION = 1.2;

/** Minimum event shape required for the focus call. */
export interface FocusableEvent {
  lat: number;
  lon: number;
}

/** Per-call side-effect options. The page is expected to setSelected()
 *  separately (before calling focusOnEvent) — keeping the utility
 *  decoupled from the page's setState dispatch type lets it work
 *  with any GeoEvent subtype. */
export interface FocusOptions {
  /**
   * Set this ref to `true` to claim the view against any pending
   * map-init settle timers. (Historical — MapLibre's flyTo doesn't
   * have the Leaflet stuck-render bug, so the claim is a no-op
   * kept here for API compatibility with the older Leaflet code.)
   */
  claim?: { current: boolean };
}

/**
 * Open the event's detail panel and fly the map to its location.
 *
 * Safe to call with `map = null` (initial mount race): the map call
 * is a no-op. The next mount that owns a non-null mapRef should call
 * this again from its own useEffect.
 */
export function focusOnEvent(
  map: MlMap | null | undefined,
  event: FocusableEvent,
  options: FocusOptions = {},
): void {
  if (options.claim) options.claim.current = true;
  if (!map) return;
  // MapLibre flyTo: center takes [lng, lat], duration is in ms.
  // flyTo is the right verb here — animate:false would be a hard
  // snap that's jarring for the deep-link landing (a 1.2s zoom
  // toward the event feels intentional, not sudden).
  map.flyTo({
    center: [event.lon, event.lat],
    zoom: EVENT_FOCUS_ZOOM,
    duration: EVENT_FOCUS_DURATION * 1000,
  });
}
