// CCTV → contextStore bridge (GEV P11 follow-up, cctv-vendor-wire-up).
//
// Vendor's CCTV layer (gev-engine/src/layers/cctv/) is self-contained: clicks
// in 3D flip layerState._activeCameraId and materialize a Cesium plane texture
// (lifecycle.js:273 → selection.js:setActiveCamera → projection.js:
// ensureProjectionRuntime). It does NOT call registerEntityContext, so the
// engine's vendor contextStore never sees CCTV selections — leaving React's
// HudDetailPanel / T14 popout dead for cctv. Bridge wraps the layer's
// setActiveCamera: every successful activation also publishes a record that
// context-bridge.ts::case "cctv" already understands.
//
// Field source split:
//   - vendor camera object (record.camera, set during setActiveCamera) carries
//     name/city/provider/feedType/heading/fov/pitch/range — the values the
//     vendor catalog preserves (catalog.js:159-187 enumerates fields, drops
//     anything else).
//   - raw source from /api/v1/gev/cctv/sources carries mediaUrl + frameUrl +
//     live. We pre-fetch this once at bridge install because vendor does not
//     round-trip the raw source after buildCatalogFromSources().
//
// Why a wrapper (not a layer replacement): lifecycle.js wires its click handler
// during init() (line 273 of lifecycle.js) and the Cesium ScreenSpaceEventHandler
// holds a closure over parts.selection.setActiveCamera. Replacing
// cctvLayer.setActiveCamera with our version is invisible to the click path —
// the wrapper delegates to the original first, then publishes to contextStore.
//
// Reference (vendor): see selection.js:setActiveCamera's return contract
// (CCTV_ACTIVATION_RESULT ∈ {ACTIVATED, UNCHANGED, NOT_FOUND}). Only ACTIVATED
// means a real selection change.

import {
  registerEntityContext,
  selectEntityContext,
} from "gev-engine/src/data/contextStore.js";
import type { CctvSource } from "../gev-adapters/cctv";

interface VendorCctvLayer {
  setActiveCamera(cameraId: string): string;
  /** vendor lifecycle internal state; typed loosely to avoid leaking vendor
   *  private fields into our public types. */
  _state?: {
    _recordById?: Map<string, { camera: VendorCamera }>;
  };
  /** Lifecycle methods object (vendor index.js return shape). */
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  [key: string]: any;
}

interface VendorCamera {
  id: string;
  name?: string;
  city?: string;
  provider?: string;
  feedType?: string;
  headingDeg?: number;
  fovDeg?: number;
  pitchDeg?: number;
  rangeM?: number;
  license?: string;
  credit?: string;
  lat?: number;
  lon?: number;
}

interface RawCameraSource {
  id?: string;
  frameUrl?: string;
  mediaUrl?: string;
  live?: boolean;
  status?: string;
}

const CCTV_ACTIVATION_RESULT = Object.freeze({
  ACTIVATED: "activated",
  UNCHANGED: "unchanged",
  NOT_FOUND: "not-found",
});

const CCTV_METADATA = Object.freeze({
  layerId: "cctv",
  layerName: "CCTV",
  source: "Hub CCTV catalog",
});

export interface BridgeOptions {
  cctvLayer: VendorCctvLayer;
  cctvSource: CctvSource;
}

/**
 * Install the CCTV → contextStore bridge. Returns a disposer that restores
 * the original setActiveCamera. Pre-fetches the raw catalog so we can keep
 * `mediaUrl`/`frameUrl`/`live` even though vendor's catalog enumeration
 * (catalog.js:159-187) drops them.
 */
export async function bridgeCctvToContextStore({
  cctvLayer,
  cctvSource,
}: BridgeOptions): Promise<() => void> {
  // Pre-fetch the raw source list. Failures here must not break the bridge:
  // a missing entry degrades to "no mediaUrl / no frameUrl" — the popout will
  // render the placeholder state instead of a stream, which is correct.
  const rawById = new Map<string, RawCameraSource>();
  try {
    const { sources } = await cctvSource.getCatalog();
    for (const s of sources as RawCameraSource[]) {
      if (s?.id) rawById.set(String(s.id), s);
    }
  } catch (err) {
    console.warn(
      "[cctv-bridge] raw catalog preload failed (popout will lack mediaUrl):",
      err,
    );
  }

  const original = cctvLayer.setActiveCamera.bind(cctvLayer);
  cctvLayer.setActiveCamera = (cameraId: string): string => {
    const result = original(cameraId);
    if (result === CCTV_ACTIVATION_RESULT.ACTIVATED) {
      publishToContextStore(cctvLayer, cameraId, rawById);
    }
    return result;
  };

  return () => {
    cctvLayer.setActiveCamera = original;
  };
}

function publishToContextStore(
  cctvLayer: VendorCctvLayer,
  cameraId: string,
  rawById: Map<string, RawCameraSource>,
): void {
  const vendorRecord = cctvLayer._state?._recordById?.get(cameraId);
  const vendorCamera = vendorRecord?.camera;
  if (!vendorCamera) {
    console.warn(
      "[cctv-bridge] activated camera has no vendor record; skipping contextStore write",
      cameraId,
    );
    return;
  }
  const raw = rawById.get(cameraId);

  const properties: Record<string, unknown> = {
    name: vendorCamera.name ?? cameraId,
    city: vendorCamera.city ?? "",
    provider: vendorCamera.provider ?? "",
    feedType: vendorCamera.feedType ?? "image",
    headingDeg:
      typeof vendorCamera.headingDeg === "number"
        ? vendorCamera.headingDeg
        : null,
    fovDeg:
      typeof vendorCamera.fovDeg === "number" ? vendorCamera.fovDeg : null,
    pitchDeg:
      typeof vendorCamera.pitchDeg === "number"
        ? vendorCamera.pitchDeg
        : null,
    frameUrl: typeof raw?.frameUrl === "string" ? raw.frameUrl : "",
    mediaUrl: typeof raw?.mediaUrl === "string" ? raw.mediaUrl : "",
    live:
      typeof raw?.live === "boolean"
        ? raw.live
        : typeof raw?.status === "string"
          ? raw.status === "ok"
          : null,
  };

  const entity = {
    id: cameraId,
    properties,
  };

  // Vendor's contextStore API (gev-engine/src/data/contextStore.js) takes
  // (entity, metadata). Metadata.id is the registry key; metadata.layerId
  // is what KIND_BY_LAYER_ID maps to "cctv" (context-bridge.ts:42).
  const stored = registerEntityContext(entity, {
    id: cameraId,
    layerId: CCTV_METADATA.layerId,
    layerName: CCTV_METADATA.layerName,
    source: CCTV_METADATA.source,
    label: properties.name as string,
    latitude: vendorCamera.lat,
    longitude: vendorCamera.lon,
    properties,
  });
  if (stored) selectEntityContext(entity);
}