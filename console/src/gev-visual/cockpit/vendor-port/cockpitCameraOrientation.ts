// GEV P15 T2 — vendor port of cameraOrientationControls (spec §4.3).
//
// Ports three pure-math functions from gods-eye-view/src/ui/cameraOrientationControls.js:
//   - readCameraTargetFrame (lines 60-93)
//   - setCameraTargetFrame (lines 95-128)
//   - createCameraOrientationAnimator (lines 188-229)
//
// The math is byte-stable upstream — only the file extension / module format
// changes; logic stays identical. AGENTS.md GEV pin workflow will detect drift
// on next sync. mouse-look.ts consumes these to pan / zoom / snap-back in
// cockpit mode.

import * as Cesium from "cesium";

function isPickedWorldPosition(value: unknown): value is Cesium.Cartesian3 {
  return (
    value instanceof Cesium.Cartesian3 &&
    Number.isFinite(value.x) &&
    Number.isFinite(value.y) &&
    Number.isFinite(value.z)
  );
}

function trackedTarget(viewer: PickedViewer): Cesium.Cartesian3 | null {
  const entity = viewer.trackedEntity;
  const displayPosition =
    (entity as { gevDisplayPosition?: () => Cesium.Cartesian3 } | undefined)
      ?.gevDisplayPosition;
  const fromDisplay =
    typeof displayPosition === "function" ? displayPosition() : null;
  if (isPickedWorldPosition(fromDisplay)) return fromDisplay;
  const position = entity?.position?.getValue?.(viewer.clock?.currentTime);
  return isPickedWorldPosition(position) ? position : null;
}

export interface PickedViewer {
  clock?: { currentTime: Cesium.JulianDate };
  trackedEntity?: Cesium.Entity | undefined;
  camera: Cesium.Camera;
  scene: Cesium.Scene;
  isDestroyed?: () => boolean;
}

export interface CameraTargetFrame {
  target: Cesium.Cartesian3;
  range: number;
  heading: number;
  pitch: number;
}

export function readCameraTargetFrame(
  viewer: PickedViewer,
): CameraTargetFrame | null {
  const camera = viewer?.camera;
  const target = viewer?.trackedEntity
    ? trackedTarget(viewer)
    : pickViewTarget(viewer);
  if (!camera || !target || !isPickedWorldPosition(camera.positionWC))
    return null;
  const transform = Cesium.Transforms.eastNorthUpToFixedFrame(target);
  const inverse = Cesium.Matrix4.inverseTransformation(
    transform,
    new Cesium.Matrix4(),
  );
  const localOffset = Cesium.Matrix4.multiplyByPoint(
    inverse,
    camera.positionWC,
    new Cesium.Cartesian3(),
  );
  const range = Cesium.Cartesian3.magnitude(localOffset);
  if (!Number.isFinite(range) || range < 1) return null;
  const pitch = -Math.asin(
    Cesium.Math.clamp(localOffset.z / range, -1, 1),
  );
  const targetHeading = Math.atan2(-localOffset.x, -localOffset.y);
  const heading = normalizedHeading(
    pitch < Cesium.Math.toRadians(-88.5) ? camera.heading : targetHeading,
  );
  return { target, range, heading, pitch };
}

function pickViewTarget(viewer: PickedViewer): Cesium.Cartesian3 | null {
  const scene = viewer?.scene;
  const camera = viewer?.camera;
  const canvas = scene?.canvas;
  if (!scene || !camera || !canvas) return null;
  const width = canvas.clientWidth || canvas.width || 0;
  const height = canvas.clientHeight || canvas.height || 0;
  if (!width || !height) return null;
  const center = new Cesium.Cartesian2(width / 2, height / 2);
  let target: Cesium.Cartesian3 | null = null;
  if (
    scene.pickPositionSupported &&
    typeof scene.pickPosition === "function"
  ) {
    try {
      target = scene.pickPosition(center) ?? null;
    } catch {
      target = null;
    }
  }
  if (
    !isPickedWorldPosition(target) &&
    typeof camera.getPickRay === "function"
  ) {
    try {
      const ray = camera.getPickRay(center);
      target = ray ? scene.globe?.pick(ray, scene) || null : null;
    } catch {
      target = null;
    }
  }
  if (
    !isPickedWorldPosition(target) &&
    typeof camera.pickEllipsoid === "function"
  ) {
    try {
      target = camera.pickEllipsoid(center, Cesium.Ellipsoid.WGS84) ?? null;
    } catch {
      target = null;
    }
  }
  return isPickedWorldPosition(target) ? target : null;
}

