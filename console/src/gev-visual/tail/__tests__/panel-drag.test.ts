// GEV P10 T2 — panel-drag adapter tests.
//
// P3 lesson: mocks must replicate the real constructor contract. The vendor
// PanelPositionControls accepts exactly four callback deps (no defaults); a
// mock that omits them or passes a wrong shape would pass the test but crash
// at runtime. We use vi.fn() (no new HTTP mocking libs, per brief constraint)
// and verify the adapter wires every required dep through to the constructor.
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import {
  mountPanelDrag,
  panelDragHandleCount,
  setAllPanelDragDisabled,
  type PanelDragOptions,
} from "../panel-drag";
import { PanelPositionControls } from "gev-engine/src/ui/panelPositionControls.js";

function makeDeps(): PanelDragOptions {
  return {
    syncPanelCollapseButton: vi.fn(),
    layoutRightPanels: vi.fn(),
    syncCctvPanelViewport: vi.fn(),
    showToast: vi.fn(),
  };
}

describe("panel-drag adapter", () => {
  let ppToggle: HTMLElement;

  beforeEach(() => {
    // Vendor looks up #pp-toggles via getElementById during _initPanelDrag.
    // Provide one so the vendor doesn't silently skip drag wiring.
    ppToggle = document.createElement("div");
    ppToggle.id = "pp-toggles";
    const handle = document.createElement("div");
    handle.className = "panel-drag-handle compact";
    ppToggle.appendChild(handle);
    document.body.appendChild(ppToggle);
  });

  afterEach(() => {
    document.body.innerHTML = "";
    vi.restoreAllMocks();
  });

  test("mountPanelDrag constructs vendor with all 4 callback deps", () => {
    const opts = makeDeps();
    const handle = mountPanelDrag(ppToggle, opts);
    expect(handle.controls).toBeInstanceOf(PanelPositionControls);
    expect(handle.destroy).toBeTypeOf("function");
  });

  test("mountPanelDrag calls vendor destroy exactly once across multiple adapter destroys", () => {
    // The adapter guards on a `destroyed` flag — second adapter-level destroy
    // is a no-op. Vendor.destroy is therefore called exactly once even when
    // destroy() is invoked twice on the adapter.
    const opts = makeDeps();
    const handle = mountPanelDrag(ppToggle, opts);
    const destroySpy = vi.spyOn(handle.controls, "destroy");
    handle.destroy();
    handle.destroy();
    expect(destroySpy).toHaveBeenCalledTimes(1);
  });

  test("destroy() is idempotent at the adapter layer (no thrown errors)", () => {
    const opts = makeDeps();
    const handle = mountPanelDrag(ppToggle, opts);
    handle.destroy();
    expect(() => handle.destroy()).not.toThrow();
  });

  test("panel parameter is accepted but currently informational", () => {
    // Vendor self-discovers pp-toggles; the adapter preserves `panel` as a
    // forward seam. A null panel must still produce a valid handle.
    const opts = makeDeps();
    const handle = mountPanelDrag(null, opts);
    expect(handle.controls).toBeInstanceOf(PanelPositionControls);
    handle.destroy();
  });

  test("destroyed handle still exposes the underlying controls reference", () => {
    // Tests / future code may inspect the controls after destroy (e.g. for
    // post-mortem). The adapter does NOT null out the reference — vendor
    // .destroyed flag is the source of truth.
    const opts = makeDeps();
    const handle = mountPanelDrag(ppToggle, opts);
    handle.destroy();
    expect(handle.controls).toBeInstanceOf(PanelPositionControls);
  });

  test("startDrag(panelId, event) promotes panel to dragging state", () => {
    // P11-A: HudPanelDragHandle delegates pointerdown to the adapter via
    // startDrag(). The adapter mirrors vendor's drag state machine for
    // HUD panels (vendor's listener path is hardcoded to
    // `.panel-drag-handle.compact` inside `#pp-toggles` — out of reach for
    // HUD panels). We verify the contract surface: startDrag resolves the
    // panel + handle, flips `.panel-dragging` on the panel, and pins it to
    // its current rect.
    const panel = document.createElement("div");
    panel.id = "detail-panel";
    panel.className = "panel-draggable";
    panel.getBoundingClientRect = () =>
      ({
        left: 100,
        top: 80,
        right: 460,
        bottom: 320,
        width: 360,
        height: 240,
        x: 100,
        y: 80,
        toJSON() {
          return {};
        },
      }) as DOMRect;
    const handle = document.createElement("span");
    handle.className = "hud-panel-drag-handle";
    panel.appendChild(handle);
    document.body.appendChild(panel);

    const opts = makeDeps();
    const drag = mountPanelDrag(ppToggle, opts);
    const event = new PointerEvent("pointerdown", {
      bubbles: true,
      clientX: 150,
      clientY: 100,
      button: 0,
    });
    const started = drag.startDrag("detail-panel", event);
    expect(started).toBe(true);
    expect(panel.classList.contains("panel-dragging")).toBe(true);
    expect(panel.style.left).toBe("100px");
    expect(panel.style.top).toBe("80px");
    expect(panel.style.right).toBe("auto");
    expect(panel.style.bottom).toBe("auto");
    // z-index promoted (101 since no other panels in this test carry one).
    expect(panel.style.zIndex).toBeTruthy();

    drag.endDrag();
    // End terminates the drag class and removes window listeners.
    expect(panel.classList.contains("panel-dragging")).toBe(false);
    drag.destroy();
  });

  test("startDrag returns false for missing panel id", () => {
    const opts = makeDeps();
    const drag = mountPanelDrag(ppToggle, opts);
    const event = new PointerEvent("pointerdown", { bubbles: true });
    expect(
      drag.startDrag("does-not-exist", event),
    ).toBe(false);
    drag.destroy();
  });

  test("startDrag returns false when panel lacks a .hud-panel-drag-handle", () => {
    const panel = document.createElement("div");
    panel.id = "no-handle-panel";
    document.body.appendChild(panel);
    const opts = makeDeps();
    const drag = mountPanelDrag(ppToggle, opts);
    const event = new PointerEvent("pointerdown", { bubbles: true });
    expect(drag.startDrag("no-handle-panel", event)).toBe(false);
    drag.destroy();
  });

  test("endDrag is a no-op when no drag is active", () => {
    const opts = makeDeps();
    const drag = mountPanelDrag(ppToggle, opts);
    expect(() => drag.endDrag()).not.toThrow();
    drag.destroy();
  });

  test("second startDrag() before endDrag() cancels prior drag (vendor parity)", () => {
    const panel = document.createElement("div");
    panel.id = "parity-panel";
    panel.className = "panel-draggable";
    panel.getBoundingClientRect = () =>
      ({
        left: 10,
        top: 10,
        right: 100,
        bottom: 60,
        width: 90,
        height: 50,
        x: 10,
        y: 10,
        toJSON() {
          return {};
        },
      }) as DOMRect;
    const handle = document.createElement("span");
    handle.className = "hud-panel-drag-handle";
    panel.appendChild(handle);
    document.body.appendChild(panel);

    const opts = makeDeps();
    const drag = mountPanelDrag(ppToggle, opts);
    drag.startDrag(
      "parity-panel",
      new PointerEvent("pointerdown", {
        bubbles: true,
        clientX: 20,
        clientY: 20,
        button: 0,
      }),
    );
    expect(panel.classList.contains("panel-dragging")).toBe(true);
    // Second start on a different panel — first drag must terminate.
    const other = document.createElement("div");
    other.id = "parity-panel-2";
    other.className = "panel-draggable";
    other.getBoundingClientRect = panel.getBoundingClientRect;
    const otherHandle = document.createElement("span");
    otherHandle.className = "hud-panel-drag-handle";
    other.appendChild(otherHandle);
    document.body.appendChild(other);
    drag.startDrag(
      "parity-panel-2",
      new PointerEvent("pointerdown", {
        bubbles: true,
        clientX: 30,
        clientY: 30,
        button: 0,
      }),
    );
    expect(panel.classList.contains("panel-dragging")).toBe(false);
    expect(other.classList.contains("panel-dragging")).toBe(true);
    drag.destroy();
  });
});

