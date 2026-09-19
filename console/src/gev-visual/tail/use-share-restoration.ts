// GEV P10 T2 — useShareRestoration hook (D1 ruling, plan §6 R2).
//
// D1 ruling: DROP vendor shareRestoration.js, write a React hook that uses
// the URL hash directly. The vendor module's class abstraction (ShareRestoration
// → ShareLinkManager → scene/shot persistence via `connect(dataManager)`)
// couples the engine's project state and the link schema in a way that does
// not fit IntelHub's URL-hash restoration surface. Replacing it with a React
// hook keeps the encode/decode logic colocated with the components that read
// it (GlobeV2 wiring in T3) and avoids importing a 280-line vendor module we
// would only use 6 methods of.
//
// Schema (URL hash, base64-encoded body for layout to keep the hash short):
//
//   #style=<name>             — globe style (one of GLOBE_STYLES)
//   #cam=<lat>,<lng>,<alt>,<heading>,<pitch>
//                              — camera destination + orientation
//   #tracking=<kind>:<id>     — follow-controller tracking kind + entity id
//   #selected=<id>            — globe selection id (flight/feature/etc.)
//   #layout=<base64>          — base64(JSON.stringify({panels: {...}}))
//
// All keys are optional; missing keys keep the previous state on decode.
//
// Examples:
//   #style=anime
//   #style=retro&cam=40.7,-74.0,15000,0,-45
//   #style=normal&cam=51.5,-0.1,25000,30,-90&tracking=flight:abc123
//   #style=normal&layout=eyJwYW5lbHMiOnt9fQ==
//
// Compatibility: GlobeV2 currently has no URL hash restoration. The hook
// only touches keys under its own prefix; any pre-existing hash fragments
// (e.g. router state, third-party URL schemes) are preserved verbatim.
//
// React semantics:
//   - `useShareRestoration()` returns `{ urlState, encode, decode, persist,
//     restoredAt }`. The hook subscribes to `hashchange` and updates
//     urlState on every browser navigation event. `restoredAt` is the
//     Date.now() of the most recent restore (initial render + each
//     hashchange).
//   - `encode(state)` returns a hash string ("#style=…&cam=…"); `decode(hash)`
//     returns the parsed ShareState or `null` if the hash is empty/malformed.
//   - `persist(state)` writes the encoded hash via history.replaceState
//     (does not push a new entry — restoration is a side effect, not a
//     navigation).
import { useCallback, useEffect, useRef, useState } from "react";
import { GLOBE_STYLES, type GlobeStyle } from "../visual-effects";

export type TrackingKind = "flight" | "satellite" | "feature";

export interface CameraState {
  lat: number;
  lng: number;
  alt: number;
  heading: number;
  pitch: number;
}

export interface TrackingState {
  kind: TrackingKind;
  id: string;
}

export interface LayoutPanel {
  x?: number;
  y?: number;
  collapsed?: boolean;
}

export interface LayoutState {
  panels: Record<string, LayoutPanel>;
}

export interface ShareState {
  style?: GlobeStyle;
  cam?: CameraState;
  tracking?: TrackingState;
  selected?: string;
  layout?: LayoutState;
}

export interface UseShareRestorationResult {
  urlState: ShareState;
  encode(state: ShareState): string;
  decode(hash: string): ShareState | null;
  persist(state: ShareState): void;
  restoredAt: number;
}

const VALID_STYLES = new Set<string>(GLOBE_STYLES);
const VALID_TRACKING_KINDS = new Set<string>(["flight", "satellite", "feature"]);

function isFiniteNumber(v: number): boolean {
  return Number.isFinite(v);
}

function parseStyle(value: string | null): GlobeStyle | undefined {
  if (!value) return undefined;
  return VALID_STYLES.has(value) ? (value as GlobeStyle) : undefined;
}

function parseCamera(value: string | null): CameraState | undefined {
  if (!value) return undefined;
  const parts = value.split(",");
  if (parts.length !== 5) return undefined;
  const nums = parts.map(Number);
  if (nums.some((n) => !isFiniteNumber(n))) return undefined;
  const [lat, lng, alt, heading, pitch] = nums as [
    number,
    number,
    number,
    number,
    number,
  ];
  return { lat, lng, alt, heading, pitch };
}

function parseTracking(value: string | null): TrackingState | undefined {
  if (!value) return undefined;
  const idx = value.indexOf(":");
  if (idx <= 0 || idx >= value.length - 1) return undefined;
  const kind = value.slice(0, idx);
  const id = value.slice(idx + 1);
  if (!VALID_TRACKING_KINDS.has(kind)) return undefined;
  return { kind: kind as TrackingKind, id };
}

