// GEV P12 T7 — cockpit viewport lock tests (spec §6.E5).
//
// The handler is the REAL Cesium ScreenSpaceEventHandler driven against a
// jsdom canvas: the P3 lesson is that a lenient fake hides a broken
// constructor contract (T7 R5 depends on the handler binding to the canvas,
// not the scene). Asserting the real constructor means a wrong canvas-arg
// shape fails here instead of in the browser.
import { afterEach, describe, expect, it } from "vitest";
import {
  mountCockpitViewportLock,
  type CockpitViewportLockViewer,
} from "../viewport-lock";

function makeViewer(): CockpitViewportLockViewer & {
  canvas: HTMLCanvasElement;
} {
  const canvas = document.createElement("canvas");
  return {
    scene: { screenSpaceCameraController: { enableInputs: true } },
    cesiumWidget: { canvas },
    canvas,
  };
}

describe("mountCockpitViewportLock", () => {
  const mounted: { destroy(): void }[] = [];
  afterEach(() => {
    for (const lock of mounted.splice(0)) lock.destroy();
  });

  it("throws TypeError when screenSpaceCameraController is missing", () => {
    expect(() =>
      mountCockpitViewportLock({
        scene: {} as never,
        cesiumWidget: { canvas: document.createElement("canvas") },
      }),
    ).toThrow(TypeError);
  });

  it("throws TypeError when cesiumWidget.canvas is missing", () => {
    expect(() =>
      mountCockpitViewportLock({
        scene: { screenSpaceCameraController: { enableInputs: true } },
        cesiumWidget: {} as never,
      }),
    ).toThrow(TypeError);
  });

  it("lock() disables camera inputs and hides the cursor", () => {
    const viewer = makeViewer();
    const lock = mountCockpitViewportLock(viewer);
    mounted.push(lock);

    expect(lock.isLocked()).toBe(false);
    lock.lock();

    expect(lock.isLocked()).toBe(true);
    expect(viewer.scene.screenSpaceCameraController.enableInputs).toBe(false);
    expect(viewer.canvas.style.cursor).toBe("none");
  });

  it("unlock() restores camera inputs and the pre-lock cursor", () => {
    const viewer = makeViewer();
    viewer.canvas.style.cursor = "crosshair";
    const lock = mountCockpitViewportLock(viewer);
    mounted.push(lock);

    lock.lock();
    lock.unlock();

    expect(lock.isLocked()).toBe(false);
    expect(viewer.scene.screenSpaceCameraController.enableInputs).toBe(true);
    expect(viewer.canvas.style.cursor).toBe("crosshair");
  });

  it("double lock() is a no-op and destroy() auto-unlocks", () => {
    const viewer = makeViewer();
    const lock = mountCockpitViewportLock(viewer);

    lock.lock();
    lock.lock();
    expect(lock.isLocked()).toBe(true);
    expect(viewer.scene.screenSpaceCameraController.enableInputs).toBe(false);

    lock.destroy();
    expect(lock.isLocked()).toBe(false);
    expect(viewer.scene.screenSpaceCameraController.enableInputs).toBe(true);
    expect(viewer.canvas.style.cursor).toBe("");
    // Teardown is idempotent and a post-destroy lock() cannot re-arm.
    expect(() => lock.destroy()).not.toThrow();
    lock.lock();
    expect(lock.isLocked()).toBe(false);
  });
});
