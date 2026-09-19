// GEV P10 T2 — shortcuts adapter tests.
//
// Verifies the 8 brief-mandated actions, the R5 gate (cockpitStore.active
// suppresses setStyle), the cheatsheet `?` adapter extension, and the
// form-control exclusion (Escape still works while typing).
import { afterEach, describe, expect, test, vi } from "vitest";
import {
  bindShortcuts,
  type ShortcutsActions,
} from "../shortcuts";
import { createCockpitStore } from "../../cockpit/cockpit-store";

function fakeActions(): ShortcutsActions & Record<string, ReturnType<typeof vi.fn>> {
  return {
    setStyle: vi.fn(),
    dismissSearch: vi.fn(),
    toggleHud: vi.fn(),
    toggleOrbit: vi.fn(),
    toggleCleanView: vi.fn(),
    toggleLayers: vi.fn(),
    cycleDetection: vi.fn(),
    toggleCctv: vi.fn(),
    toggleCheatsheet: vi.fn(),
  };
}

function dispatchKey(key: string, target: EventTarget = document.body) {
  const event = new KeyboardEvent("keydown", {
    key,
    bubbles: true,
    cancelable: true,
  });
  Object.defineProperty(event, "target", { value: target });
  document.dispatchEvent(event);
  return event;
}

describe("shortcuts adapter — 8 brief actions", () => {
  let actions: ReturnType<typeof fakeActions>;
  let cleanup: () => void;

  afterEach(() => {
    cleanup?.();
    vi.restoreAllMocks();
  });

  function setup(cockpitStore?: Parameters<typeof bindShortcuts>[0]["cockpitStore"]) {
    actions = fakeActions();
    const handle = bindShortcuts({
      documentRef: document,
      actions,
      ...(cockpitStore ? { cockpitStore } : {}),
    });
    cleanup = () => handle.destroy();
  }

  test("digits 1-7 dispatch setStyle with the correct vendor key name", () => {
    setup();
    const cases: Array<[string, string]> = [
      ["1", "normal"],
      ["2", "retro"],
      ["3", "surveillance"],
      ["4", "thermal"],
      ["5", "anime"],
      ["6", "noir"],
      ["7", "snow"],
    ];
    for (const [key, expected] of cases) {
      dispatchKey(key);
      expect(actions.setStyle).toHaveBeenCalledWith(expected);
    }
  });

  test("Escape triggers dismissSearch", () => {
    setup();
    dispatchKey("Escape");
    expect(actions.dismissSearch).toHaveBeenCalledTimes(1);
  });

  test("h triggers toggleHud", () => {
    setup();
    dispatchKey("h");
    expect(actions.toggleHud).toHaveBeenCalledTimes(1);
  });

  test("o triggers toggleOrbit", () => {
    setup();
    dispatchKey("o");
    expect(actions.toggleOrbit).toHaveBeenCalledTimes(1);
  });

  test("v triggers toggleCleanView", () => {
    setup();
    dispatchKey("v");
    expect(actions.toggleCleanView).toHaveBeenCalledTimes(1);
  });

  test("f triggers toggleLayers", () => {
    setup();
    dispatchKey("f");
    expect(actions.toggleLayers).toHaveBeenCalledTimes(1);
  });

  test("d triggers cycleDetection", () => {
    setup();
    dispatchKey("d");
    expect(actions.cycleDetection).toHaveBeenCalledTimes(1);
  });

  test("c triggers toggleCctv", () => {
    setup();
    dispatchKey("c");
    expect(actions.toggleCctv).toHaveBeenCalledTimes(1);
  });
});

