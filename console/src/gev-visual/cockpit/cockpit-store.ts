// GEV P9 cockpit store — a tiny pure-reducer React state holder for the cockpit
// overlay lifecycle (spec §0: 重写状态机 in React, no cockpitController).
//
// Five actions (plan Task 3): enter(id) / exit() / setVisionMode(mode) /
// pauseBriefing() / resumeBriefing(). The reducer is exported for direct
// transition testing; createCockpitStore wraps it with subscription.
//
// GEV P17 extension: elementVisibility (per-element show/hide map for the
// 8 cockpit HUD elements) + toggleElement / setElementVisibility actions.
// Visibility persists across enter/exit like visionMode — user preference,
// not session state.
//
// GEV §6.3 extension: replayState (recording/playback lifecycle) +
// 6 actions (start/stopRecording, start/seek/stopPlayback, setPlaybackSpeed).
// Persists across enter/exit (user preference).
import type { VisionMode } from "./vision-mount";
import type { ElementVisibility } from "./element-visibility";
import {
  DEFAULT_ELEMENT_VISIBILITY,
  type CockpitElementKey,
} from "./element-visibility";
import type { ReplaySpeed, ReplayState } from "./replay-types";
import { DEFAULT_REPLAY_STATE } from "./replay-types";

export interface CockpitStoreState {
  active: boolean;
  trackedId: string | null;
  visionMode: VisionMode;
  briefingPaused: boolean;
  /** Shift+C (T5): panels hidden so the globe is unobstructed. Reset on both
   *  enter and exit so every session starts visible. */
  hidden: boolean;
  /** GEV P17: per-element show/hide map for the 8 cockpit HUD elements.
   *  Persists across enter/exit like visionMode (user preference). */
  elementVisibility: ElementVisibility;
  /** GEV §6.3: replay recording/playback state. */
  replayState: ReplayState;
  /** GEV P19 SVS: synthetic vision toggle (terrain overlay on pitch
   *  ladder). Persists across enter/exit like visionMode (user
   *  preference). Defaults to false so the overlay doesn't clutter
   *  standard optical-mode flying. */
  svsEnabled: boolean;
  /** GEV P20 TCAS: traffic collision avoidance toggle (proximity
   *  diamonds + altitude bars). Defaults to false — TCAS is opt-in
   *  like SVS. */
  tcasEnabled: boolean;
}

export type CockpitAction =
  | { type: "enter"; id: string }
  | { type: "exit" }
  | { type: "setVisionMode"; mode: VisionMode }
  | { type: "pauseBriefing" }
  | { type: "resumeBriefing" }
  | { type: "toggleHidden" }
  | { type: "toggleElement"; key: CockpitElementKey }
  | { type: "setElementVisibility"; visibility: Partial<ElementVisibility> }
  | { type: "startRecording"; segmentId: string }
  | { type: "stopRecording"; segmentId: string }
  | { type: "startPlayback"; segmentId: string; startMs: number }
  | { type: "seekPlayback"; timeMs: number }
  | { type: "stopPlayback" }
  | { type: "setPlaybackSpeed"; speed: ReplaySpeed }
  | { type: "setSvsEnabled"; enabled: boolean }
  | { type: "setTcasEnabled"; enabled: boolean };

export const INITIAL_COCKPIT_STATE: CockpitStoreState = {
  active: false,
  trackedId: null,
  visionMode: "optical",
  briefingPaused: false,
  hidden: false,
  elementVisibility: { ...DEFAULT_ELEMENT_VISIBILITY },
  replayState: { ...DEFAULT_REPLAY_STATE },
  svsEnabled: false,
  tcasEnabled: false,
};

export function cockpitReducer(
  state: CockpitStoreState,
  action: CockpitAction,
): CockpitStoreState {
  switch (action.type) {
    case "enter":
      // A fresh entry starts with the briefing rotation unpaused and the
      // panels visible.
      return {
        ...state,
        active: true,
        trackedId: action.id,
        briefingPaused: false,
        hidden: false,
      };
    case "exit":
      // Leaving clears the tracked id and re-arms the briefing rotation so the
      // next entry begins fresh. visionMode persists (user preference).
      return {
        ...state,
        active: false,
        trackedId: null,
        briefingPaused: false,
        hidden: false,
      };
    case "setVisionMode":
      return { ...state, visionMode: action.mode };
    case "pauseBriefing":
      return { ...state, briefingPaused: true };
    case "resumeBriefing":
      return { ...state, briefingPaused: false };
    case "toggleHidden":
      return { ...state, hidden: !state.hidden };
    case "toggleElement":
      return {
        ...state,
        elementVisibility: {
          ...state.elementVisibility,
          [action.key]: !state.elementVisibility[action.key],
        },
      };
    case "setElementVisibility":
      return {
        ...state,
        elementVisibility: { ...state.elementVisibility, ...action.visibility },
      };
    // GEV §6.3: replay recording/playback lifecycle.
    case "startRecording":
      return {
        ...state,
        replayState: {
          ...state.replayState,
          isRecording: true,
          recordingSegmentId: action.segmentId,
        },
      };
    case "stopRecording":
      return {
        ...state,
        replayState: {
          ...state.replayState,
          isRecording: false,
          recordingSegmentId: null,
          lastSavedSegmentId: action.segmentId,
        },
      };
    case "startPlayback":
      return {
        ...state,
        replayState: {
          ...state.replayState,
          isPlaying: true,
          playbackSegmentId: action.segmentId,
          playbackTimeMs: action.startMs,
        },
      };
    case "seekPlayback":
      return {
        ...state,
        replayState: {
          ...state.replayState,
          playbackTimeMs: action.timeMs,
        },
      };
    case "stopPlayback":
      return {
        ...state,
        replayState: {
          ...state.replayState,
          isPlaying: false,
          playbackSegmentId: null,
          playbackTimeMs: 0,
        },
      };
    case "setPlaybackSpeed":
      return {
        ...state,
        replayState: {
          ...state.replayState,
          playbackSpeed: action.speed,
        },
      };
    case "setSvsEnabled":
      return { ...state, svsEnabled: action.enabled };
    case "setTcasEnabled":
      return { ...state, tcasEnabled: action.enabled };
  }
}