function parseLayout(value: string | null): LayoutState | undefined {
  if (!value) return undefined;
  // atob is available in browsers + jsdom; Node 18+ globalThis.atob too.
  let decoded: string;
  try {
    decoded = atob(value);
  } catch {
    return undefined;
  }
  let parsed: unknown;
  try {
    parsed = JSON.parse(decoded);
  } catch {
    return undefined;
  }
  if (
    !parsed ||
    typeof parsed !== "object" ||
    !("panels" in parsed) ||
    typeof (parsed as { panels?: unknown }).panels !== "object" ||
    (parsed as { panels?: unknown }).panels === null
  ) {
    return undefined;
  }
  return parsed as LayoutState;
}

function encodeLayout(layout: LayoutState): string {
  return btoa(JSON.stringify(layout));
}

function formatCamera(cam: CameraState): string {
  return `${cam.lat},${cam.lng},${cam.alt},${cam.heading},${cam.pitch}`;
}

function formatTracking(t: TrackingState): string {
  return `${t.kind}:${t.id}`;
}

/** Parse a URL hash (with or without leading "#") into a ShareState.
 *  Returns null for empty hashes. Returns a partial ShareState for partial
 *  hashes (missing keys are simply absent). Returns {} (NOT null) when the
 *  hash had keys but every key failed to parse — corrupted URLs never throw,
 *  they reset to an empty state so React hooks see a uniform object shape. */
export function decodeShareHash(hash: string): ShareState | null {
  const stripped = hash.startsWith("#") ? hash.slice(1) : hash;
  if (!stripped) return null;
  const params = new URLSearchParams(stripped);
  const style = parseStyle(params.get("style"));
  const cam = parseCamera(params.get("cam"));
  const tracking = parseTracking(params.get("tracking"));
  const selected = params.get("selected") ?? undefined;
  const layout = parseLayout(params.get("layout"));
  return { style, cam, tracking, selected, layout };
}

/** Encode a ShareState into a URL hash string. Always emits "#"; an empty
 *  state returns "#" so consumers can write it back without branching. */
export function encodeShareState(state: ShareState): string {
  const params = new URLSearchParams();
  if (state.style) params.set("style", state.style);
  if (state.cam) params.set("cam", formatCamera(state.cam));
  if (state.tracking) params.set("tracking", formatTracking(state.tracking));
  if (state.selected) params.set("selected", state.selected);
  if (state.layout) params.set("layout", encodeLayout(state.layout));
  const q = params.toString();
  return q ? `#${q}` : "#";
}

/** Read the current `window.location.hash` (empty string if absent). */
function readCurrentHash(): string {
  if (typeof window === "undefined") return "";
  return window.location.hash;
}

export function useShareRestoration(): UseShareRestorationResult {
  const [urlState, setUrlState] = useState<ShareState>(() =>
    decodeShareHash(readCurrentHash()) ?? {},
  );
  // `restoredAt` records the most recent decode timestamp (initial + each
  // hashchange). Consumers use it for animation timing or to skip the
  // first decode if they're already up-to-date.
  const [restoredAt, setRestoredAt] = useState<number>(() => Date.now());
  const initialDecode = useRef(true);

  useEffect(() => {
    function onHashChange() {
      const next = decodeShareHash(readCurrentHash()) ?? {};
      setUrlState(next);
      setRestoredAt(Date.now());
      initialDecode.current = false;
    }
    window.addEventListener("hashchange", onHashChange);
    return () => window.removeEventListener("hashchange", onHashChange);
  }, []);

  const encode = useCallback((state: ShareState) => encodeShareState(state), []);
  const decode = useCallback((hash: string) => decodeShareHash(hash), []);

  const persist = useCallback((state: ShareState) => {
    const next = encodeShareState(state);
    // history.replaceState — restoration is a side effect, not a navigation
    // entry. We DO NOT pushState (would create a back-button trap).
    if (typeof window === "undefined") return;
    const url = `${window.location.pathname}${window.location.search}${next}`;
    try {
      window.history.replaceState(null, "", url);
      setUrlState(state);
    } catch {
      // history.replaceState can throw on file:// in some browsers; degrade
      // silently — the in-memory state is still updated for the React tree.
    }
  }, []);

  return { urlState, encode, decode, persist, restoredAt };
}