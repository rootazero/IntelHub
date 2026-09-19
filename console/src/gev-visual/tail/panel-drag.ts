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
  /**
   * Start a drag on the panel identified by `panelId`. Looks up the
   * `.hud-panel-drag-handle` inside `#<panelId>` and synthesizes the same
   * state machine the vendor runs for `#pp-toggles`:
   *   - promotes z-index (panel-draggable ladder)
   *   - pins panel to current rect, adds `.panel-dragging` class
   *   - registers pointermove/pointerup on window until endDrag()
   * Returns true when drag started, false when the panel/handle could not
   * be resolved (caller can no-op). Idempotent: a second startDrag() before
   * endDrag() cancels the prior one (matches vendor semantics).
   */
  startDrag(panelId: string, event: PointerEvent): boolean;
  /** Terminate any in-flight drag started by startDrag(). No-op if none. */
  endDrag(): void;
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

  // ── P11-A drag state machine ─────────────────────────────────────────
  // The vendor PanelPositionControls hardcodes its pointerdown listener to
  // the `.panel-drag-handle.compact` selector inside `#pp-toggles`. IntelHub
  // HUD panels use `.hud-panel-drag-handle` (HudPanelDragHandle) and live in
  // different containers, so they bypass the vendor's listener path. We
  // mirror the vendor's vendor-localStorage-and-clamp semantics for our
  // own handle so existing tests / save-state / z-promotion still behave
  // consistently across both surfaces.
  type DragState = {
    panelId: string;
    panelEl: HTMLElement;
    handleEl: HTMLElement;
    rect: DOMRect;
    offsetX: number;
    offsetY: number;
    onMove: (event: PointerEvent) => void;
    onUp: () => void;
  };
  let active: DragState | null = null;

  const storageKey = (panelId: string) =>
    `godsEyeView.v8.panelPos.${panelId}`;

  const clamp = (left: number, top: number, rect: DOMRect) => {
    const maxLeft = Math.max(6, window.innerWidth - rect.width - 6);
    const maxTop = Math.max(6, window.innerHeight - rect.height - 6);
    return {
      left: Math.max(6, Math.min(maxLeft, left)),
      top: Math.max(6, Math.min(maxTop, top)),
    };
  };

  const savePosition = (panelId: string, panelEl: HTMLElement) => {
    const rect = panelEl.getBoundingClientRect();
    try {
      localStorage.setItem(
        storageKey(panelId),
        JSON.stringify({
          left: Math.round(rect.left),
          top: Math.round(rect.top),
        }),
      );
    } catch {
      // storage unavailable — silently skip persistence (matches vendor).
    }
  };

  const finish = () => {
    if (!active) return;
    active.panelEl.classList.remove("panel-dragging");
    window.removeEventListener("pointermove", active.onMove);
    window.removeEventListener("pointerup", active.onUp);
    window.removeEventListener("pointercancel", active.onUp);
    savePosition(active.panelId, active.panelEl);
    active = null;
  };

  const start = (panelId: string, event: PointerEvent): boolean => {
    if (destroyed) return false;
    // A second startDrag() before endDrag() cancels the in-flight one
    // (vendor semantics; preserves single-source-of-truth for pointermove).
    if (active) finish();
    const panelEl =
      opts.getPanelElement?.(panelId) ??
      (typeof document !== "undefined"
        ? document.getElementById(panelId)
        : null);
    if (!panelEl) return false;
    const handleEl = panelEl.querySelector<HTMLElement>(
      ".hud-panel-drag-handle",
    );
    if (!handleEl) return false;
    const rect = panelEl.getBoundingClientRect();
    const offsetX = event.clientX - rect.left;
    const offsetY = event.clientY - rect.top;
    panelEl.style.left = `${rect.left}px`;
    panelEl.style.top = `${rect.top}px`;
    panelEl.style.right = "auto";
    panelEl.style.bottom = "auto";
    panelEl.classList.add("panel-dragging");
    // Mirror vendor's z-promotion ladder for the `panel-draggable` family.
    const promoted = [
      ...document.querySelectorAll<HTMLElement>(".panel-draggable"),
    ].filter((el) => el.style.zIndex);
    if (promoted.length > 0) {
      const sorted = promoted.sort(
        (a, b) => Number(a.style.zIndex) - Number(b.style.zIndex),
      );
      let z = 101;
      for (const el of sorted) {
        el.style.zIndex = String(z);
        z += 1;
      }
      panelEl.style.zIndex = String(z);
    } else {
      panelEl.style.zIndex = "139";
    }
    event.preventDefault();

    const onMove = (moveEvent: PointerEvent) => {
      const { left, top } = clamp(
        moveEvent.clientX - offsetX,
        moveEvent.clientY - offsetY,
        rect,
      );
      panelEl.style.left = `${left}px`;
      panelEl.style.top = `${top}px`;
    };
    const onUp = () => finish();
    active = { panelId, panelEl, handleEl, rect, offsetX, offsetY, onMove, onUp };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
    window.addEventListener("pointercancel", onUp);
    return true;
  };

  return {
    controls,
    startDrag: start,
    endDrag: finish,
    destroy() {
      if (destroyed) return;
      destroyed = true;
      finish();
      controls.destroy();
    },
  };
}