export interface CockpitStore {
  getState(): CockpitStoreState;
  enter(id: string): void;
  exit(): void;
  setVisionMode(mode: VisionMode): void;
  pauseBriefing(): void;
  resumeBriefing(): void;
  toggleHidden(): void;
  toggleElement(key: CockpitElementKey): void;
  setElementVisibility(visibility: Partial<ElementVisibility>): void;
  // GEV §6.3: replay lifecycle.
  startRecording(segmentId: string): void;
  stopRecording(segmentId: string): void;
  startPlayback(segmentId: string, startMs: number): void;
  seekPlayback(timeMs: number): void;
  stopPlayback(): void;
  setPlaybackSpeed(speed: ReplaySpeed): void;
  // GEV P19 SVS toggle.
  setSvsEnabled(enabled: boolean): void;
  // GEV P20 TCAS toggle.
  setTcasEnabled(enabled: boolean): void;
  subscribe(fn: (state: CockpitStoreState) => void): () => void;
}

/** Side-effect dependencies for the store. `getVision` lets `enter()` replay
 *  the persisted vision mode onto the live vision handle (mounted
 *  asynchronously, so resolved lazily rather than passed by value). */
export interface CockpitStoreDeps {
  getVision?: () => { setMode(mode: VisionMode): unknown } | null;
}

export function createCockpitStore(
  initial: Partial<CockpitStoreState> = {},
  deps: CockpitStoreDeps = {},
): CockpitStore {
  let state: CockpitStoreState = { ...INITIAL_COCKPIT_STATE, ...initial };
  const listeners = new Set<(state: CockpitStoreState) => void>();

  function dispatch(action: CockpitAction): void {
    const next = cockpitReducer(state, action);
    if (next === state) return;
    state = next;
    for (const fn of [...listeners]) fn(state);
  }

  return {
    getState: () => state,
    enter: (id) => {
      dispatch({ type: "enter", id });
      // D2 re-entry fix: replay the persisted vision mode (seeded into
      // state.visionMode from localStorage upstream) onto the live vision
      // handle so the visual effect matches the highlighted mode after a
      // reload. optical is a no-op — the handle already defaults to it.
      const vision = deps.getVision?.();
      if (vision && state.visionMode !== "optical") {
        vision.setMode(state.visionMode);
      }
    },
    exit: () => dispatch({ type: "exit" }),
    setVisionMode: (mode) => dispatch({ type: "setVisionMode", mode }),
    pauseBriefing: () => dispatch({ type: "pauseBriefing" }),
    resumeBriefing: () => dispatch({ type: "resumeBriefing" }),
    toggleHidden: () => dispatch({ type: "toggleHidden" }),
    toggleElement: (key) => dispatch({ type: "toggleElement", key }),
    setElementVisibility: (visibility) =>
      dispatch({ type: "setElementVisibility", visibility }),
    // GEV §6.3: replay lifecycle methods.
    startRecording: (segmentId) =>
      dispatch({ type: "startRecording", segmentId }),
    stopRecording: (segmentId) =>
      dispatch({ type: "stopRecording", segmentId }),
    startPlayback: (segmentId, startMs) =>
      dispatch({ type: "startPlayback", segmentId, startMs }),
    seekPlayback: (timeMs) => dispatch({ type: "seekPlayback", timeMs }),
    stopPlayback: () => dispatch({ type: "stopPlayback" }),
    setPlaybackSpeed: (speed) =>
      dispatch({ type: "setPlaybackSpeed", speed }),
    // GEV P19 SVS toggle.
    setSvsEnabled: (enabled) => dispatch({ type: "setSvsEnabled", enabled }),
    // GEV P20 TCAS toggle.
    setTcasEnabled: (enabled) =>
      dispatch({ type: "setTcasEnabled", enabled }),
    subscribe(fn) {
      listeners.add(fn);
      return () => {
        listeners.delete(fn);
      };
    },
  };
}
