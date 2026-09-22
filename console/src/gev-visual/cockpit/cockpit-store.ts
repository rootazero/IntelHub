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
import type { VisionMode } from "./vision-mount";
import type { ElementVisibility } from "./element-visibility";
import {
  DEFAULT_ELEMENT_VISIBILITY,
  type CockpitElementKey,
} from "./element-visibility";

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
}

export type CockpitAction =
  | { type: "enter"; id: string }
  | { type: "exit" }
  | { type: "setVisionMode"; mode: VisionMode }
  | { type: "pauseBriefing" }
  | { type: "resumeBriefing" }
  | { type: "toggleHidden" }
  | { type: "toggleElement"; key: CockpitElementKey }
  | { type: "setElementVisibility"; visibility: Partial<ElementVisibility> };

export const INITIAL_COCKPIT_STATE: CockpitStoreState = {
  active: false,
  trackedId: null,
  visionMode: "optical",
  briefingPaused: false,
  hidden: false,
  elementVisibility: { ...DEFAULT_ELEMENT_VISIBILITY },
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
    subscribe(fn) {
      listeners.add(fn);
      return () => {
        listeners.delete(fn);
      };
    },
  };
}
