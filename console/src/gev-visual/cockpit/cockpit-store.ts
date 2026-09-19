// GEV P9 cockpit store — a tiny pure-reducer React state holder for the cockpit
// overlay lifecycle (spec §0: 重写状态机 in React, no cockpitController).
//
// Five actions (plan Task 3): enter(id) / exit() / setVisionMode(mode) /
// pauseBriefing() / resumeBriefing(). The reducer is exported for direct
// transition testing; createCockpitStore wraps it with subscription.
import type { VisionMode } from "./vision-mount";

export interface CockpitStoreState {
  active: boolean;
  trackedId: string | null;
  visionMode: VisionMode;
  briefingPaused: boolean;
}

export type CockpitAction =
  | { type: "enter"; id: string }
  | { type: "exit" }
  | { type: "setVisionMode"; mode: VisionMode }
  | { type: "pauseBriefing" }
  | { type: "resumeBriefing" };

export const INITIAL_COCKPIT_STATE: CockpitStoreState = {
  active: false,
  trackedId: null,
  visionMode: "optical",
  briefingPaused: false,
};

export function cockpitReducer(
  state: CockpitStoreState,
  action: CockpitAction,
): CockpitStoreState {
  switch (action.type) {
    case "enter":
      // A fresh entry starts with the briefing rotation unpaused.
      return { ...state, active: true, trackedId: action.id, briefingPaused: false };
    case "exit":
      // Leaving clears the tracked id and re-arms the briefing rotation so the
      // next entry begins fresh. visionMode persists (user preference).
      return { ...state, active: false, trackedId: null, briefingPaused: false };
    case "setVisionMode":
      return { ...state, visionMode: action.mode };
    case "pauseBriefing":
      return { ...state, briefingPaused: true };
    case "resumeBriefing":
      return { ...state, briefingPaused: false };
  }
}

export interface CockpitStore {
  getState(): CockpitStoreState;
  enter(id: string): void;
  exit(): void;
  setVisionMode(mode: VisionMode): void;
  pauseBriefing(): void;
  resumeBriefing(): void;
  subscribe(fn: (state: CockpitStoreState) => void): () => void;
}

export function createCockpitStore(
  initial: Partial<CockpitStoreState> = {},
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
    enter: (id) => dispatch({ type: "enter", id }),
    exit: () => dispatch({ type: "exit" }),
    setVisionMode: (mode) => dispatch({ type: "setVisionMode", mode }),
    pauseBriefing: () => dispatch({ type: "pauseBriefing" }),
    resumeBriefing: () => dispatch({ type: "resumeBriefing" }),
    subscribe(fn) {
      listeners.add(fn);
      return () => {
        listeners.delete(fn);
      };
    },
  };
}
