// GEV P10 T2 — shortcuts adapter.
//
// Vendor shape (verified at console/gev-engine/src/ui/applicationShortcuts.js):
//   bindApplicationShortcuts({ documentRef, searchInput, actions })
//     → { destroy() }
//
// Vendor keys (the brief's 8-action contract):
//   1-7       → actions.setStyle(STYLE_KEYS[key])       // 1=normal..7=snow
//   Escape    → actions.dismissSearch()
//   h         → actions.toggleHud()
//   o         → actions.toggleOrbit()
//   v         → actions.toggleCleanView()
//   f         → actions.toggleLayers()
//   d         → actions.cycleDetection()
//   c         → actions.toggleCctv()
//
// The brief asks for adapter-supplied action implementations that route into
// the IntelHub HUD store (HudTopBar's `clear`, HudFrame's `data-visible`
// toggle, HudLayerRail's expand/collapse, HudDetailPanel's tab=cctv, etc.).
// Two actions are NO-OPs with a console.warn (vendor-only surface):
//   - toggleOrbit     — vendor Cesium orbit tracker, no IntelHub equivalent
//   - toggleCleanView — vendor shell-level clean-view, no IntelHub equivalent
//   - cycleDetection  — vendor detection policy; not wired in P9
//
// R5 CARRY (P9): setStyle must gate on !cockpitStore.active — the cockpit's
// vision policy owns the post-process stages while active (see
// gev-visual/cockpit/style-gate.ts). This prevents the keyboard shortcut
// from clobbering vision-driven stage changes.
//
// The cheatsheet `?` pop-key is NOT in the vendor keyset; the adapter adds
// it so T3 can wire the cheatsheet HUD overlay.
//
// `StyleSetterHandle` is the P6 / P9 GatedStyleControl surface (style-gate.ts
// re-exports it as `GatedStyleControl`). The adapter takes either a plain
// `{setStyle(s): void}` (no gating) or a `GatedStyleControl` (which already
// gates on cockpit). Either is fine — the gate lives in style-gate.ts; we
// simply forward.
import { bindApplicationShortcuts } from "gev-engine/src/ui/applicationShortcuts.js";
import { isGlobeStyle } from "../visual-effects";
import type { CockpitStore } from "../cockpit/cockpit-store";

/** Subset of the GatedStyleControl (style-gate.ts) the adapter needs. */
export interface ShortcutsStyleSetter {
  setStyle(style: string): void;
}

/** Adapter-supplied action surface — the 8 brief-mandated actions. */
export interface ShortcutsActions {
  setStyle(style: string): void;
  dismissSearch(): void;
  toggleHud(): void;
  toggleOrbit(): void;
  toggleCleanView(): void;
  toggleLayers(): void;
  cycleDetection(): void;
  toggleCctv(): void;
  /** Cheatsheet pop-key (`?`) — adapter extension, not in vendor keyset. */
  toggleCheatsheet?(): void;
}

export interface ShortcutsHandle {
  destroy(): void;
}

export interface ShortcutsOptions {
  documentRef?: Document;
  searchInput?: HTMLElement | null;
  actions: ShortcutsActions;
  /** Required for R5 gate (P9 style-gate). When active, setStyle is NO-OP. */
  cockpitStore?: Pick<CockpitStore, "getState">;
}

/** Build the vendor action map from the 8 brief actions + R5 gate. The
 *  `setStyle` wrapper reads `cockpitStore.getState().active` on every key
 *  press (cheap — a single state read). If cockpit is active, the vendor's
 *  style change is suppressed and the cockpit's vision policy retains
 *  ownership of the post-process stages. */
function buildActions(
  actions: ShortcutsActions,
  cockpitStore?: Pick<CockpitStore, "getState">,
): Record<string, (...args: unknown[]) => unknown> {
  return {
    setStyle: (...args: unknown[]) => {
      const name = args[0] as string;
      // R5 GATE: cockpit vision policy owns the post-process stages while
      // active. The keyboard shortcut is recorded by the underlying
      // `actions.setStyle` (which routes through the P9 GatedStyleControl
      // when one is provided), so vision still receives the user pick once
      // the cockpit exits. We add an extra guard here in case the caller
      // passed a raw setStyle that doesn't gate.
      if (cockpitStore?.getState().active) {
        // No-op while cockpit owns the stages; record on exit via the
        // GatedStyleControl's restorePicked path in the page wiring.
        return;
      }
      if (!isGlobeStyle(name)) {
        // Unknown style: ignore (defensive — vendor's STYLE_KEYS map is
        // exhaustive over our GLOBE_STYLES, but future vendor additions
        // would silently route here).
        return;
      }
      actions.setStyle(name);
    },
    dismissSearch: () => actions.dismissSearch(),
    toggleHud: () => actions.toggleHud(),
    toggleOrbit: () => actions.toggleOrbit(),
    toggleCleanView: () => actions.toggleCleanView(),
    toggleLayers: () => actions.toggleLayers(),
    cycleDetection: () => actions.cycleDetection(),
    toggleCctv: () => actions.toggleCctv(),
  };
}

/** Bind the vendor's bubbling keyboard listener with the 8-action map. The
 *  cheatsheet `?` key is adapter-supplied — we add a second listener (the
 *  vendor's onKeyDown ignores it because `?` is not in STYLE_KEYS or any
 *  vendor key). */
export function bindShortcuts(opts: ShortcutsOptions): ShortcutsHandle {
  const doc =
    opts.documentRef ?? (typeof document !== "undefined" ? document : null);
  if (!doc) {
    // SSR / no-DOM environment: return an inert handle.
    return { destroy() {} };
  }

  const vendorHandle = bindApplicationShortcuts({
    documentRef: doc,
    searchInput: opts.searchInput ?? undefined,
    actions: buildActions(opts.actions, opts.cockpitStore),
  });

  // Adapter extension: cheatsheet `?` key (not in vendor keyset).
  const cheatsheetHandler = (event: Event) => {
    const kb = event as KeyboardEvent;
    if (kb.key !== "?") return;
    if (kb.defaultPrevented || kb.repeat || kb.isComposing) return;
    const target = kb.target as HTMLElement | null;
    if (
      target?.isContentEditable ||
      target?.closest?.(
        'input, textarea, select, [contenteditable]:not([contenteditable="false"])',
      )
    )
      return;
    kb.preventDefault();
    opts.actions.toggleCheatsheet?.();
  };
  doc.addEventListener("keydown", cheatsheetHandler);

  let destroyed = false;
  return {
    destroy() {
      if (destroyed) return;
      destroyed = true;
      vendorHandle.destroy();
      doc.removeEventListener("keydown", cheatsheetHandler);
    },
  };
}