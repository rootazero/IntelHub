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
// in <MapControls> is wired to useMapView's reset() which flyToBounds
// REGIONS.world, so it's already a one-click return to global view.
//
// Pending settle timers in either page's map-init effect get the
// `claim` ref set so they don't snap the view back mid-animation.
//
// The utility is intentionally NOT a hook — pages own their mapRef and
// setSelected state. A hook would either need to plumb the ref through
// context (extra moving parts) or take a generic store (over-engineered
// for the current use case). The simple function + ref-flag pattern
// matches the existing tile / basemap / kindmeta style: pure module,
// no React, easy to reason about.

import type { Map as LMap } from "leaflet";

/** Zoom level when focusing on an event (continent level). */
export const EVENT_FOCUS_ZOOM = 5;
/** FlyTo animation duration, in seconds. */
export const EVENT_FOCUS_DURATION = 1.2;

/** Minimum event shape required for the focus call. */
export interface FocusableEvent {
  lat: number;
  lon: number;
  [k: string]: unknown;
}

/** Per-call side-effect options. */
export interface FocusOptions {
  /** Set the page's selected-event state to the focused event. */
  setSelected?: (e: FocusableEvent) => void;
  /**
   * Set this ref to `true` to claim the view against any pending
   * map-init settle timers (see Ruling R14 / 2026-09-15 race fix).
   * Pass `undefined` to skip the claim.
   */
  claim?: { current: boolean };
}

/**
 * Open the event's detail panel and fly the map to its location.
 *
 * Safe to call with `map = null` (initial mount race): the state update
 * still happens, the map call is a no-op. The next mount that owns
 * a non-null mapRef should call this again from its own useEffect.
 */
export function focusOnEvent(
  map: LMap | null | undefined,
  event: FocusableEvent,
  options: FocusOptions = {},
): void {
  if (options.claim) options.claim.current = true;
  options.setSelected?.(event);
  if (!map) return;
  map.flyTo([event.lat, event.lon], EVENT_FOCUS_ZOOM, {
    animate: true,
    duration: EVENT_FOCUS_DURATION,
  });
}