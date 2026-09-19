// GEV P10 T2 — scene-controls adapter.
//
// Vendor shape (verified at console/gev-engine/src/ui/sceneControls.js):
//   new SceneControls({ read, actions, subscribe, elements? })
//     .present({ state, change?, initial? })
//     .destroy()                    — idempotent
//
// `read` returns the full project state snapshot.
// `actions` is a free-form map keyed by name; the vendor calls them as
// `actions[action](...args)`. The vendor's exact action dispatch set (from
// sceneControls.js) includes: selectScene, capture, update, next, export,
// download, import (or reviewImport when present), create, deleteScene,
// deleteShot, start, stop, renameShot, selectShot, load.
// The brief asks for 8 specific actions: capture/update/next/export/download/
// import/start/stop/reviewImport — all present in the vendor dispatch set.
//
// `subscribe(fn)` notifies on state changes; vendor calls fn({ state, change,
// initial }). The adapter hooks `controls.present` (the public hook vendor
// itself calls per change) to fan notifications out to consumer-registered
// listeners via onChange(fn) → unsubscribe.
//
// `elements` defaults to sceneElements() (vendor self-lookup). Tests pass an
// explicit elements map.
import { SceneControls } from "gev-engine/src/ui/sceneControls.js";

/** Project state surface — what `read` returns. Subset of the vendor's full
 *  shape (the vendor reads every field on construction). */
export interface SceneProjectState {
  scenes: Array<{
    id: string;
    title: string;
    shots: Array<{ id: string; title: string }>;
  }>;
  selectedSceneId: string | null;
  selectedShotId: string | null;
  running: boolean;
  hasRun: boolean;
  progress: number;
  status: string;
  runtime: string;
  playbackActive: boolean;
  keyboardEnabled: boolean;
}

/** Vendor action surface — 8 actions from the brief, plus the implicit
 *  'create'/'deleteScene'/'renameShot'/'selectShot'/'load'/'deleteShot'
 *  actions the vendor itself calls. */
export interface SceneActions {
  selectScene(id: string): unknown;
  capture(): unknown;
  update(): unknown;
  next(): unknown;
  export(): unknown;
  download(): unknown;
  import(file: File): unknown;
  start(sceneId: string | null): unknown;
  stop(reason: string): unknown;
  /** When present, vendor calls 'reviewImport' instead of 'import'. */
  reviewImport?(file: File): unknown;
  create?(name: string): unknown;
  deleteScene?(): unknown;
  deleteShot?(sceneId: string, shotId: string): unknown;
  renameShot?(sceneId: string, shotId: string, title: string): unknown;
  selectShot?(id: string): unknown;
  load?(sceneId: string, shotId: string): unknown;
  /** Vendor dispatch index signature — vendor may call any name. */
  [actionName: string]: unknown;
}

export type SceneChangeType =
  | "scene-options-changed"
  | "scene-created"
  | "scene-deleted"
  | "shots-changed"
  | "shot-captured"
  | "shot-updated"
  | "shot-renamed"
  | "shot-deleted"
  | "selection-changed"
  | "buttons-changed"
  | "progress-changed"
  | "status-changed"
  | "runtime-changed"
  | "playback-presentation"
  | "playback-keyboard"
  | "project-imported"
  | "project-exported"
  | "shot-loaded"
  | "run-event";

export interface SceneChangeNotification {
  state: SceneProjectState;
  change?: { type: SceneChangeType; [key: string]: unknown };
  initial?: boolean;
}

export interface SceneElements {
  select?: HTMLElement | null;
  capture?: HTMLElement | null;
  update?: HTMLElement | null;
  next?: HTMLElement | null;
  export?: HTMLElement | null;
  download?: HTMLElement | null;
  new?: HTMLElement | null;
  delete?: HTMLElement | null;
  start?: HTMLElement | null;
  stop?: HTMLElement | null;
  import?: HTMLElement | null;
  file?: HTMLInputElement | null;
  shots?: HTMLElement | null;
  progress?: HTMLElement | null;
  status?: HTMLElement | null;
  runtime?: HTMLElement | null;
}

export interface SceneControlsHandle {
  /** Vendor handle (typed for tests / advanced use). */
  readonly controls: InstanceType<typeof SceneControls>;
  /** Forwarded notification consumer — receive({state, change, initial}).
   *  Returns an unsubscribe closure. */
  onChange(fn: (n: SceneChangeNotification) => void): () => void;
  /** Idempotent destroy. */
  destroy(): void;
}

export interface SceneControlsOptions {
  read(): SceneProjectState;
  actions: SceneActions;
  elements?: SceneElements | null;
}

export function mountSceneControls(
  opts: SceneControlsOptions,
): SceneControlsHandle {
  // Fan-out set: every consumer registered via onChange(fn) receives every
  // vendor notification. The vendor pushes notifications through
  // `this.unsubscribe?.()`-style subscribe fn, which routes into
  // `this.present(...)`. We wrap `present` AFTER construction so our wrapper
  // fires in addition to vendor's internal presentation logic.
  const consumerListeners = new Set<
    (n: SceneChangeNotification) => void
  >();

  const controls = new SceneControls({
    read: opts.read as () => unknown,
    actions: opts.actions as unknown as Record<
      string,
      (...args: unknown[]) => unknown
    >,
    elements: opts.elements
      ? (opts.elements as unknown as Record<string, unknown>)
      : undefined,
    // subscribe is the vendor's notification source; we pass undefined and
    // rely on vendor's no-subscribe branch (renderSceneSelect + renderShotList
    // on construction, plus present() invoked externally). The adapter's
    // onChange() consumers attach via the present() wrapper below.
    subscribe: undefined,
  });

  // Wrap present() so every vendor notification ALSO reaches consumers.
  // Vendor calls present() internally on destroy (e.g. setPlaybackActive(false))
  // and on every state push — our wrapper forwards to the original then
  // broadcasts to consumers.
  const originalPresent = controls.present.bind(controls);
  controls.present = ((n: SceneChangeNotification) => {
    originalPresent(n as Parameters<typeof originalPresent>[0]);
    for (const listener of [...consumerListeners]) listener(n);
  }) as typeof controls.present;

  return {
    controls,
    onChange(fn) {
      consumerListeners.add(fn);
      return () => {
        consumerListeners.delete(fn);
      };
    },
    destroy() {
      consumerListeners.clear();
      controls.destroy();
    },
  };
}