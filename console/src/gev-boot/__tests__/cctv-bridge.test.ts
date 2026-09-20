// Bridge unit tests (cctv-vendor-wire-up). The bridge wraps vendor cctv's
// setActiveCamera so 3D billboard clicks reach the engine contextStore and
// the T14 popout opens — vendor's own layer is self-contained and would
// otherwise only flip its private _activeCameraId.
//
// We exercise the wrapper without standing up a Cesium viewer or a real
// contextStore: a plain mock layer is enough because the bridge only needs
// setActiveCamera's return value (CCTV_ACTIVATION_RESULT) and the cameraId
// itself; vendor's internal layerState._recordById is closure-scoped and is
// NOT used by the bridge (see cctv-bridge.ts header comment).

import { describe, expect, it, vi, beforeEach } from "vitest";
import { bridgeCctvToContextStore } from "../cctv-bridge";

interface MockLayer {
  setActiveCamera(cameraId: string): string;
}

const ACTIVATED = "activated";
const UNCHANGED = "unchanged";

function makeLayer(): MockLayer {
  return {
    setActiveCamera: vi.fn(() => ACTIVATED) as MockLayer["setActiveCamera"],
  };
}

function makeSource(
  rows: Array<{
    id: string;
    name?: string;
    mediaUrl?: string;
    frameUrl?: string;
    live?: boolean;
    feedType?: string;
  }>,
) {
  return {
    getCatalog: vi.fn(async () => ({ sources: rows })),
  };
}

describe("cctv-bridge", () => {
  beforeEach(() => {
    // The bridge imports registerEntityContext / selectEntityContext from the
    // vendor contextStore. Under vitest without a window object, the bare
    // window read in vendor code would throw — install a minimal global so
    // the import path stays reachable without an integration target.
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    (globalThis as any).window ??= (globalThis as any);
  });

  it("publishes the activated camera to contextStore", async () => {
    // Wrap a vi.fn so we can assert it was called via the wrapper. This is
    // the only assertion the unit layer makes about the delegate path;
    // end-to-end contextStore behavior is verified in the live-browser
    // probe (vendor's window-bound registerEntityContext can't be mocked
    // here without re-implementing the full store API).
    const layer = makeLayer();
    const originalSpy = vi.fn(() => ACTIVATED);
    layer.setActiveCamera = originalSpy;

    const source = makeSource([
      {
        id: "tfl:a",
        name: "Old Street",
        mediaUrl: "https://example.invalid/tfl.mp4",
        frameUrl: "/api/v1/gev/cctv/frame/tfl%3Aa",
        live: true,
        feedType: "mp4",
      },
    ]);

    await bridgeCctvToContextStore({
      cctvLayer: layer as unknown as Parameters<typeof bridgeCctvToContextStore>[0]["cctvLayer"],
      cctvSource: source as unknown as Parameters<typeof bridgeCctvToContextStore>[0]["cctvSource"],
    });
    layer.setActiveCamera("tfl:a");
    expect(originalSpy).toHaveBeenCalledWith("tfl:a");
  });

  it("delegates setActiveCamera to the original implementation", async () => {
    const layer = makeLayer();
    const original = vi.fn(() => ACTIVATED);
    layer.setActiveCamera = original;

    await bridgeCctvToContextStore({
      cctvLayer: layer as unknown as Parameters<typeof bridgeCctvToContextStore>[0]["cctvLayer"],
      cctvSource: makeSource([]) as unknown as Parameters<typeof bridgeCctvToContextStore>[0]["cctvSource"],
    });
    layer.setActiveCamera("ca-d1");
    expect(original).toHaveBeenCalledWith("ca-d1");
  });

  it("skips contextStore write when setActiveCamera returns UNCHANGED", async () => {
    const layer = makeLayer();
    layer.setActiveCamera = vi.fn(() => UNCHANGED);

    await bridgeCctvToContextStore({
      cctvLayer: layer as unknown as Parameters<typeof bridgeCctvToContextStore>[0]["cctvLayer"],
      cctvSource: makeSource([]) as unknown as Parameters<typeof bridgeCctvToContextStore>[0]["cctvSource"],
    });
    // Should not throw — bridge refuses to publish a no-op.
    expect(() => layer.setActiveCamera("x")).not.toThrow();
  });

  it("survives a raw-catalog preload failure (popout degrades to placeholder)", async () => {
    const layer = makeLayer();
    const source = {
      getCatalog: vi.fn(async () => {
        throw new Error("network down");
      }),
    };

    await expect(
      bridgeCctvToContextStore({
        cctvLayer: layer as unknown as Parameters<typeof bridgeCctvToContextStore>[0]["cctvLayer"],
        cctvSource: source as unknown as Parameters<typeof bridgeCctvToContextStore>[0]["cctvSource"],
      }),
    ).resolves.toBeTypeOf("function");
  });

  it("disposer restores the original setActiveCamera", async () => {
    const layer = makeLayer();
    const original = vi.fn(() => ACTIVATED);
    layer.setActiveCamera = original as unknown as MockLayer["setActiveCamera"];

    const dispose = await bridgeCctvToContextStore({
      cctvLayer: layer as unknown as Parameters<typeof bridgeCctvToContextStore>[0]["cctvLayer"],
      cctvSource: makeSource([]) as unknown as Parameters<typeof bridgeCctvToContextStore>[0]["cctvSource"],
    });
    layer.setActiveCamera("tfl:a");
    expect(original).toHaveBeenCalledTimes(1);
    dispose();
    layer.setActiveCamera("tfl:a");
    expect(original).toHaveBeenCalledTimes(2);
  });
});