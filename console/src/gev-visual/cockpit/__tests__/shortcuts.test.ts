// GEV P12 T5 — cockpit keyboard shortcut tests (spec §6.E1).
//
// Driven through the REAL store (no mocked state machine) so `Escape` and
// `Space` assert the actual transition, not just that a spy fired.
import { cleanup, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { createCockpitStore } from "../cockpit-store";
import { VISION_MODE_STORAGE_KEY } from "../vision-mount";
import type { BriefingHandle } from "../briefing-mount";
import type { VisionMountHandle } from "../vision-mount";
import { useCockpitShortcuts } from "../shortcuts";

function press(key: string, init: KeyboardEventInit = {}, target?: Element) {
  const event = new KeyboardEvent("keydown", {
    key,
    bubbles: true,
    cancelable: true,
    ...init,
  });
  (target ?? window).dispatchEvent(event);
  return event;
}

interface Harness {
  store: ReturnType<typeof createCockpitStore>;
  briefingNext: ReturnType<typeof vi.fn>;
  briefingPrev: ReturnType<typeof vi.fn>;
  visionSetMode: ReturnType<typeof vi.fn>;
  onNextTab: ReturnType<typeof vi.fn>;
  onPrevTab: ReturnType<typeof vi.fn>;
  onToggleHidden: ReturnType<typeof vi.fn>;
}

function mount(active = true): { harness: Harness; unmount: () => void } {
  const store = createCockpitStore();
  if (active) store.enter("abc123");
  const briefingNext = vi.fn(() => 1);
  const briefingPrev = vi.fn(() => 0);
  const visionSetMode = vi.fn((mode: string) => mode);
  const onNextTab = vi.fn();
  const onPrevTab = vi.fn();
  const onToggleHidden = vi.fn();

  const briefing = {
    next: briefingNext,
    prev: briefingPrev,
  } as unknown as BriefingHandle;
  const vision = {
    setMode: visionSetMode,
  } as unknown as VisionMountHandle;

  const { unmount } = renderHook(() =>
    useCockpitShortcuts({
      store,
      briefing,
      vision,
      onNextTab,
      onPrevTab,
      onToggleHidden,
    }),
  );

  return {
    harness: {
      store,
      briefingNext,
      briefingPrev,
      visionSetMode,
      onNextTab,
      onPrevTab,
      onToggleHidden,
    },
    unmount,
  };
}

beforeEach(() => {
  localStorage.clear();
});

afterEach(() => {
  cleanup();
  document.body.innerHTML = "";
});

describe("useCockpitShortcuts", () => {
  test("ArrowLeft / ArrowRight drive the briefing rotation", () => {
    const { harness } = mount();
    press("ArrowRight");
    expect(harness.briefingNext).toHaveBeenCalledTimes(1);
    press("ArrowLeft");
    expect(harness.briefingPrev).toHaveBeenCalledTimes(1);
    expect(harness.briefingNext).toHaveBeenCalledTimes(1);
  });

  test("Escape exits the cockpit (real store transition, not just a spy)", () => {
    const { harness } = mount();
    expect(harness.store.getState().active).toBe(true);
    press("Escape");
    expect(harness.store.getState().active).toBe(false);
  });

  test("number keys 1-5 apply + record + persist the vision mode", () => {
    const { harness } = mount();
    const modes = ["optical", "crt", "nvg", "thermal", "noir"];
    modes.forEach((mode, i) => {
      press(String(i + 1));
      expect(harness.visionSetMode).toHaveBeenLastCalledWith(mode);
      expect(harness.store.getState().visionMode).toBe(mode);
      expect(localStorage.getItem(VISION_MODE_STORAGE_KEY)).toBe(mode);
    });
    expect(harness.visionSetMode).toHaveBeenCalledTimes(5);
    // Out-of-range digits are not vision picks.
    press("6");
    press("0");
    expect(harness.visionSetMode).toHaveBeenCalledTimes(5);
  });

  test("Space toggles the briefing pause and back", () => {
    const { harness } = mount();
    press(" ");
    expect(harness.store.getState().briefingPaused).toBe(true);
    press(" ");
    expect(harness.store.getState().briefingPaused).toBe(false);
  });

  test("Tab advances and Shift+Tab retreats the briefing tab", () => {
    const { harness } = mount();
    press("Tab");
    expect(harness.onNextTab).toHaveBeenCalledTimes(1);
    expect(harness.onPrevTab).not.toHaveBeenCalled();
    press("Tab", { shiftKey: true });
    expect(harness.onPrevTab).toHaveBeenCalledTimes(1);
  });

  test("Shift+C toggles the hidden state", () => {
    const { harness } = mount();
    press("C", { shiftKey: true });
    expect(harness.onToggleHidden).toHaveBeenCalledTimes(1);
    // Plain "c" is not a shortcut (no accidental hide while flying).
    press("c");
    expect(harness.onToggleHidden).toHaveBeenCalledTimes(1);
  });

  test("does nothing while the cockpit is inactive", () => {
    const { harness } = mount(false);
    press("ArrowRight");
    press("ArrowLeft");
    press("Escape");
    press("1");
    press(" ");
    press("Tab");
    press("C", { shiftKey: true });
    expect(harness.briefingNext).not.toHaveBeenCalled();
    expect(harness.briefingPrev).not.toHaveBeenCalled();
    expect(harness.visionSetMode).not.toHaveBeenCalled();
    expect(harness.onNextTab).not.toHaveBeenCalled();
    expect(harness.onPrevTab).not.toHaveBeenCalled();
    expect(harness.onToggleHidden).not.toHaveBeenCalled();
    expect(harness.store.getState().active).toBe(false);
  });

  test("ignores keys typed into an input", () => {
    const { harness } = mount();
    const input = document.createElement("input");
    document.body.appendChild(input);
    press("Escape", {}, input);
    press("ArrowRight", {}, input);
    press("1", {}, input);
    expect(harness.store.getState().active).toBe(true);
    expect(harness.briefingNext).not.toHaveBeenCalled();
    expect(harness.visionSetMode).not.toHaveBeenCalled();
  });

  test("ignores keys typed into a textarea or contentEditable host", () => {
    const { harness } = mount();
    const textarea = document.createElement("textarea");
    const editable = document.createElement("div");
    editable.setAttribute("contenteditable", "true");
    const child = document.createElement("span");
    editable.appendChild(child);
    document.body.append(textarea, editable);

    press("Escape", {}, textarea);
    press("Escape", {}, child); // nested event target → ancestor walk
    press("ArrowRight", {}, textarea);
    expect(harness.store.getState().active).toBe(true);
    expect(harness.briefingNext).not.toHaveBeenCalled();
  });

  test("ignores modifier chords (Ctrl / Meta / Alt)", () => {
    const { harness } = mount();
    press("Escape", { ctrlKey: true });
    press("Escape", { metaKey: true });
    press("Escape", { altKey: true });
    press("1", { ctrlKey: true });
    press("ArrowRight", { metaKey: true });
    expect(harness.store.getState().active).toBe(true);
    expect(harness.visionSetMode).not.toHaveBeenCalled();
    expect(harness.briefingNext).not.toHaveBeenCalled();
  });

  test("ignores auto-repeat from a held key", () => {
    const { harness } = mount();
    press("ArrowRight", { repeat: true });
    press(" ", { repeat: true });
    press("1", { repeat: true });
    expect(harness.briefingNext).not.toHaveBeenCalled();
    expect(harness.store.getState().briefingPaused).toBe(false);
    expect(harness.visionSetMode).not.toHaveBeenCalled();
  });

  test("removes the keydown listener on unmount", () => {
    const { harness, unmount } = mount();
    press("ArrowRight");
    expect(harness.briefingNext).toHaveBeenCalledTimes(1);
    unmount();
    press("ArrowRight");
    press("Escape");
    expect(harness.briefingNext).toHaveBeenCalledTimes(1);
    expect(harness.store.getState().active).toBe(true);
  });

  test("preventDefault()s handled keys only (never while inactive)", () => {
    const { unmount } = mount();
    expect(press("ArrowRight").defaultPrevented).toBe(true);
    expect(press("Escape").defaultPrevented).toBe(true);
    // Escape left the cockpit → the listener is now inert.
    expect(press("Escape").defaultPrevented).toBe(false);
    expect(press("Tab").defaultPrevented).toBe(false);
    unmount();
  });

  test("handles a null briefing / vision handle without throwing", () => {
    const store = createCockpitStore();
    store.enter("abc123");
    renderHook(() =>
      useCockpitShortcuts({ store, briefing: null, vision: null }),
    );
    expect(() => press("ArrowRight")).not.toThrow();
    press("2");
    expect(store.getState().visionMode).toBe("crt");
  });

  test("VISION_MODE_STORAGE_KEY matches the literal the switch panel reads", () => {
    expect(VISION_MODE_STORAGE_KEY).toBe("intelhub.cockpit.visionMode");
  });
});
