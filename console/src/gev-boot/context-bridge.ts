// T10 selection bridge: engine contextStore → React.
//
// Vendor surface (console/gev-engine/src/data/contextStore.js, read-only):
//   getSelectedEntityContext() → record | null
//     The ONLY read API. The store itself is a bare object hung on
//     window.__gevContextStore with NO subscribe — selection changes are
//     announced on window CustomEvents:
//       gev:entity-selected            click-selections; detail = record
//       gev:entity-selection-cleared   deliberate deselect / eviction
//       gev:awareness-subject-selected TRACKING layers (flights/satellites)
//       gev:awareness-subject-cleared  tracking deselect
//
// VENDOR QUIRK (layers/flights/tracking.js::_publishTrackedSelection): the
// tracking layers dispatch `gev:awareness-subject-selected` BEFORE calling
// selectTrackedSubjectContext(), so a synchronous resync on that event reads
// the PREVIOUS selection. The resync therefore runs on a microtask, after the
// store write has settled.
//
// refreshTrackedSubjectContext() mutates the record in place on every poll
// WITHOUT an event — position/velocity churn never reaches React (the
// templates show no live positions, and a per-poll re-render would be waste).
import { useSyncExternalStore } from "react";
import { getSelectedEntityContext } from "gev-engine/src/data/contextStore.js";

export type GlobeSelectionKind = "flight" | "satellite" | "quake";

export interface GlobeSelection {
  kind: GlobeSelectionKind | null;
  /** Normalized vendor context record, or null when nothing is selected. */
  data: Record<string, unknown> | null;
}

/** layerId → panel kind. Anything unmapped renders the empty state. */
const KIND_BY_LAYER_ID: Record<string, GlobeSelectionKind> = {
  flights: "flight",
  satellites: "satellite",
  earthquakes: "quake",
};

const SELECTION_EVENTS = [
  "gev:entity-selected",
  "gev:entity-selection-cleared",
  "gev:awareness-subject-selected",
  "gev:awareness-subject-cleared",
] as const;

// ---- vendor record normalization -------------------------------------------
//
// The tracking layers publish FLAT DISPLAY STRINGS in record.properties
// (altitude "32,000 ft", speed "450 kt", heading "271°") — see
// layers/{flights,satellites}/tracking.js::_contextSubjectMetadata. The
// panel displays metric values, so the strings are parsed and converted
// here; already-numeric fields (future publishers / quake records) pass
// through with the documented unit assumption (m / m·s⁻¹).

function parseAltitudeM(value: unknown): number | null {
  if (typeof value === "number" && Number.isFinite(value)) return Math.round(value);
  if (typeof value !== "string") return null;
  const ft = /([\d,]+(?:\.\d+)?)\s*ft/.exec(value);
  if (ft) return Math.round(parseFloat(ft[1].replace(/,/g, "")) * 0.3048);
  return null;
}

function parseSpeedKmh(value: unknown): number | null {
  if (typeof value === "number" && Number.isFinite(value))
    return Math.round(value * 3.6); // m·s⁻¹ → km/h
  if (typeof value !== "string") return null;
  const kt = /([\d,]+(?:\.\d+)?)\s*kt/.exec(value);
  if (kt) return Math.round(parseFloat(kt[1].replace(/,/g, "")) * 1.852);
  return null;
}

function parseHeadingDeg(value: unknown): number | null {
  if (typeof value === "number" && Number.isFinite(value)) return Math.round(value);
  if (typeof value !== "string") return null;
  const deg = /([\d.]+)\s*°/.exec(value);
  return deg ? Math.round(parseFloat(deg[1])) : null;
}

interface VendorRecord {
  id?: string | number;
  layerId?: string;
  label?: string;
  properties?: Record<string, unknown> | null;
}

export function normalizeSelection(record: unknown): GlobeSelection {
  const rec = record as VendorRecord | null;
  if (!rec?.layerId) return { kind: null, data: null };
  const kind = KIND_BY_LAYER_ID[rec.layerId];
  if (!kind) return { kind: null, data: null };
  const p = rec.properties ?? {};
  const base: Record<string, unknown> = {
    id: rec.id != null ? String(rec.id) : "",
    label: rec.label ?? "",
  };
  switch (kind) {
    case "flight":
      return {
        kind,
        data: {
          ...base,
          callsign: p.callsign || rec.label || "",
          altitudeM: parseAltitudeM(p.altitude),
          speedKmh: parseSpeedKmh(p.speed),
          headingDeg: parseHeadingDeg(p.heading),
          route: typeof p.route === "string" ? p.route : "",
          status: typeof p.status === "string" ? p.status : "",
        },
      };
    case "satellite":
      return {
        kind,
        data: {
          ...base,
          name: p.name || rec.label || "",
          noradId: p.noradId != null ? String(p.noradId) : String(rec.id ?? ""),
          group: typeof p.class === "string" ? p.class : "",
        },
      };
    case "quake":
      return {
        kind,
        data: {
          ...base,
          mag: typeof p.mag === "number" ? p.mag : null,
          place: typeof p.place === "string" ? p.place : "",
          timeMs: typeof p.time === "number" ? p.time : null,
        },
      };
  }
}

// ---- subscription ------------------------------------------------------------

const listeners = new Set<() => void>();
let attached = false;
let snapshot: GlobeSelection = { kind: null, data: null };
let initialized = false;

function readSelection(): GlobeSelection {
  if (typeof window === "undefined") return { kind: null, data: null };
  let record: unknown = null;
  try {
    record = getSelectedEntityContext();
  } catch {
    record = null; // a torn store must render as empty, never crash the HUD
  }
  return normalizeSelection(record);
}

function resyncSoon(): void {
  // Microtask, not synchronous: the tracking lane's awareness event fires
  // before the store write (vendor quirk above).
  queueMicrotask(() => {
    const next = readSelection();
    if (next.kind === snapshot.kind && next.data === snapshot.data) return;
    snapshot = next;
    for (const listener of [...listeners]) listener();
  });
}

function attach(): void {
  if (attached || typeof window === "undefined") return;
  attached = true;
  for (const event of SELECTION_EVENTS)
    window.addEventListener(event, resyncSoon);
}

export function subscribeGlobeSelection(listener: () => void): () => void {
  listeners.add(listener);
  attach();
  return () => {
    listeners.delete(listener);
  };
}

export function getGlobeSelectionSnapshot(): GlobeSelection {
  // Lazy first read: a selection made before React mounted must show up on
  // the first render without waiting for an event.
  if (!initialized) {
    initialized = true;
    snapshot = readSelection();
  }
  return snapshot;
}

/** Bridge the engine selection store into React state. */
export function useGlobeSelection(): GlobeSelection {
  return useSyncExternalStore(
    subscribeGlobeSelection,
    getGlobeSelectionSnapshot,
    getGlobeSelectionSnapshot,
  );
}

/** Exported for tests: reset module-level subscription state. */
export function _resetGlobeSelectionForTests(): void {
  listeners.clear();
  snapshot = { kind: null, data: null };
  initialized = false;
}
