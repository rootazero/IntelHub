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

export type GlobeSelectionKind =
  | "flight"
  | "satellite"
  | "quake"
  | "vessel"
  | "cctv"
  | "installation";

export interface GlobeSelection {
  kind: GlobeSelectionKind | null;
  /** Normalized vendor context record, or null when nothing is selected. */
  data: Record<string, unknown> | null;
}

/** layerId → panel kind. Anything unmapped renders the empty state.
 *
 * P3 layer ids (grep-verified against the vendor tree, 2026-09-17):
 *   vessels       → 'ais-live-vessels'      (gev-engine/src/layers/vessels/selection.js:195 —
 *                    registerSelectedContext writes layerId: 'ais-live-vessels'; properties at :196-203)
 *   installations → 'military-installations' (gev-engine/src/layers/installations/policy.js:1 LAYER_ID;
 *                    context write at layers/installations/rendering.js:121-137)
 *   cctv          → NO vendor contextStore write exists: `grep -rn 'registerEntityContext|selectEntityContext'
 *                    layers/cctv/` returns 0 hits (2026-09-17). The registry id is 'cctv'
 *                    (gev-engine/src/data/layerState.js:327); mapping it here is forward-compat for
 *                    the day the engine publishes cctv selections, and inert until then. */
const KIND_BY_LAYER_ID: Record<string, GlobeSelectionKind> = {
  flights: "flight",
  satellites: "satellite",
  earthquakes: "quake",
  "ais-live-vessels": "vessel",
  "military-installations": "installation",
  cctv: "cctv",
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
    case "vessel": {
      // Vendor context record (layers/vessels/selection.js:196-203) publishes
      // mmsi / type / speedKt / course / destination under properties, with
      // the vessel name as record.label. The contracts' full VesselRecord
      // (sources/live/vessels.js:4-26) additionally carries imo / heading /
      // observedAtMs / speedMps — accepted here when present so a future
      // richer publisher needs no bridge change.
      const idStr = String(rec.id ?? "");
      const speedKn =
        typeof p.speedKt === "number"
          ? p.speedKt // already knots (selection.js:200 speedKt: record.speed)
          : typeof p.speedMps === "number"
            ? p.speedMps / 0.514444 // m/s → kn (ingestion.js:157 convention)
            : null;
      return {
        kind,
        data: {
          ...base,
          name: rec.label ?? "",
          mmsi:
            typeof p.mmsi === "string"
              ? p.mmsi
              : idStr.replace(/^ais-/, ""), // selection.js:197 id `ais-<mmsi>`
          imo: typeof p.imo === "string" ? p.imo : "",
          type: typeof p.type === "string" ? p.type : "",
          destination: typeof p.destination === "string" ? p.destination : "",
          speedKn,
          courseDeg:
            typeof p.course === "number"
              ? p.course
              : typeof p.courseDeg === "number"
                ? p.courseDeg
                : null,
          headingDeg:
            typeof p.heading === "number"
              ? p.heading
              : typeof p.headingDeg === "number"
                ? p.headingDeg
                : null,
          observedAtMs:
            typeof p.observedAtMs === "number" ? p.observedAtMs : null,
        },
      };
    }
    case "installation": {
      // Vendor context record (layers/installations/rendering.js:121-137):
      // id = 'osm:<type>:<id>' (militaryInstallationData.js:137-145), name as
      // record.label, properties class / primaryType / placeTypes /
      // validation / retrievedAt. The record's sources array is NOT part of
      // the context write — the template degrades to '—' until the engine
      // publishes it.
      const idStr = String(rec.id ?? "");
      const osmParts = idStr.startsWith("osm:") ? idStr.split(":") : [];
      const retrievedAtMs =
        typeof p.retrievedAt === "number"
          ? p.retrievedAt
          : typeof p.retrievedAt === "string"
            ? Date.parse(p.retrievedAt)
            : null;
      return {
        kind,
        data: {
          ...base,
          name: rec.label ?? "",
          class: typeof p.class === "string" ? p.class : "",
          osmType: osmParts[1] ?? "",
          osmId: osmParts[2] ?? "",
          sources: Array.isArray(p.sources) ? p.sources : [],
          validation: typeof p.validation === "string" ? p.validation : "",
          retrievedAtMs,
        },
      };
    }
    case "cctv": {
      // No vendor publisher today (see KIND_BY_LAYER_ID). Field names follow
      // the contracts' CameraSource table (catalog.js:121-214). frameUrl is
      // the link target for the 「实时画面」 entry — LINK SEMANTICS ONLY; the
      // HUD never embeds <img>/<video> (live pixels are the engine pipeline's
      // job: layers/cctv/projection.js monitor plane via canvas texture).
      return {
        kind,
        data: {
          ...base,
          name: rec.label ?? "",
          city: typeof p.city === "string" ? p.city : "",
          provider: typeof p.provider === "string" ? p.provider : "",
          feedType: typeof p.feedType === "string" ? p.feedType : "",
          headingDeg: typeof p.headingDeg === "number" ? p.headingDeg : null,
          fovDeg: typeof p.fovDeg === "number" ? p.fovDeg : null,
          pitchDeg: typeof p.pitchDeg === "number" ? p.pitchDeg : null,
          frameUrl: typeof p.frameUrl === "string" ? p.frameUrl : "",
          live:
            typeof p.live === "boolean"
              ? p.live
              : typeof p.status === "string"
                ? p.status === "ok"
                : null,
        },
      };
    }
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
