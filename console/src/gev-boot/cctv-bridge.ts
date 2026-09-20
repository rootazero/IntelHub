// CCTV → contextStore bridge (cctv-vendor-wire-up).
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
// Vendor's layerState is closure-scoped (created inside createState, see
// state.js:11-39) and is NOT exposed on the layer instance. The bridge
// therefore cannot read _state._recordById — it must look the camera up
// elsewhere. Two sources are available at the time setActiveCamera fires:
//
//   1. raw source map (preloaded from /api/v1/gev/cctv/sources). This is
//      the canonical row the hub returns, including `mediaUrl` + `frameUrl` +
//      `live` (vendor's catalog.js:159-187 enumerates fields and drops these
//      when it builds the vendor-internal camera object, so we cannot read
//      them from anywhere else).
//   2. vendor camera object (record.camera) inside _state._recordById —
//      available to vendor's own selection.js but not to us.
//
// We use #1. The preloaded map is keyed by the same cameraId string that
// setActiveCamera receives, so a single Map.get() recovers the full row.
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
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  [key: string]: any;
}

interface RawCameraSource {
  id?: string;
  name?: string;
  city?: string;
  provider?: string;
  feedType?: string;
  /** Lat (raw source degrees). */
  lat?: number;
  /** Lon (raw source degrees). */
  lon?: number;
  /** Heading (compass degrees, 0=N). */
  headingDeg?: number;
  /** Field of view, degrees. */
  fovDeg?: number;
  /** Pitch, degrees (negative = downward). */
  pitchDeg?: number;
  /** Same-origin proxy URL for the current still frame. */
  frameUrl?: string;
  /** Upstream mp4/hls/webm URL when the camera has a video stream. */
  mediaUrl?: string;
  live?: boolean;
  status?: string;
  license?: string;
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
 * the original setActiveCamera. Pre-fetches the raw catalog so we can look
 * up the full camera row (vendor's catalog enumeration drops mediaUrl).
 */
export async function bridgeCctvToContextStore({
  cctvLayer,
  cctvSource,
}: BridgeOptions): Promise<() => void> {
  // Pre-fetch the raw source list. Failures here must not break the bridge:
  // a missing entry degrades to "no mediaUrl / no frameUrl" — the popout
  // will render the placeholder state instead of a stream, which is the
  // correct degraded behavior.
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
      publishToContextStore(cameraId, rawById);
    }
    return result;
  };

  return () => {
    cctvLayer.setActiveCamera = original;
  };
}

function publishToContextStore(
  cameraId: string,
  rawById: Map<string, RawCameraSource>,
): void {
  const raw = rawById.get(cameraId);
  if (!raw) {
    // No raw row — we still publish a minimal record so the React side can
    // render the placeholder popout, but it will lack mediaUrl. The next
    // health refresh will give us the row (if the camera is real).
    console.warn(
      "[cctv-bridge] activated camera not in preloaded catalog (raw sources may still be loading):",
      cameraId,
    );
  }

  const properties: Record<string, unknown> = {
    name: raw?.name ?? cameraId,
    city: raw?.city ?? "",
    provider: raw?.provider ?? "",
    feedType: raw?.feedType ?? "image",
    headingDeg: typeof raw?.headingDeg === "number" ? raw.headingDeg : null,
    fovDeg: typeof raw?.fovDeg === "number" ? raw.fovDeg : null,
    pitchDeg: typeof raw?.pitchDeg === "number" ? raw.pitchDeg : null,
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
  // (entity, metadata). metadata.id is the registry key; metadata.layerId
  // is what KIND_BY_LAYER_ID maps to "cctv" (context-bridge.ts:42).
  const stored = registerEntityContext(entity, {
    id: cameraId,
    layerId: CCTV_METADATA.layerId,
    layerName: CCTV_METADATA.layerName,
    source: CCTV_METADATA.source,
    label: properties.name as string,
    latitude: raw?.lat,
    longitude: raw?.lon,
    properties,
  });
  if (stored) selectEntityContext(entity);
}