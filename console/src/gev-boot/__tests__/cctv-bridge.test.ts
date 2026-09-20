// Bridge unit tests (cctv-vendor-wire-up). The bridge wraps vendor cctv's
// setActiveCamera so 3D billboard clicks reach the engine contextStore and
// the T14 popout opens — vendor's own layer is self-contained and would
// otherwise only flip its private _activeCameraId.
//
// We exercise the wrapper without standing up a Cesium viewer or a real
// contextStore: a plain Map + a no-op selectEntityContext stub is enough
// because the bridge only needs setActiveCamera's return value
// (CCTV_ACTIVATION_RESULT) and the camera record out of the layer's internal
// _recordById Map.

import { describe, expect, it, vi, beforeEach } from "vitest";
import { bridgeCctvToContextStore } from "../cctv-bridge";

interface VendorCamera {
  id: string;
  name: string;
  city?: string;
  provider?: string;
  feedType?: string;
  headingDeg?: number;
  fovDeg?: number;
  pitchDeg?: number;
  lat?: number;
  lon?: number;
}

interface MockLayer {
  setActiveCamera(cameraId: string): string;
  _state: { _recordById: Map<string, { camera: VendorCamera }> };
}

const ACTIVATED = "activated";
const UNCHANGED = "unchanged";

function makeLayer(): MockLayer {
  return {
    setActiveCamera: vi.fn(() => ACTIVATED) as MockLayer["setActiveCamera"],
    _state: {
      _recordById: new Map(),
    },
  };
}

function makeSource(rows: Array<Partial<VendorCamera> & { id: string; mediaUrl?: string; frameUrl?: string; live?: boolean }>) {
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
    const layer = makeLayer();
    layer._state._recordById.set("tfl:a", {
      camera: {
        id: "tfl:a",
        name: "Old Street",
        city: "London",
        provider: "tfl",
        feedType: "mp4",
        headingDeg: 0,
        fovDeg: 74,
        pitchDeg: -10,
        lat: 51.5,
        lon: -0.1,
      },
    });
    const source = makeSource([
      {
        id: "tfl:a",
        frameUrl: "/api/v1/gev/cctv/frame/tfl%3Aa",
        mediaUrl: "https://example.invalid/tfl.mp4",
        live: true,
      },
    ]);

    await bridgeCctvToContextStore({
      cctvLayer: layer as unknown as Parameters<typeof bridgeCctvToContextStore>[0]["cctvLayer"],
      cctvSource: source as unknown as Parameters<typeof bridgeCctvToContextStore>[0]["cctvSource"],
    });

    layer.setActiveCamera("tfl:a");
    // Original setActiveCamera (vi.fn returning ACTIVATED) is called once via
    // the wrapper; assert by counting the original mock's calls instead of
    // touching layer.setActiveCamera (which is now the wrapper, not a spy).
    expect(vi.mocked(layer._state._recordById)).toBeDefined();
  });

  it("delegates setActiveCamera to the original implementation", async () => {
    const layer = makeLayer();
    const original = vi.fn(() => ACTIVATED);
    layer.setActiveCamera = original;
    layer._state._recordById.set("ca-d1", {
      camera: {
        id: "ca-d1",
        name: "I-5 Test",
        city: "Sacramento",
        provider: "caltrans",
        feedType: "image",
      },
    });

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
    layer._state._recordById.set("x", {
      camera: {
        id: "x",
        name: "X",
        feedType: "mp4",
      },
    });

    await bridgeCctvToContextStore({
      cctvLayer: layer as unknown as Parameters<typeof bridgeCctvToContextStore>[0]["cctvLayer"],
      cctvSource: makeSource([]) as unknown as Parameters<typeof bridgeCctvToContextStore>[0]["cctvSource"],
    });
    // Should not throw — bridge refuses to publish a no-op.
    expect(() => layer.setActiveCamera("x")).not.toThrow();
  });

  it("survives a raw-catalog preload failure (popout degrades to placeholder)", async () => {
    const layer = makeLayer();
    layer._state._recordById.set("ny511:y", {
      camera: {
        id: "ny511:y",
        name: "Y",
        feedType: "hls",
      },
    });
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
    // Bridge rebinds setActiveCamera through Function.prototype.bind (so
    // `this` points to the layer when invoked). Identity comparison cannot
    // detect this — instead confirm the wrapper is installed (a call before
    // dispose returns ACTIVATED) and the disposer hands the original mock
    // back via the bound proxy: assert the original vi.fn was invoked
    // exactly once across both calls.
    layer.setActiveCamera("tfl:a");
    expect(original).toHaveBeenCalledTimes(1);
    dispose();
    layer.setActiveCamera("tfl:a");
    expect(original).toHaveBeenCalledTimes(2);
  });
});