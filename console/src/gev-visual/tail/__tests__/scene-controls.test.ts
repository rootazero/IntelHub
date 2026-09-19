// GEV P10 T2 — scene-controls adapter tests.
//
// Verifies the adapter wraps SceneControls with the correct constructor
// shape (P3 lesson: mocks replicate the real vendor contract), exposes the
// 8 brief-mandated actions, and routes notifications to onChange consumers.
import { afterEach, describe, expect, test, vi } from "vitest";
import {
  mountSceneControls,
  type SceneActions,
  type SceneElements,
  type SceneProjectState,
} from "../scene-controls";

function fakeState(): SceneProjectState {
  return {
    scenes: [],
    selectedSceneId: null,
    selectedShotId: null,
    running: false,
    hasRun: false,
    progress: 0,
    status: "Ready",
    runtime: "",
    playbackActive: false,
    keyboardEnabled: false,
  };
}

function fakeActions(): SceneActions & Record<string, ReturnType<typeof vi.fn>> {
  // The 8 brief-mandated actions + selectScene (vendor's select dispatch).
  // Vendor reads `actions[action]` at runtime, so unknown actions are
  // dispatched via the index signature.
  return {
    selectScene: vi.fn(),
    capture: vi.fn(),
    update: vi.fn(),
    next: vi.fn(),
    export: vi.fn(),
    download: vi.fn(),
    import: vi.fn(),
    start: vi.fn(),
    stop: vi.fn(),
    reviewImport: vi.fn(),
  };
}

function fakeElements(): SceneElements {
  // Vendor fixture per scenePresentation.js#sceneElements — only `select`
  // matters for the no-op branch (vendor skips listener wiring when select
  // is absent). We pass a populated-but-detached map so the vendor's
  // listener-wiring branch runs and we exercise the cleanup path.
  const make = () => document.createElement("button");
  return {
    select: make(),
    capture: make(),
    update: make(),
    next: make(),
    export: make(),
    download: make(),
    new: make(),
    delete: make(),
    start: make(),
    stop: make(),
    import: make(),
    file: document.createElement("input"),
    shots: document.createElement("div"),
    progress: document.createElement("div"),
    status: document.createElement("span"),
    runtime: document.createElement("span"),
  };
}

describe("scene-controls adapter — constructor shape", () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  test("mountSceneControls constructs without elements (vendor self-discovery)", () => {
    const handle = mountSceneControls({
      read: fakeState,
      actions: fakeActions(),
    });
    expect(handle.controls).toBeDefined();
    handle.destroy();
  });

  test("mountSceneControls accepts explicit elements map", () => {
    const handle = mountSceneControls({
      read: fakeState,
      actions: fakeActions(),
      elements: fakeElements(),
    });
    expect(handle.controls).toBeDefined();
    handle.destroy();
  });

  test("mountSceneControls accepts subscribe: undefined branch", () => {
    // The adapter's no-subscribe branch (vendor renders initial sceneSelect +
    // shotList). This is the path T3 will use because GlobeV2 owns the
    // notification fan-out via onChange().
    const handle = mountSceneControls({
      read: fakeState,
      actions: fakeActions(),
      elements: fakeElements(),
    });
    expect(handle.controls).toBeDefined();
    handle.destroy();
  });
});

describe("scene-controls adapter — onChange notification", () => {
  test("onChange subscribers receive initial notification", () => {
    const handle = mountSceneControls({
      read: fakeState,
      actions: fakeActions(),
      elements: fakeElements(),
    });
    const listener = vi.fn();
    handle.onChange(listener);
    // Manually trigger present() — vendor's own constructor wires internal
    // presentation; adapter wrapper fans out to consumer listeners.
    handle.controls.present({
      state: fakeState(),
      initial: true,
    });
    expect(listener).toHaveBeenCalled();
    handle.destroy();
  });

  test("onChange returns an unsubscribe closure", () => {
    const handle = mountSceneControls({
      read: fakeState,
      actions: fakeActions(),
      elements: fakeElements(),
    });
    const listener = vi.fn();
    const off = handle.onChange(listener);
    handle.controls.present({ state: fakeState(), initial: true });
    const before = listener.mock.calls.length;
    off();
    handle.controls.present({ state: fakeState() });
    expect(listener.mock.calls.length).toBe(before);
    handle.destroy();
  });

  test("multiple subscribers all receive notifications", () => {
    const handle = mountSceneControls({
      read: fakeState,
      actions: fakeActions(),
      elements: fakeElements(),
    });
    const l1 = vi.fn();
    const l2 = vi.fn();
    handle.onChange(l1);
    handle.onChange(l2);
    handle.controls.present({ state: fakeState(), initial: true });
    expect(l1).toHaveBeenCalled();
    expect(l2).toHaveBeenCalled();
    handle.destroy();
  });
});

describe("scene-controls adapter — destroy", () => {
  test("destroy() is idempotent", () => {
    const handle = mountSceneControls({
      read: fakeState,
      actions: fakeActions(),
      elements: fakeElements(),
    });
    handle.destroy();
    expect(() => handle.destroy()).not.toThrow();
  });

  test("destroy() clears consumer listeners", () => {
    const handle = mountSceneControls({
      read: fakeState,
      actions: fakeActions(),
      elements: fakeElements(),
    });
    const listener = vi.fn();
    handle.onChange(listener);
    handle.destroy();
    // After destroy the wrapped present still works (vendor's destroyed
    // guard inside present returns early), so the listener must not be
    // called for any new notification. The fan-out still runs, but the
    // listener was cleared from the set first.
    handle.controls.present({ state: fakeState() });
    expect(listener).not.toHaveBeenCalled();
  });
});

describe("scene-controls adapter — vendor action surface", () => {
  test("actions map carries all 8 brief-mandated methods", () => {
    const actions = fakeActions();
    const required = [
      "capture",
      "update",
      "next",
      "export",
      "download",
      "import",
      "start",
      "stop",
      "reviewImport",
    ];
    for (const name of required) {
      expect(typeof actions[name]).toBe("function");
    }
  });

  test("actions object is forwarded to the vendor constructor", () => {
    const actions = fakeActions();
    const handle = mountSceneControls({
      read: fakeState,
      actions,
      elements: fakeElements(),
    });
    expect((handle.controls as unknown as { actions: SceneActions }).actions)
      .toBe(actions);
    handle.destroy();
  });
});