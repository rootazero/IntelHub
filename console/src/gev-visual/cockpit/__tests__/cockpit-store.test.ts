import { describe, expect, test, vi } from "vitest";
import {
  cockpitReducer,
  createCockpitStore,
  INITIAL_COCKPIT_STATE,
} from "../cockpit-store";
import type { CockpitStoreState } from "../cockpit-store";

describe("cockpitReducer (pure)", () => {
  test("initial state is inactive with no tracked id and optical vision", () => {
    expect(INITIAL_COCKPIT_STATE).toEqual({
      active: false,
      trackedId: null,
      visionMode: "optical",
      briefingPaused: false,
    });
  });

  test("enter(id) activates and records the tracked id", () => {
    const s = cockpitReducer(INITIAL_COCKPIT_STATE, { type: "enter", id: "abc123" });
    expect(s.active).toBe(true);
    expect(s.trackedId).toBe("abc123");
  });

  test("enter(id) re-arms the briefing rotation (unpauses)", () => {
    const paused: CockpitStoreState = {
      ...INITIAL_COCKPIT_STATE,
      active: true,
      trackedId: "x",
      briefingPaused: true,
    };
    const s = cockpitReducer(paused, { type: "enter", id: "abc123" });
    expect(s.briefingPaused).toBe(false);
  });

  test("exit() clears active + tracked id and unpauses", () => {
    const active: CockpitStoreState = {
      ...INITIAL_COCKPIT_STATE,
      active: true,
      trackedId: "abc123",
      briefingPaused: true,
    };
    const s = cockpitReducer(active, { type: "exit" });
    expect(s.active).toBe(false);
    expect(s.trackedId).toBeNull();
    expect(s.briefingPaused).toBe(false);
  });

  test("exit() preserves visionMode (user preference)", () => {
    const active: CockpitStoreState = {
      ...INITIAL_COCKPIT_STATE,
      active: true,
      trackedId: "abc123",
      visionMode: "thermal",
    };
    const s = cockpitReducer(active, { type: "exit" });
    expect(s.visionMode).toBe("thermal");
  });

  test("setVisionMode updates visionMode", () => {
    const s = cockpitReducer(INITIAL_COCKPIT_STATE, {
      type: "setVisionMode",
      mode: "noir",
    });
    expect(s.visionMode).toBe("noir");
  });

  test("pauseBriefing / resumeBriefing toggle briefingPaused", () => {
    const paused = cockpitReducer(INITIAL_COCKPIT_STATE, { type: "pauseBriefing" });
    expect(paused.briefingPaused).toBe(true);
    const resumed = cockpitReducer(paused, { type: "resumeBriefing" });
    expect(resumed.briefingPaused).toBe(false);
  });

  test("reducer returns a NEW object (immutability)", () => {
    const next = cockpitReducer(INITIAL_COCKPIT_STATE, { type: "enter", id: "a" });
    expect(next).not.toBe(INITIAL_COCKPIT_STATE);
    expect(INITIAL_COCKPIT_STATE.active).toBe(false); // original untouched
  });
});

describe("createCockpitStore", () => {
  test("getState reflects the initial override", () => {
    const store = createCockpitStore({ visionMode: "nvg" });
    expect(store.getState().visionMode).toBe("nvg");
    expect(store.getState().active).toBe(false);
  });

  test("the five actions drive state transitions through the store", () => {
    const store = createCockpitStore();
    store.enter("abc123");
    expect(store.getState()).toMatchObject({ active: true, trackedId: "abc123" });
    store.setVisionMode("thermal");
    expect(store.getState().visionMode).toBe("thermal");
    store.pauseBriefing();
    expect(store.getState().briefingPaused).toBe(true);
    store.resumeBriefing();
    expect(store.getState().briefingPaused).toBe(false);
    store.exit();
    expect(store.getState()).toMatchObject({ active: false, trackedId: null });
  });

  test("subscribe fires on every transition and unsubscribe stops it", () => {
    const store = createCockpitStore();
    const fn = vi.fn();
    const off = store.subscribe(fn);
    store.enter("a");
    store.setVisionMode("crt");
    expect(fn).toHaveBeenCalledTimes(2);
    off();
    store.exit();
    expect(fn).toHaveBeenCalledTimes(2); // no further notifications
  });

  test("subscribe delivers the NEW state, not the previous one", () => {
    const store = createCockpitStore();
    let seen: CockpitStoreState | null = null;
    store.subscribe((s) => {
      seen = s;
    });
    store.enter("abc123");
    expect(seen!.active).toBe(true);
    expect(seen!.trackedId).toBe("abc123");
  });

  test("enter() replays persisted vision mode onto vision handle", () => {
    const vision = { setMode: vi.fn() };
    const store = createCockpitStore(
      { visionMode: "crt" },
      { getVision: () => vision },
    );
    store.enter("flight-1");
    expect(vision.setMode).toHaveBeenCalledWith("crt");
    expect(vision.setMode).toHaveBeenCalledTimes(1);
  });

  test("enter() does not replay optical (no-op)", () => {
    const vision = { setMode: vi.fn() };
    const store = createCockpitStore(
      { visionMode: "optical" },
      { getVision: () => vision },
    );
    store.enter("flight-1");
    expect(vision.setMode).not.toHaveBeenCalled();
  });
});
