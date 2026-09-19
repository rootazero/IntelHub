// GEV P10 T2 — frame-rate-monitor adapter tests.
//
// Verifies the vendor pass-through wires correctly: title-bar lookup, viewer
// subscription, and clean destroy without leaking listeners.
import { afterEach, describe, expect, test, vi } from "vitest";
import { mountFrameRateMonitor } from "../frame-rate-monitor";

describe("frame-rate-monitor adapter", () => {
  afterEach(() => {
    document.body.innerHTML = "";
    vi.restoreAllMocks();
  });

  test("inert viewer + missing title-bar yields a valid handle", () => {
    const handle = mountFrameRateMonitor({ viewer: null });
    expect(typeof handle.destroy).toBe("function");
    handle.destroy();
  });

  test("mounts a readout child into #title-bar", () => {
    const titleBar = document.createElement("div");
    titleBar.id = "title-bar";
    document.body.appendChild(titleBar);
    const fakeViewer = {
      scene: {
        postRender: { addEventListener: () => () => {} },
      },
    };
    const handle = mountFrameRateMonitor({ viewer: fakeViewer });
    const readout = titleBar.querySelector(".frame-rate-readout");
    expect(readout).not.toBeNull();
    expect(readout?.textContent).toBe("FPS —");
    handle.destroy();
  });

  test("destroy() removes the readout element", () => {
    const titleBar = document.createElement("div");
    titleBar.id = "title-bar";
    document.body.appendChild(titleBar);
    const handle = mountFrameRateMonitor({
      viewer: { scene: { postRender: { addEventListener: () => () => {} } } },
    });
    expect(titleBar.querySelector(".frame-rate-readout")).not.toBeNull();
    handle.destroy();
    expect(titleBar.querySelector(".frame-rate-readout")).toBeNull();
  });

  test("destroy() is idempotent", () => {
    const handle = mountFrameRateMonitor({ viewer: null });
    handle.destroy();
    expect(() => handle.destroy()).not.toThrow();
  });

  test("no documentRef falls back to globalThis.document", () => {
    const titleBar = document.createElement("div");
    titleBar.id = "title-bar";
    document.body.appendChild(titleBar);
    const handle = mountFrameRateMonitor({
      viewer: { scene: { postRender: { addEventListener: () => () => {} } } },
    });
    handle.destroy();
  });

  test("viewer with no postRender event is treated as inert", () => {
    const titleBar = document.createElement("div");
    titleBar.id = "title-bar";
    document.body.appendChild(titleBar);
    // No postRender → vendor returns inert handle, no readout appended.
    const handle = mountFrameRateMonitor({ viewer: {} });
    expect(titleBar.querySelector(".frame-rate-readout")).toBeNull();
    handle.destroy();
  });

  test("backtick key toggles the readout visibility", () => {
    // Vendor subscribes to viewer.scene.postRender ONLY when the user toggles
    // the readout via the backtick key. The contract is "toggle on press";
    // we verify the readout is hidden → visible → hidden via three keydown
    // events without poking internals.
    const titleBar = document.createElement("div");
    titleBar.id = "title-bar";
    document.body.appendChild(titleBar);
    const handle = mountFrameRateMonitor({
      viewer: { scene: { postRender: { addEventListener: () => () => {} } } },
    });
    const readout = titleBar.querySelector(
      ".frame-rate-readout",
    ) as HTMLElement | null;
    expect(readout?.hidden).toBe(true);
    const press = new KeyboardEvent("keydown", { key: "`", bubbles: true });
    document.dispatchEvent(press);
    expect(
      (titleBar.querySelector(".frame-rate-readout") as HTMLElement | null)
        ?.hidden,
    ).toBe(false);
    document.dispatchEvent(press);
    expect(
      (titleBar.querySelector(".frame-rate-readout") as HTMLElement | null)
        ?.hidden,
    ).toBe(true);
    handle.destroy();
  });
});