export function setCameraTargetFrame(
  viewer: PickedViewer,
  frame: CameraTargetFrame,
): boolean {
  const camera = viewer?.camera;
  if (!camera || !frame?.target || !Number.isFinite(frame.range)) return false;
  try {
    const trackedTransform = viewer.trackedEntity
      ? Cesium.Matrix4.clone(camera.transform)
      : null;
    camera.lookAt(
      frame.target,
      new Cesium.HeadingPitchRange(
        normalizedHeading(frame.heading),
        frame.pitch,
        frame.range,
      ),
    );
    if (trackedTransform) {
      camera.lookAtTransform(trackedTransform);
      viewer.scene?.requestRender?.();
      return true;
    }
    const destination = Cesium.Cartesian3.clone(camera.positionWC);
    const direction = Cesium.Cartesian3.clone(camera.directionWC);
    const up = Cesium.Cartesian3.clone(camera.upWC);
    camera.lookAtTransform(Cesium.Matrix4.IDENTITY);
    if (destination && direction && up) {
      camera.setView({ destination, orientation: { direction, up } });
    }
    viewer.scene?.requestRender?.();
    return true;
  } catch {
    return false;
  }
}

export interface AnimatorHandle {
  animate(
    frame: CameraTargetFrame,
    destination: { heading: number; pitch: number },
  ): boolean;
  cancel(): void;
  readonly destination: { heading: number; pitch: number } | null;
}

export function createCameraOrientationAnimator(
  viewer: PickedViewer,
  {
    now = () => performance.now(),
    duration = 650,
  }: { now?: () => number; duration?: number } = {},
): AnimatorHandle {
  let remove: (() => void) | null = null;
  let pending: { heading: number; pitch: number } | null = null;
  const cancel = () => {
    remove?.();
    remove = null;
    pending = null;
  };
  function animate(
    frame: CameraTargetFrame,
    destination: { heading: number; pitch: number },
  ): boolean {
    cancel();
    if (!viewer.scene?.preUpdate?.addEventListener || duration <= 0)
      return setCameraTargetFrame(viewer, { ...frame, ...destination });
    const entity = viewer.trackedEntity;
    pending = destination;
    const start = now();
    const headingDelta = Cesium.Math.negativePiToPi(
      destination.heading - frame.heading,
    );
    remove = viewer.scene.preUpdate.addEventListener(() => {
      if (viewer.isDestroyed?.() || viewer.trackedEntity !== entity) {
        cancel();
        return;
      }
      const progress = Cesium.Math.clamp((now() - start) / duration, 0, 1);
      const eased = Cesium.EasingFunction.CUBIC_IN_OUT(progress);
      const target = entity ? trackedTarget(viewer) : frame.target;
      if (
        !isPickedWorldPosition(target) ||
        !setCameraTargetFrame(viewer, {
          ...frame,
          target,
          heading: frame.heading + headingDelta * eased,
          pitch: Cesium.Math.lerp(frame.pitch, destination.pitch, eased),
        })
      ) {
        cancel();
        return;
      }
      if (progress === 1) cancel();
    });
    viewer.scene.requestRender?.();
    return true;
  }
  return {
    animate,
    cancel,
    get destination() {
      return pending;
    },
  };
}

function normalizedHeading(heading: number): number {
  const wrapped = Cesium.Math.zeroToTwoPi(
    Number.isFinite(heading) ? heading : 0,
  );
  return Math.abs(wrapped - Cesium.Math.TWO_PI) < Cesium.Math.EPSILON10
    ? 0
    : wrapped;
}
