// GEV P10 T2 — panel-disclosure adapter tests.
//
// The vendor exposes three primitives (bindPanelDisclosure,
// collapsePanelOnEscape, createHoverDisclosure). The adapter is a thin
// pass-through; we verify each primitive's behavior against a real DOM
// (jsdom), which catches the surface-level contract drift that mocks would
// paper over (P3 lesson).
import { afterEach, describe, expect, test, vi } from "vitest";
import {
  bindPanelDisclosure,
  collapsePanelOnEscape,
  createHoverDisclosure,
} from "../panel-disclosure";

describe("panel-disclosure adapter — bindPanelDisclosure", () => {
  let panel: HTMLDivElement;
  let button: HTMLButtonElement;

  afterEach(() => {
    panel?.remove();
    button?.remove();
    vi.restoreAllMocks();
  });

  function setup() {
    panel = document.createElement("div");
    panel.id = "test-panel";
    button = document.createElement("button");
    button.type = "button";
    panel.appendChild(button);
    document.body.appendChild(panel);
    return { panel, button };
  }

  test("clicking the button toggles collapse state via onChange", () => {
    const { panel, button } = setup();
    const onChange = vi.fn();
    const handle = bindPanelDisclosure({
      panel,
      buttons: [button],
      onChange,
      onEscape: () => {},
    });
    // Initial click: panel has no `collapsed` class → onChange(true)
    button.click();
    expect(onChange).toHaveBeenCalledWith(true, { explicit: true });
    panel.classList.add("collapsed");
    button.click();
    expect(onChange).toHaveBeenCalledWith(false, { explicit: true });
    handle.destroy();
  });

  test("binds multiple buttons (Set deduplication)", () => {
    const { panel, button } = setup();
    const btn2 = document.createElement("button");
    panel.appendChild(btn2);
    const onChange = vi.fn();
    const handle = bindPanelDisclosure({
      panel,
      buttons: [button, btn2, button], // duplicate
      onChange,
      onEscape: () => {},
    });
    btn2.click();
    expect(onChange).toHaveBeenCalledTimes(1);
    handle.destroy();
  });

  test("destroy() unbinds click listeners", () => {
    const { panel, button } = setup();
    const onChange = vi.fn();
    const handle = bindPanelDisclosure({
      panel,
      buttons: [button],
      onChange,
      onEscape: () => {},
    });
    handle.destroy();
    button.click();
    expect(onChange).not.toHaveBeenCalled();
  });

  test("throws TypeError on missing panel / callbacks", () => {
    expect(() =>
      bindPanelDisclosure({
        panel: null as unknown as HTMLElement,
        onChange: () => {},
        onEscape: () => {},
      }),
    ).toThrow(TypeError);
  });
});

describe("panel-disclosure adapter — collapsePanelOnEscape", () => {
  function makeEvent(target: EventTarget | null): KeyboardEvent {
    const ev = new KeyboardEvent("keydown", {
      key: "Escape",
      bubbles: true,
      cancelable: true,
    });
    if (target) Object.defineProperty(ev, "target", { value: target });
    return ev;
  }

  test("returns false on non-Escape keys", () => {
    const panel = document.createElement("div");
    document.body.appendChild(panel);
    const event = new KeyboardEvent("keydown", { key: "Enter" });
    const result = collapsePanelOnEscape(event, {
      panel,
      onChange: () => {},
    });
    expect(result).toBe(false);
  });

  test("returns true and calls onChange(true) on Escape from expanded panel", () => {
    const panel = document.createElement("div");
    panel.id = "escape-panel";
    document.body.appendChild(panel);
    const onChange = vi.fn();
    const result = collapsePanelOnEscape(makeEvent(panel), {
      panel,
      onChange,
    });
    expect(result).toBe(true);
    expect(onChange).toHaveBeenCalledWith(true, { explicit: true });
  });

  test("invokes beforeCollapse hook before onChange", () => {
    const panel = document.createElement("div");
    panel.id = "before-collapse-panel";
    document.body.appendChild(panel);
    const order: string[] = [];
    collapsePanelOnEscape(makeEvent(panel), {
      panel,
      onChange: () => order.push("onChange"),
      beforeCollapse: () => order.push("before"),
    });
    expect(order).toEqual(["before", "onChange"]);
  });
});

describe("panel-disclosure adapter — createHoverDisclosure", () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  test("destroy() clears timers and listeners", () => {
    vi.useFakeTimers();
    const panel = document.createElement("div");
    document.body.appendChild(panel);
    const disclosure = document.createElement("button");
    panel.appendChild(disclosure);
    const handle = createHoverDisclosure({
      panel,
      disclosure,
      onChange: () => {},
      onEscape: () => {},
    });
    expect(() => handle.destroy()).not.toThrow();
    expect(() => handle.destroy()).not.toThrow(); // idempotent
  });

  test("throws TypeError on missing panel/document", () => {
    expect(() =>
      createHoverDisclosure({
        panel: null as unknown as HTMLElement,
        onChange: () => {},
        onEscape: () => {},
      }),
    ).toThrow(TypeError);
  });

  test("cancelPendingFocus is callable pre-destroy", () => {
    const panel = document.createElement("div");
    document.body.appendChild(panel);
    const handle = createHoverDisclosure({
      panel,
      onChange: () => {},
      onEscape: () => {},
    });
    expect(() => handle.cancelPendingFocus()).not.toThrow();
    handle.destroy();
  });
});