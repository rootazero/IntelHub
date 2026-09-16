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

/** Minimum event shape required for the focus call. Generic
 *  parameter lets callers (Radar's GeoEvent, etc.) satisfy the
 *  constraint without an index signature — TS structural typing
 *  treats the index signature as the discriminator otherwise. */
export interface FocusableEvent {
  lat: number;
  lon: number;
}

/** Per-call side-effect options. */
export interface FocusOptions<E extends FocusableEvent = FocusableEvent> {
  /** Set the page's selected-event state to the focused event. */
  setSelected?: (e: E) => void;
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
export function focusOnEvent<E extends FocusableEvent>(
  map: LMap | null | undefined,
  event: E,
  options: FocusOptions<E> = {},
): void {
  if (options.claim) options.claim.current = true;
  options.setSelected?.(event);
  if (!map) return;
  // setView(animate:false), not flyTo or setView(animate:true). Two
  // Leaflet quirks drove this:
  //   1. flyTo (and setView animate:true) called on a map that already
  //      has another animation in flight silently no-ops — observed on
  //      Radar where the deep-link flyTo worked on first arrival but a
  //      marker click afterwards had no visual effect.
  //   2. flyToBounds(REGIONS.world) — world bounds span -179..180 across
  //      the antimeridian and Leaflet's fitBounds computes the same
  //      view it already has, looking like a no-op.
  // Instant snap is the right verb here: focus and reset are deliberate
  // gestures, not ambient motion. The 300ms settle race in the deep-link
  // effect is still guarded by `claim`.
  map.setView([event.lat, event.lon], EVENT_FOCUS_ZOOM, { animate: false });
}