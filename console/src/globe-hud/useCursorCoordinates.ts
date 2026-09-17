// T14 live pointer readout: Cesium ScreenSpaceEventHandler MOUSE_MOVE →
// pickEllipsoid → cartographic degrees, consumed by HudBottomBar.
//
// Lifecycle rules (P2 lesson — Leaflet "Set map center and zoom first"):
//   * the handler is registered ONLY when a viewer handle exists, which the
//     page surfaces after globe.start() resolves — never during boot;
//   * route switch / unmount removes the input action and destroys the
//     handler (removeInputAction + destroy in the effect cleanup);
//   * a pick that misses the ellipsoid (camera off-globe) clears the readout
//     back to em-dash, never stale coordinates.
//
// The Cesium module is injectable (default: the real one) so tests can drive
// the handler with a fake viewer + fake math and assert the degrees path.
import { useEffect, useState } from "react";
import * as Cesium from "cesium";

/** Structural viewer slice — the HUD only needs the pick surface. The
 * globe itself is optional: photoreal scenes hide it (scene.js sets
 * globe.show = false) and a torn scene may not have it at all — Cesium's
 * pickEllipsoid defaults to WGS84 when the ellipsoid arg is undefined.
 *
 * `canvas` is MANDATORY: ScreenSpaceEventHandler's constructor registers
 * DOM listeners on it (`element.addEventListener`) — passing the scene
 * object instead crashes the whole HUD tree at mount (T17 315 catch:
 * "element.addEventListener is not a function"). The vitest fake asserts
 * this shape so the regression can't hide behind a lenient mock again. */
export interface CursorViewer {
  scene: {
    canvas: unknown;
    camera: {
      pickEllipsoid(windowPosition: unknown, ellipsoid?: unknown): unknown;
    };
    globe?: { ellipsoid?: unknown } | null;
  };
}

/** Cesium surface the hook consumes (real module or a test fake). */
export interface CursorCesium {
  ScreenSpaceEventHandler: new (
    scene?: unknown,
  ) => {
    setInputAction(
      action: (movement: { endPosition?: unknown }) => void,
      type: unknown,
    ): void;
    removeInputAction(type: unknown): void;
    destroy(): void;
  };
  ScreenSpaceEventType: { MOUSE_MOVE: unknown };
  Cartographic: {
    fromCartesian(cartesian: unknown): { latitude: number; longitude: number };
  };
  Math: { toDegrees(radians: number): number };
}

export interface CursorCoordinates {
  lat: number;
  lon: number;
}

/** Signed degrees, 4 decimals — the HUD readout format. */
export function formatCoord(deg: number): string {
  return deg.toFixed(4);
}

export function useCursorCoordinates(
  viewer: unknown,
  cesium: CursorCesium = Cesium as unknown as CursorCesium,
): CursorCoordinates | null {
  const [coords, setCoords] = useState<CursorCoordinates | null>(null);
  useEffect(() => {
    const v = viewer as CursorViewer | null;
    // Viewer-not-ready guard: no handle, or a scene without the pick
    // surface → stay quiet (em-dash) instead of throwing mid-boot.
    if (
      !v?.scene?.camera ||
      !v.scene.canvas ||
      typeof v.scene.camera.pickEllipsoid !== "function"
    ) {
      setCoords(null);
      return;
    }
    const handler = new cesium.ScreenSpaceEventHandler(v.scene.canvas);
    let moveType: unknown;
    // rAF throttle: Cesium fires MOUSE_MOVE at display rate (60-120 Hz) and
    // every event would otherwise setState → re-render the whole bottom bar.
    // Keep only the LATEST pick per frame; the pending frame is cancelled on
    // teardown so no setState escapes an unmounted bar.
    let pending: CursorCoordinates | null = null;
    let frame: number | null = null;
    const flush = () => {
      frame = null;
      setCoords(pending);
    };
    handler.setInputAction((movement) => {
      const cartesian = v.scene.camera.pickEllipsoid(
        movement.endPosition,
        v.scene.globe?.ellipsoid,
      );
      pending = cartesian
        ? (() => {
            const cartographic =
              cesium.Cartographic.fromCartesian(cartesian);
            return {
              lat: cesium.Math.toDegrees(cartographic.latitude),
              lon: cesium.Math.toDegrees(cartographic.longitude),
            };
          })()
        : null; // off-globe pick → clear the readout
      if (frame == null) frame = requestAnimationFrame(flush);
    }, (moveType = cesium.ScreenSpaceEventType.MOUSE_MOVE));
    return () => {
      // removeInputAction BEFORE destroy: destroy alone detaches the DOM
      // listeners, but the explicit removal keeps the contract honest and
      // mirrors the engine's own teardown idiom. The pending rAF dies too —
      // otherwise a last mousemove could setState after unmount.
      if (frame != null) cancelAnimationFrame(frame);
      handler.removeInputAction(moveType);
      handler.destroy();
    };
  }, [viewer, cesium]);
  return coords;
}
