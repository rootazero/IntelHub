import "@testing-library/jest-dom/vitest";
import { act, cleanup, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test } from "vitest";
import { HudCockpitElementSwitch } from "../HudCockpitElementSwitch";
import { createCockpitStore } from "../../gev-visual/cockpit/cockpit-store";
import { DEFAULT_ELEMENT_VISIBILITY } from "../../gev-visual/cockpit/element-visibility";
import type { ElementVisibility } from "../../gev-visual/cockpit/element-visibility";

const allHidden: ElementVisibility = {
  compass: false,
  altimeter: false,
  speedRuler: false,
  altitudeLadder: false,
  speedTape: false,
  pitchLadder: false,
  bankIndicator: false,
  vsiChevron: false,
};

beforeEach(() => {
  localStorage.clear();
});
afterEach(() => {
  cleanup();
  localStorage.clear();
});

describe("HudCockpitElementSwitch", () => {
  test("renders the toggle button", () => {
    const store = createCockpitStore();
    render(<HudCockpitElementSwitch store={store} />);
    expect(
      screen.getByTestId("hud-cockpit-element-switch"),
    ).toBeInTheDocument();
  });

  test("does not render popover by default", () => {
    const store = createCockpitStore();
    render(<HudCockpitElementSwitch store={store} />);
    expect(screen.queryByRole("menu")).not.toBeInTheDocument();
  });

  test("opens popover with 8 checkboxes on click", () => {
    const store = createCockpitStore();
    render(<HudCockpitElementSwitch store={store} />);
    act(() => {
      screen.getByTestId("hud-cockpit-element-switch").click();
    });
    const menu = screen.getByRole("menu");
    expect(menu).toBeInTheDocument();
    const checkboxes = screen.getAllByRole("checkbox");
    expect(checkboxes).toHaveLength(8);
  });

  test("checkboxes reflect current visibility state", () => {
    const store = createCockpitStore();
    store.setElementVisibility({ speedTape: false });
    render(<HudCockpitElementSwitch store={store} />);
    act(() => {
      screen.getByTestId("hud-cockpit-element-switch").click();
    });
    const speedTapeCheckbox = screen.getByTestId(
      "hud-cockpit-element-speedTape",
    ) as HTMLInputElement;
    expect(speedTapeCheckbox.checked).toBe(false);
    const compassCheckbox = screen.getByTestId(
      "hud-cockpit-element-compass",
    ) as HTMLInputElement;
    expect(compassCheckbox.checked).toBe(true);
  });

  test("clicking a checkbox toggles visibility in the store", () => {
    const store = createCockpitStore();
    render(<HudCockpitElementSwitch store={store} />);
    act(() => {
      screen.getByTestId("hud-cockpit-element-switch").click();
    });
    const checkbox = screen.getByTestId(
      "hud-cockpit-element-pitchLadder",
    ) as HTMLInputElement;
    act(() => {
      checkbox.click();
    });
    expect(store.getState().elementVisibility.pitchLadder).toBe(false);
  });

  test("toggle persists to localStorage", () => {
    const store = createCockpitStore();
    render(<HudCockpitElementSwitch store={store} />);
    act(() => {
      screen.getByTestId("hud-cockpit-element-switch").click();
    });
    const checkbox = screen.getByTestId(
      "hud-cockpit-element-pitchLadder",
    ) as HTMLInputElement;
    act(() => {
      checkbox.click();
    });
    const stored = JSON.parse(
      localStorage.getItem("intelhub.cockpit.elementVisibility")!,
    );
    expect(stored.pitchLadder).toBe(false);
  });

  test("button shows hot class when any element is hidden", () => {
    const store = createCockpitStore();
    store.setElementVisibility({ speedTape: false });
    render(<HudCockpitElementSwitch store={store} />);
    const button = screen.getByTestId("hud-cockpit-element-switch");
    expect(button.className).toContain("hot");
  });

  test("button does NOT show hot class when all elements are visible", () => {
    const store = createCockpitStore();
    expect(store.getState().elementVisibility).toEqual(DEFAULT_ELEMENT_VISIBILITY);
    render(<HudCockpitElementSwitch store={store} />);
    const button = screen.getByTestId("hud-cockpit-element-switch");
    expect(button.className).not.toContain("hot");
  });

  test("button shows hot class even when ALL elements are hidden", () => {
    const store = createCockpitStore();
    store.setElementVisibility(allHidden);
    render(<HudCockpitElementSwitch store={store} />);
    const button = screen.getByTestId("hud-cockpit-element-switch");
    expect(button.className).toContain("hot");
  });

  test("clicking outside closes the popover", () => {
    const store = createCockpitStore();
    render(
      <div>
        <HudCockpitElementSwitch store={store} />
        <button data-testid="outside">outside</button>
      </div>,
    );
    act(() => {
      screen.getByTestId("hud-cockpit-element-switch").click();
    });
    expect(screen.queryByRole("menu")).toBeInTheDocument();
    const outside = screen.getByTestId("outside");
    act(() => {
      outside.dispatchEvent(
        new PointerEvent("pointerdown", { bubbles: true }),
      );
    });
    expect(screen.queryByRole("menu")).not.toBeInTheDocument();
  });

  test("clicking the toggle button twice closes the popover", () => {
    const store = createCockpitStore();
    render(<HudCockpitElementSwitch store={store} />);
    const button = screen.getByTestId("hud-cockpit-element-switch");
    act(() => {
      button.click();
    });
    expect(screen.queryByRole("menu")).toBeInTheDocument();
    act(() => {
      button.click();
    });
    expect(screen.queryByRole("menu")).not.toBeInTheDocument();
  });

  test("subscribes to store — updates checkboxes after external change", () => {
    const store = createCockpitStore();
    render(<HudCockpitElementSwitch store={store} />);
    act(() => {
      screen.getByTestId("hud-cockpit-element-switch").click();
    });
    // External change (e.g., from another window or a keyboard shortcut)
    act(() => {
      store.setElementVisibility({ compass: false });
    });
    const compassCheckbox = screen.getByTestId(
      "hud-cockpit-element-compass",
    ) as HTMLInputElement;
    expect(compassCheckbox.checked).toBe(false);
  });
});