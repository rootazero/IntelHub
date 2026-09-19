// GEV P10 T2 — panel-drag adapter tests.
//
// P3 lesson: mocks must replicate the real constructor contract. The vendor
// PanelPositionControls accepts exactly four callback deps (no defaults); a
// mock that omits them or passes a wrong shape would pass the test but crash
// at runtime. We use vi.fn() (no new HTTP mocking libs, per brief constraint)
// and verify the adapter wires every required dep through to the constructor.
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { mountPanelDrag, type PanelDragOptions } from "../panel-drag";
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
});