describe("shortcuts adapter — R5 gate (cockpit active suppresses setStyle)", () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  test("setStyle is suppressed while cockpit is active", () => {
    const actions = fakeActions();
    const store = createCockpitStore();
    store.enter("flight-1");
    const cleanup = bindShortcuts({
      documentRef: document,
      actions,
      cockpitStore: store,
    });
    dispatchKey("1"); // normal
    dispatchKey("5"); // anime
    expect(actions.setStyle).not.toHaveBeenCalled();
    cleanup.destroy();
  });

  test("setStyle resumes after cockpit exits", () => {
    const actions = fakeActions();
    const store = createCockpitStore();
    const cleanup = bindShortcuts({
      documentRef: document,
      actions,
      cockpitStore: store,
    });
    dispatchKey("1"); // normal — active
    expect(actions.setStyle).toHaveBeenCalledWith("normal");
    store.enter("flight-1");
    (actions.setStyle as ReturnType<typeof vi.fn>).mockClear();
    dispatchKey("5"); // anime — gated
    expect(actions.setStyle).not.toHaveBeenCalled();
    store.exit();
    dispatchKey("5"); // anime — re-enabled
    expect(actions.setStyle).toHaveBeenCalledWith("anime");
    cleanup.destroy();
  });

  test("non-style actions pass through regardless of cockpit state", () => {
    const actions = fakeActions();
    const store = createCockpitStore();
    store.enter("flight-1");
    const cleanup = bindShortcuts({
      documentRef: document,
      actions,
      cockpitStore: store,
    });
    dispatchKey("f"); // toggleLayers
    dispatchKey("c"); // toggleCctv
    expect(actions.toggleLayers).toHaveBeenCalledTimes(1);
    expect(actions.toggleCctv).toHaveBeenCalledTimes(1);
    cleanup.destroy();
  });
});

describe("shortcuts adapter — cheatsheet `?` extension", () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  test("? triggers toggleCheatsheet", () => {
    const actions = fakeActions();
    const cleanup = bindShortcuts({ documentRef: document, actions });
    dispatchKey("?");
    expect(actions.toggleCheatsheet).toHaveBeenCalledTimes(1);
    cleanup.destroy();
  });

  test("? inside an input is ignored", () => {
    const actions = fakeActions();
    const cleanup = bindShortcuts({ documentRef: document, actions });
    const input = document.createElement("input");
    document.body.appendChild(input);
    dispatchKey("?", input);
    expect(actions.toggleCheatsheet).not.toHaveBeenCalled();
    cleanup.destroy();
  });

  test("missing toggleCheatsheet does not throw", () => {
    const actions = fakeActions();
    delete (actions as { toggleCheatsheet?: unknown }).toggleCheatsheet;
    const cleanup = bindShortcuts({ documentRef: document, actions });
    expect(() => dispatchKey("?")).not.toThrow();
    cleanup.destroy();
  });
});

describe("shortcuts adapter — lifecycle", () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  test("destroy() unbinds the listener", () => {
    const actions = fakeActions();
    const cleanup = bindShortcuts({ documentRef: document, actions });
    cleanup.destroy();
    dispatchKey("h");
    expect(actions.toggleHud).not.toHaveBeenCalled();
  });

  test("destroy() is idempotent", () => {
    const cleanup = bindShortcuts({
      documentRef: document,
      actions: fakeActions(),
    });
    cleanup.destroy();
    expect(() => cleanup.destroy()).not.toThrow();
  });

  test("no documentRef returns an inert handle", () => {
    const cleanup = bindShortcuts({
      // No documentRef → inert handle, no crash.
      actions: fakeActions(),
    });
    expect(() => cleanup.destroy()).not.toThrow();
  });
});

describe("shortcuts adapter — form-control exclusion", () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  test("typing in input suppresses style shortcuts", () => {
    const actions = fakeActions();
    const cleanup = bindShortcuts({ documentRef: document, actions });
    const input = document.createElement("input");
    document.body.appendChild(input);
    dispatchKey("1", input);
    expect(actions.setStyle).not.toHaveBeenCalled();
    cleanup.destroy();
  });

  test("Escape inside input still triggers dismissSearch", () => {
    // The vendor explicitly allows Escape to bypass form-control exclusion.
    const actions = fakeActions();
    const cleanup = bindShortcuts({ documentRef: document, actions });
    const input = document.createElement("input");
    document.body.appendChild(input);
    dispatchKey("Escape", input);
    expect(actions.dismissSearch).toHaveBeenCalledTimes(1);
    cleanup.destroy();
  });
});