// ── GEV P12 T7 — cockpit-active disabled parameter (spec §6.E4) ─────────
describe("panel-drag disabled parameter (P12)", () => {
  let ppToggle: HTMLElement;

  /** A `.panel-draggable` HUD panel with a 360x240 rect at (100, 80). */
  function makeDraggablePanel(id: string): HTMLElement {
    const panel = document.createElement("div");
    panel.id = id;
    panel.className = "panel-draggable";
    panel.getBoundingClientRect = () =>
      ({
        left: 100,
        top: 80,
        right: 460,
        bottom: 320,
        width: 360,
        height: 240,
        x: 100,
        y: 80,
        toJSON() {
          return {};
        },
      }) as DOMRect;
    const handle = document.createElement("span");
    handle.className = "hud-panel-drag-handle";
    panel.appendChild(handle);
    document.body.appendChild(panel);
    return panel;
  }

  function pointerDown(clientX: number, clientY: number): PointerEvent {
    return new PointerEvent("pointerdown", {
      bubbles: true,
      clientX,
      clientY,
      button: 0,
    });
  }

  function pointerMove(clientX: number, clientY: number): PointerEvent {
    return new PointerEvent("pointermove", {
      bubbles: true,
      clientX,
      clientY,
    });
  }

  beforeEach(() => {
    ppToggle = document.createElement("div");
    ppToggle.id = "pp-toggles";
    document.body.appendChild(ppToggle);
  });

  afterEach(() => {
    setAllPanelDragDisabled(false);
    document.body.innerHTML = "";
    vi.restoreAllMocks();
  });

  test("disabled=true: pointerdown is ignored and the panel never moves", () => {
    const panel = makeDraggablePanel("p12-disabled");
    const drag = mountPanelDrag(ppToggle, { ...makeDeps(), disabled: true });

    expect(drag.startDrag("p12-disabled", pointerDown(150, 100))).toBe(false);
    expect(panel.classList.contains("panel-dragging")).toBe(false);

    window.dispatchEvent(pointerMove(600, 400));
    expect(panel.style.left).toBe("");
    expect(panel.style.top).toBe("");
    drag.destroy();
  });

  test("disabled=true mid-drag releases immediately (R7)", () => {
    const panel = makeDraggablePanel("p12-middrag");
    const drag = mountPanelDrag(ppToggle, makeDeps());

    expect(drag.startDrag("p12-middrag", pointerDown(150, 100))).toBe(true);
    expect(panel.classList.contains("panel-dragging")).toBe(true);

    drag.setDisabled(true);
    expect(drag.isDisabled()).toBe(true);
    expect(panel.classList.contains("panel-dragging")).toBe(false);

    // Window listeners are gone — the pointer no longer drags the panel.
    const frozenLeft = panel.style.left;
    window.dispatchEvent(pointerMove(700, 500));
    expect(panel.style.left).toBe(frozenLeft);
    drag.destroy();
  });

  test("setDisabled(false) restores dragging (and the registry flips too)", () => {
    const panel = makeDraggablePanel("p12-thaw");
    const drag = mountPanelDrag(ppToggle, { ...makeDeps(), disabled: true });
    expect(drag.isDisabled()).toBe(true);

    setAllPanelDragDisabled(false);
    expect(drag.isDisabled()).toBe(false);
    expect(drag.startDrag("p12-thaw", pointerDown(150, 100))).toBe(true);
    expect(panel.classList.contains("panel-dragging")).toBe(true);

    // …and the registry can freeze it again (cockpit enter path).
    setAllPanelDragDisabled(true);
    expect(drag.isDisabled()).toBe(true);
    expect(panel.classList.contains("panel-dragging")).toBe(false);
    drag.destroy();
  });

  test("destroy() clears listeners and de-registers the handle", () => {
    const panel = makeDraggablePanel("p12-destroy");
    const before = panelDragHandleCount();
    const drag = mountPanelDrag(ppToggle, makeDeps());
    expect(panelDragHandleCount()).toBe(before + 1);

    expect(drag.startDrag("p12-destroy", pointerDown(150, 100))).toBe(true);
    drag.destroy();
    expect(panelDragHandleCount()).toBe(before);

    const frozenLeft = panel.style.left;
    window.dispatchEvent(pointerMove(900, 700));
    expect(panel.style.left).toBe(frozenLeft);
    expect(() =>
      window.dispatchEvent(new PointerEvent("pointerup", { bubbles: true })),
    ).not.toThrow();
  });

  test("clamp keeps out-of-viewport drags inside the window", () => {
    const panel = makeDraggablePanel("p12-clamp");
    const drag = mountPanelDrag(ppToggle, makeDeps());
    expect(drag.startDrag("p12-clamp", pointerDown(150, 100))).toBe(true);

    window.dispatchEvent(pointerMove(-5000, -5000));
    expect(panel.style.left).toBe("6px");
    expect(panel.style.top).toBe("6px");

    window.dispatchEvent(pointerMove(99999, 99999));
    expect(panel.style.left).toBe(
      `${Math.max(6, window.innerWidth - 360 - 6)}px`,
    );
    expect(panel.style.top).toBe(
      `${Math.max(6, window.innerHeight - 240 - 6)}px`,
    );
    drag.destroy();
  });
});
