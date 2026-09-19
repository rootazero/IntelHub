// GEV P10 T2 — scene-sharing adapter.
//
// Vendor shape (verified at console/gev-engine/src/ui/sceneSharing.js):
//   createSceneDialog(title, onClose)
//     → { body, footer, status, text, button, input, listen, element, dispose }
//   mountSceneSharing({ edit, share }, panel?)
//     → () => void   — disposer
//
// Both vendor functions are self-contained DOM factories (the dialog appends
// a `<dialog>` to document.body and shows it modal; mountSceneSharing appends
// a button bar to the supplied panel). The brief asks for an adapter that
// exposes `createDialog()` and `mount(panel, opts): Handle`.
//
// createDialog wraps createSceneDialog so React can capture the dispose()
// handle (vendor returns it inline). mount() returns a Handle whose destroy()
// forwards to the vendor's returned disposer.
//
// NOT directly importing vendor module class — vendor has no class here; both
// exports are factory functions. The adapter wraps each so consumers don't
// need to remember the `{ edit, share }` payload shape (we accept separate
// edit/share callbacks and assemble the payload).
import {
  createSceneDialog as vendorCreateSceneDialog,
  mountSceneSharing as vendorMountSceneSharing,
} from "gev-engine/src/ui/sceneSharing.js";

export interface SceneDialogHandle {
  body: HTMLElement;
  footer: HTMLElement;
  status: HTMLElement;
  text(value: string, parent?: HTMLElement): HTMLElement;
  button(
    label: string,
    fn: () => void,
    parent?: HTMLElement,
  ): HTMLButtonElement;
  input(
    label: string,
    value: string,
    options?: { type?: string; multiline?: boolean },
  ): HTMLElement;
  listen(node: HTMLElement, type: string, fn: EventListener): void;
  element: HTMLElement;
  dispose(): void;
}

export interface SceneSharingMountHandle {
  destroy(): void;
}

export interface SceneSharingOptions {
  /** EDIT DETAILS click handler. */
  edit(): void;
  /** SHARE SCENE click handler. */
  share(): void;
}

/** Vendor pass-through. Returns a SceneDialogHandle wrapping the vendor's
 *  returned object — same shape, adapter typing only. */
export function createSceneDialog(
  title: string,
  onClose: () => void,
): SceneDialogHandle {
  // Vendor return: { body, footer, status, text, button, input, listen, element, dispose }
  const v = vendorCreateSceneDialog(title, onClose);
  return v as unknown as SceneDialogHandle;
}

/** Vendor pass-through — adapter wraps the bare disposer in a Handle. */
export function mountSceneSharing(
  panel: HTMLElement,
  opts: SceneSharingOptions,
): SceneSharingMountHandle {
  const dispose = vendorMountSceneSharing(
    { edit: opts.edit, share: opts.share },
    panel,
  );
  let destroyed = false;
  return {
    destroy() {
      if (destroyed) return;
      destroyed = true;
      dispose();
    },
  };
}