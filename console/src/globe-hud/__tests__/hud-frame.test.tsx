// T8: GlobeV2 HUD page skeleton.
//   describe 1 — DOM contract: mock the T7 bootstrap (jsdom has no WebGL) and
//     assert HudFrame renders the engine's required mount points.
//   describe 2 — Ruling 9 wiring: mock the engine's scene/catalog factories
//     AND LayerLifecycle, keep createApplication + the real
//     createIntelHubGlobe, and assert the createData phase performs the
//     vendor data.js sequence (register per catalog layer, attachDataManager,
//     attachMapStackController, finalizeRegistrations, default enable set,
//     defer-registered destroyAll).
import { cleanup, render } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { afterEach, describe, expect, test, vi } from "vitest";
// Typed jest-dom matchers (tsc does not see the augmentation from
// test-setup.ts, which imports the untyped runtime entry).
import "@testing-library/jest-dom/vitest";

// ---- Shared hoisted state for the wiring mocks (describe 2) ----------------
const h = vi.hoisted(() => {
  const layerIds = ["flights", "military", "satellites", "earthquakes", "vessels"];
  const layers = layerIds.map((id) => ({
    id,
    name: id,
    attachDataManager: vi.fn(),
    attachMapStackController: vi.fn(),
  }));
  const metadata = layerIds.map((id, i) => ({
    id,
    token: String(i),
    disposition: "enabled-only",
  }));
  // P13 T1: the controls phase calls mountDefaultCamera(scene.viewer), whose
  // contract is `viewer.camera.setView` — the fake must expose that seam the
  // real Cesium.Viewer has, or the mock would hide the wiring (P3 lesson:
  // lenient mocks must mirror the real constructor contract).
  const viewer = { fake: "viewer", camera: { setView: vi.fn() } };
  const mapStackController = { fake: "mapStackController" };
  class MockLayerLifecycle {
    static instances: InstanceType<typeof MockLayerLifecycle>[] = [];
    viewer: unknown;
    options: unknown;
    layers = new Map<string, unknown>();
    register = vi.fn((m: { id: string }) => {
      this.layers.set(m.id, { module: m });
    });
    finalizeRegistrations = vi.fn();
    setEnabled = vi.fn((..._args: unknown[]) => Promise.resolve(true));
    destroyAll = vi.fn(async () => {
      this.layers.clear();
    });
    constructor(viewer: unknown, options: unknown) {
      this.viewer = viewer;
      this.options = options;
      MockLayerLifecycle.instances.push(this);
    }
  }
  return { layers, metadata, viewer, mapStackController, MockLayerLifecycle };
});

vi.mock("gev-engine/src/app/scene.js", () => ({
  createApplicationScene: vi.fn(async () => ({
    viewer: h.viewer,
    mapStackController: h.mapStackController,
    operations: { surface: { groundFloor: {}, terrain: {} } },
  })),
}));

vi.mock("gev-engine/src/app/constructCatalog.js", () => ({
  createApplicationCatalog: vi.fn(() => ({
    layers: h.layers,
    metadata: h.metadata,
  })),
}));

vi.mock("gev-engine/src/data/lifecycle.js", () => ({
  LayerLifecycle: h.MockLayerLifecycle,
}));

// Mock the T7 bootstrap ONLY for the DOM-contract test (describe 1). Vitest
// has no per-test vi.mock, so the module is mocked here and describe 2
// imports the real one through a non-mocked alias: we instead give describe 1
// its own module-level mock and reset between describes via importActual.
vi.mock("../../gev-boot/application", () => ({
  createIntelHubGlobe: vi.fn(() => ({
    start: vi.fn(async () => {}),
    destroy: vi.fn(async () => {}),
    getComponents: () => ({}),
  })),
}));

import GlobeV2 from "../../pages/GlobeV2";

afterEach(cleanup);

describe("GlobeV2 HUD frame DOM contract", () => {
  test("mounts four-edge frame + engine DOM contract", async () => {
    render(
      <MemoryRouter>
        <GlobeV2 />
      </MemoryRouter>,
    );
    expect(document.querySelector("#cesiumContainer")).toBeInTheDocument();
    expect(
      document.querySelector("#loading-screen .loader-status"),
    ).toBeInTheDocument();
    expect(document.querySelector("#data-toggles")).toBeInTheDocument();
    for (const edge of ["top", "left", "right", "bottom"])
      expect(document.querySelector(`[data-hud="${edge}"]`)).toBeInTheDocument();
  });
});

describe("createData LayerLifecycle wiring (Ruling 9)", () => {
  test("register/attach/finalize/enable sequence matches vendor data.js", async () => {
    const { createIntelHubGlobe } = (await vi.importActual(
      "../../gev-boot/application",
    )) as typeof import("../../gev-boot/application");

    const globe = createIntelHubGlobe({
      apiFetch: async () => new Response("{}"),
      googleApiKey: "",
      cesiumToken: "",
    });
    await globe.start();

    // P13 T1: the controls phase applies the global default view to the live
    // viewer (one top-down setView — DEFAULT_VIEW.pitch = -90).
    const setView = h.viewer.camera.setView;
    expect(setView).toHaveBeenCalledTimes(1);
    const view = setView.mock.calls[0][0] as {
      orientation: { pitch: number };
      destination: { x: number };
    };
    expect(view.orientation.pitch).toBeCloseTo(-Math.PI / 2);
    expect(view.destination.x).toBeGreaterThan(0);

    const dm = h.MockLayerLifecycle.instances.at(-1)!;
    expect(dm).toBeDefined();
    expect(dm.viewer).toBe(h.viewer);
    expect(dm.options).toEqual({ allowQaRegistration: false });

    // One register() per catalog layer, in catalog order.
    expect(dm.register).toHaveBeenCalledTimes(h.layers.length);
    for (const layer of h.layers)
      expect(dm.register).toHaveBeenCalledWith(layer);

    // attachDataManager / attachMapStackController run for every layer, with
    // the data manager / scene mapStackController respectively.
    for (const layer of h.layers) {
      expect(layer.attachDataManager).toHaveBeenCalledTimes(1);
      expect(layer.attachDataManager).toHaveBeenCalledWith(dm);
      expect(layer.attachMapStackController).toHaveBeenCalledTimes(1);
      expect(layer.attachMapStackController).toHaveBeenCalledWith(
        h.mapStackController,
      );
    }

    // finalizeRegistrations seals with the catalog metadata array.
    expect(dm.finalizeRegistrations).toHaveBeenCalledTimes(1);
    expect(dm.finalizeRegistrations).toHaveBeenCalledWith(h.metadata);

    // Default enable set: flights/military/satellites/earthquakes ON.
    const enabledCalls = dm.setEnabled.mock.calls.map((c) => [c[0], c[1]]);
    expect(enabledCalls).toEqual([
      ["flights", true],
      ["military", true],
      ["satellites", true],
      ["earthquakes", true],
    ]);
    for (const call of dm.setEnabled.mock.calls)
      expect(call[2]).toEqual({ origin: "programmatic" });

    // The data phase exposes the manager for getComponents() (T9 rail).
    const components = globe.getComponents() as {
      data: { dataManager: unknown; lifecycle: unknown };
    };
    expect(components.data.dataManager).toBe(dm);
    expect(components.data.lifecycle).toBe(dm);

    // defer()-registered cleanup destroys all layers on destroy().
    await globe.destroy();
    expect(dm.destroyAll).toHaveBeenCalledTimes(1);
    expect(dm.layers.size).toBe(0);
  });
});
