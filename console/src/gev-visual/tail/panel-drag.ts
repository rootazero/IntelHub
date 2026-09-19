// GEV P10 T2 — panel-drag adapter.
//
// Vendor shape (verified at console/gev-engine/src/ui/panelPositionControls.js):
//   new PanelPositionControls({
//     syncPanelCollapseButton, // () => void  — vendor-owned disclosure refresh
//     layoutRightPanels,       // () => void  — vendor-owned right rail relayout
//     syncCctvPanelViewport,   // () => void  — vendor-owned CCTV panel sync
//     showToast,               // (msg: string) => void
//   })
//   .listen(target, type, callback)  — internal listener book-keeping
//   .destroy()                       — idempotent
//
// The vendor class is the engine-side composition root: it looks up the
// `pp-toggles` element by ID, owns its `godsEyeView.v8.panelPos.<id>` localStorage
// namespace, registers ResizeObserver for re-clamp on height changes, and
// promotes panels in a z-ladder [100, 139]. The IntelHub adapter must NOT
// re-implement any of that — it wraps the constructor with the four callback
// dependencies the vendor requires (all owned upstream by the composition
// layer; for jsdom/test we wire them to vi.fn() stubs).
//
// Brief adapter contract: `mount(panel, opts): Handle; destroy()`.
// The `panel` parameter is unused in the current vendor surface (vendor
// self-discovers `pp-toggles`); the adapter preserves it as a forward-looking
// seam so a future vendor refactor exposing per-panel injection has a
// no-ripple adapter surface. `opts` carries the four vendor callbacks plus
// optional `getPanelElement(id)` lookup (defaults to document.getElementById).
import { PanelPositionControls } from "gev-engine/src/ui/panelPositionControls.js";

export interface PanelDragDeps {
  /** Refresh a panel's disclosure chrome after collapse-state change. */
  syncPanelCollapseButton: (panel: HTMLElement) => void;
  /** Re-layout right-rail panels after a pp-toggles drag. */
  layoutRightPanels: () => void;
  /** Re-sync CCTV panel viewport after a cctv-panel drag. */
  syncCctvPanelViewport: () => void;
  /** Show a transient toast (e.g. layout reset notice). */
  showToast: (message: string) => void;
}

export interface PanelDragHandle {
  /** Vendor handle — useful for future extension; never null after mount. */
  readonly controls: InstanceType<typeof PanelPositionControls>;
  /** Idempotent — second call is a no-op (matches vendor semantics). */
  destroy(): void;
}

export interface PanelDragOptions extends PanelDragDeps {
  /** Lookup for `#<panelId>` resolution; default document.getElementById. */
  getPanelElement?: (id: string) => HTMLElement | null;
}

export function mountPanelDrag(
  panel: HTMLElement | null,
  opts: PanelDragOptions,
): PanelDragHandle {
  // The vendor class is idempotent — a fresh instance reads localStorage on
  // construction and binds drag listeners. Passing all four callback deps is
  // mandatory (no defaults in vendor); we surface them through `opts` instead
  // of letting the adapter hold them, so test fixtures can introspect them.
  const controls = new PanelPositionControls({
    syncPanelCollapseButton: opts.syncPanelCollapseButton,
    layoutRightPanels: opts.layoutRightPanels,
    syncCctvPanelViewport: opts.syncCctvPanelViewport,
    showToast: opts.showToast,
  });
  // `panel` is reserved for future per-panel injection. Vendor currently
  // self-discovers `pp-toggles` via getElementById, so no work here.
  void panel;
  let destroyed = false;
  return {
    controls,
    destroy() {
      if (destroyed) return;
      destroyed = true;
      controls.destroy();
    },
  };
}
