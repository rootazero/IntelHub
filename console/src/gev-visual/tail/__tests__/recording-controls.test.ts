// GEV P10 T2 — recording-controls adapter tests.
//
// Verifies DOM injection (the adapter creates #safe-frame-overlay etc. when
// missing), HUD contract wiring, and setRecordingMode toggling the body
// class (sp8 check_44 acceptance hook).
import { afterEach, describe, expect, test, vi } from "vitest";
import {
  mountRecordingControls,
  type RecordingHud,
} from "../recording-controls";

function fakeHud(): RecordingHud {
  return {
    getMode: vi.fn(() => "auto"),
    getVariant: vi.fn(() => "tactical"),
    setMode: vi.fn(),
    setVariant: vi.fn(),
    visible: true,
  };
}

describe("recording-controls adapter — DOM injection", () => {
  afterEach(() => {
    document.body.innerHTML = "";
    vi.restoreAllMocks();
  });

  test("injects the four required DOM elements when missing", () => {
    const handle = mountRecordingControls({
      hud: fakeHud(),
      syncShareState: vi.fn(),
    });
    expect(document.getElementById("safe-frame-overlay")).not.toBeNull();
    expect(document.getElementById("safe-frame-box")).not.toBeNull();
    expect(document.getElementById("hud-toggle")).not.toBeNull();
    expect(document.getElementById("hud-layout-select")).not.toBeNull();
    handle.destroy();
  });

  test("reuses existing DOM elements when present", () => {
    const overlay = document.createElement("div");
    overlay.id = "safe-frame-overlay";
    document.body.appendChild(overlay);
    const hudToggle = document.createElement("button");
    hudToggle.id = "hud-toggle";
    document.body.appendChild(hudToggle);
    const handle = mountRecordingControls({
      hud: fakeHud(),
      syncShareState: vi.fn(),
      overlayElements: {
        overlay,
        hudToggle,
      },
    });
    // Same overlay reference reused (no second element appended)
    const allOverlays = document.querySelectorAll("#safe-frame-overlay");
    expect(allOverlays.length).toBe(1);
    expect(allOverlays[0]).toBe(overlay);
    handle.destroy();
  });

  test("destroy() removes injected elements (only those we created)", () => {
    // Pre-existing element should survive destroy.
    const preExisting = document.createElement("div");
    preExisting.id = "safe-frame-overlay";
    document.body.appendChild(preExisting);
    const handle = mountRecordingControls({
      hud: fakeHud(),
      syncShareState: vi.fn(),
      overlayElements: { overlay: preExisting },
    });
    handle.destroy();
    // Pre-existing overlay survives (consumer-owned)
    expect(document.getElementById("safe-frame-overlay")).toBe(preExisting);
    // Injected elements removed
    expect(document.getElementById("safe-frame-box")).toBeNull();
    expect(document.getElementById("hud-toggle")).toBeNull();
    expect(document.getElementById("hud-layout-select")).toBeNull();
  });
});

describe("recording-controls adapter — HUD contract wiring", () => {
  afterEach(() => {
    document.body.innerHTML = "";
    vi.restoreAllMocks();
  });

  test("setRecordingMode(true) calls hud.setMode + setVariant", () => {
    const hud = fakeHud();
    const handle = mountRecordingControls({
      hud,
      syncShareState: vi.fn(),
    });
    handle.setRecordingMode(true, { hudMode: "minimal", safeFrame: "16:9" });
    expect(hud.setMode).toHaveBeenCalled();
    expect(hud.setVariant).toHaveBeenCalled();
    handle.destroy();
  });

  test("setRecordingMode(true) toggles body.recording-mode class", () => {
    const handle = mountRecordingControls({
      hud: fakeHud(),
      syncShareState: vi.fn(),
    });
    handle.setRecordingMode(true);
    expect(document.body.classList.contains("recording-mode")).toBe(true);
    handle.setRecordingMode(false);
    expect(document.body.classList.contains("recording-mode")).toBe(false);
    handle.destroy();
  });

  test("safe-frame overlay class toggles with ratio", () => {
    const handle = mountRecordingControls({
      hud: fakeHud(),
      syncShareState: vi.fn(),
    });
    handle.setRecordingMode(true, { safeFrame: "9:16" });
    const overlay = document.getElementById("safe-frame-overlay");
    expect(overlay?.classList.contains("ratio-9-16")).toBe(true);
    handle.setRecordingMode(true, { safeFrame: "16:9" });
    expect(overlay?.classList.contains("ratio-16-9")).toBe(true);
    handle.destroy();
  });

  test("syncShareState is called on every setRecordingMode invocation", () => {
    const sync = vi.fn();
    const handle = mountRecordingControls({
      hud: fakeHud(),
      syncShareState: sync,
    });
    handle.setRecordingMode(true);
    expect(sync).toHaveBeenCalledTimes(1);
    handle.setRecordingMode(false);
    expect(sync).toHaveBeenCalledTimes(2);
    handle.destroy();
  });
});

describe("recording-controls adapter — destroy", () => {
  afterEach(() => {
    document.body.innerHTML = "";
    vi.restoreAllMocks();
  });

  test("destroy() is idempotent", () => {
    const handle = mountRecordingControls({
      hud: fakeHud(),
      syncShareState: vi.fn(),
    });
    handle.destroy();
    expect(() => handle.destroy()).not.toThrow();
  });

  test("destroy() restores HUD if recording was active", () => {
    const hud = fakeHud();
    const handle = mountRecordingControls({
      hud,
      syncShareState: vi.fn(),
    });
    handle.setRecordingMode(true);
    (hud.setMode as ReturnType<typeof vi.fn>).mockClear();
    (hud.setVariant as ReturnType<typeof vi.fn>).mockClear();
    handle.destroy();
    // Vendor's destroy calls setRecordingMode(false), which restores the
    // pre-recording HUD state.
    expect(hud.setMode).toHaveBeenCalled();
  });
});