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
      hidden: false,
      elementVisibility: {
        compass: true,
        altimeter: true,
        speedRuler: true,
        altitudeLadder: true,
        speedTape: true,
        pitchLadder: true,
        bankIndicator: true,
        vsiChevron: true,
      },
      replayState: {
        isRecording: false,
        recordingSegmentId: null,
        lastSavedSegmentId: null,
        isPlaying: false,
        playbackSegmentId: null,
        playbackTimeMs: 0,
        playbackSpeed: 1,
      },
      svsEnabled: false,
      tcasEnabled: false,
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

  test("toggleHidden flips hidden; enter/exit reset it to visible", () => {
    const active: CockpitStoreState = {
      ...INITIAL_COCKPIT_STATE,
      active: true,
      trackedId: "abc123",
    };
    const hidden = cockpitReducer(active, { type: "toggleHidden" });
    expect(hidden.hidden).toBe(true);
    expect(cockpitReducer(hidden, { type: "toggleHidden" }).hidden).toBe(false);
    expect(cockpitReducer(hidden, { type: "enter", id: "abc123" }).hidden).toBe(false);
    expect(cockpitReducer(hidden, { type: "exit" }).hidden).toBe(false);
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

// ── GEV P17: elementVisibility field + actions ────────────────────────

describe("cockpit-store element visibility (GEV P17)", () => {
  test("createCockpitStore initializes elementVisibility to all-true", () => {
    const store = createCockpitStore();
    const v = store.getState().elementVisibility;
    expect(v.compass).toBe(true);
    expect(v.altimeter).toBe(true);
    expect(v.speedRuler).toBe(true);
    expect(v.altitudeLadder).toBe(true);
    expect(v.speedTape).toBe(true);
    expect(v.pitchLadder).toBe(true);
    expect(v.bankIndicator).toBe(true);
    expect(v.vsiChevron).toBe(true);
  });

  test("toggleElement flips one key", () => {
    const store = createCockpitStore();
    store.toggleElement("speedTape");
    expect(store.getState().elementVisibility.speedTape).toBe(false);
    store.toggleElement("speedTape");
    expect(store.getState().elementVisibility.speedTape).toBe(true);
  });

  test("toggleElement does not flip other keys", () => {
    const store = createCockpitStore();
    store.toggleElement("pitchLadder");
    const after = store.getState().elementVisibility;
    expect(after.pitchLadder).toBe(false);
    expect(after.compass).toBe(true);
    expect(after.bankIndicator).toBe(true);
    expect(after.vsiChevron).toBe(true);
  });

  test("setElementVisibility merges partial (sparse update)", () => {
    const store = createCockpitStore();
    store.setElementVisibility({ speedTape: false, pitchLadder: false });
    const after = store.getState().elementVisibility;
    expect(after.speedTape).toBe(false);
    expect(after.pitchLadder).toBe(false);
    expect(after.compass).toBe(true);
    expect(after.altimeter).toBe(true);
  });

  test("enter() preserves elementVisibility (user preference, not reset)", () => {
    const store = createCockpitStore();
    store.setElementVisibility({ speedTape: false });
    store.enter("abc");
    expect(store.getState().elementVisibility.speedTape).toBe(false);
  });

  test("exit() preserves elementVisibility (user preference, not reset)", () => {
    const store = createCockpitStore();
    store.enter("abc");
    store.setElementVisibility({ speedTape: false });
    store.exit();
    expect(store.getState().elementVisibility.speedTape).toBe(false);
  });

  test("subscribe fires on toggleElement", () => {
    const store = createCockpitStore();
    const fn = vi.fn();
    store.subscribe(fn);
    store.toggleElement("compass");
    store.setElementVisibility({ altimeter: false });
    expect(fn).toHaveBeenCalledTimes(2);
  });
});

// ── GEV §6.3: replayState field + actions ───────────────────────

describe("cockpit-store replay state (GEV §6.3)", () => {
  test("initial state has replayState default", () => {
    const store = createCockpitStore();
    expect(store.getState().replayState).toEqual({
      isRecording: false,
      recordingSegmentId: null,
      lastSavedSegmentId: null,
      isPlaying: false,
      playbackSegmentId: null,
      playbackTimeMs: 0,
      playbackSpeed: 1,
    });
    expect(store.getState().svsEnabled).toBe(false);
    expect(store.getState().tcasEnabled).toBe(false);
  });

  test("startRecording dispatches and exposes segment id", () => {
    const store = createCockpitStore();
    store.startRecording("seg-1");
    const r = store.getState().replayState;
    expect(r.recordingSegmentId).toBe("seg-1");
    expect(r.isRecording).toBe(true);
    expect(r.isPlaying).toBe(false);
  });

  test("stopRecording clears recording state, keeps segment id as lastSaved", () => {
    const store = createCockpitStore();
    store.startRecording("seg-1");
    store.stopRecording("seg-1");
    const r = store.getState().replayState;
    expect(r.isRecording).toBe(false);
    expect(r.recordingSegmentId).toBeNull();
    expect(r.lastSavedSegmentId).toBe("seg-1");
  });

  test("startPlayback transitions to playing", () => {
    const store = createCockpitStore();
    store.startPlayback("seg-1", 0);
    const r = store.getState().replayState;
    expect(r.isPlaying).toBe(true);
    expect(r.playbackSegmentId).toBe("seg-1");
    expect(r.playbackTimeMs).toBe(0);
  });

  test("seekPlayback updates time without leaving playback", () => {
    const store = createCockpitStore();
    store.startPlayback("seg-1", 0);
    store.seekPlayback(5000);
    expect(store.getState().replayState.playbackTimeMs).toBe(5000);
    expect(store.getState().replayState.isPlaying).toBe(true);
  });

  test("stopPlayback clears all playback state", () => {
    const store = createCockpitStore();
    store.startPlayback("seg-1", 0);
    store.seekPlayback(2000);
    store.stopPlayback();
    const r = store.getState().replayState;
    expect(r.isPlaying).toBe(false);
    expect(r.playbackSegmentId).toBeNull();
    expect(r.playbackTimeMs).toBe(0);
  });

  test("setPlaybackSpeed updates playbackSpeed", () => {
    const store = createCockpitStore();
    store.setPlaybackSpeed(2);
    expect(store.getState().replayState.playbackSpeed).toBe(2);
  });

  test("enter() preserves replayState (user preference)", () => {
    const store = createCockpitStore();
    store.startRecording("seg-1");
    store.enter("abc");
    expect(store.getState().replayState.isRecording).toBe(true);
  });

  test("exit() preserves replayState (user preference)", () => {
    const store = createCockpitStore();
    store.startPlayback("seg-1", 0);
    store.exit();
    expect(store.getState().replayState.isPlaying).toBe(true);
  });
});
