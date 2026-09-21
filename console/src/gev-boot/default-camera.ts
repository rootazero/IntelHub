// GEV P13 T1 — global default Cesium camera for the IntelHub globe boot.
//
// Why this module exists: IntelHub's bootstrap builds its own controls phase
// and therefore never runs the vendor's `createApplicationControls`, so the
// vendor's `defer(flyToAustin(viewer))` (gev-engine/src/app/controls.js:45)
// never fires here. Without it the globe keeps Cesium's built-in continental-US
// default rectangle and operators read the live aircraft field as a regional
// (Florida-only) picture — the P13 perception problem this task fixes.
//
// The values below are the spec (plan §Task 1) and are pinned by
// __tests__/default-camera.test.ts.
//
// `import * as Cesium from "cesium"` at module top is safe here: the only
// importer is console/src/gev-boot/application.ts, which already pulls Cesium
// into its graph via `gev-engine/src/app/scene.js` -> `app/viewer.js`. (Sibling
// modules that need only Cesium *types* — e.g. cockpit/vision-mount.ts — use a
// `import type` instead so they stay Cesium-free.)
import * as Cesium from "cesium";

export const DEFAULT_VIEW = {
  lon: -50, // center Western hemisphere
  lat: 20, // mid-latitude (above equator, sees N + S)
  alt: 12_000_000, // 12,000 km — sees entire Americas + Pacific rim
  pitch: -90, // straight down (top-down globe view)
} as const;

/** Numeric override shape (the const above narrows its fields to literals). */
export type DefaultViewOverride = Partial<
  Record<keyof typeof DEFAULT_VIEW, number>
>;

export interface MountDefaultCameraOpts {
  /** Per-field override of DEFAULT_VIEW (unset fields keep the default). */
  view?: DefaultViewOverride;
}

/**
 * Force the globe's initial framing to the global overview. Synchronous by
 * design: the caller (application.ts controls phase) invokes it inline once the
 * scene phase has produced a live viewer.
 *
 * @throws TypeError when `viewer.camera.setView` is absent — the P3 lesson: a
 *   lenient fake must not hide a seam this module depends on.
 */
export function mountDefaultCamera(
  viewer: unknown,
  opts: MountDefaultCameraOpts = {},
): void {
  const camera = (
    viewer as { camera?: { setView?: (options: unknown) => void } } | null
  )?.camera;
  if (typeof camera?.setView !== "function") {
    throw new TypeError("mountDefaultCamera: viewer.camera.setView is missing");
  }
  const view = { ...DEFAULT_VIEW, ...(opts.view ?? {}) };
  // Method call on `camera` (not a detached reference): Cesium's setView
  // reads `this`, so extracting the function would break in the real viewer.
  camera.setView({
    destination: Cesium.Cartesian3.fromDegrees(view.lon, view.lat, view.alt),
    orientation: {
      heading: 0,
      pitch: Cesium.Math.toRadians(view.pitch),
      roll: 0,
    },
  });
}
