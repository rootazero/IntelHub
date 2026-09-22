import "@testing-library/jest-dom/vitest";
import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
} from "@testing-library/react";
import { afterEach, describe, expect, test } from "vitest";
import { HudCockpitSvsSwitch } from "../HudCockpitSvsSwitch";
import { createCockpitStore } from "../../gev-visual/cockpit/cockpit-store";

describe("HudCockpitSvsSwitch (GEV P19)", () => {
  afterEach(() => {
    cleanup();
    localStorage.clear();
  });

  test("renders the toggle button", () => {
    const store = createCockpitStore();
    render(<HudCockpitSvsSwitch store={store} />);
    expect(
      screen.getByTestId("hud-cockpit-svs-switch"),
    ).toBeInTheDocument();
  });

  test("does not render popover by default", () => {
    const store = createCockpitStore();
    render(<HudCockpitSvsSwitch store={store} />);
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  test("opens popover on click", () => {
    const store = createCockpitStore();
    render(<HudCockpitSvsSwitch store={store} />);
    act(() => {
      screen.getByTestId("hud-cockpit-svs-switch").click();
    });
    expect(screen.getByRole("dialog")).toBeInTheDocument();
    expect(
      screen.getByTestId("hud-cockpit-svs-checkbox"),
    ).toBeInTheDocument();
  });

  test("checkbox dispatches setSvsEnabled", () => {
    const store = createCockpitStore();
    render(<HudCockpitSvsSwitch store={store} />);
    act(() => {
      screen.getByTestId("hud-cockpit-svs-switch").click();
    });
    const cb = screen.getByTestId(
      "hud-cockpit-svs-checkbox",
    ) as HTMLInputElement;
    act(() => {
      cb.click();
    });
    expect(store.getState().svsEnabled).toBe(true);
  });

  test("hydrates svsEnabled from localStorage on mount", () => {
    localStorage.setItem("intelhub.cockpit.svsEnabled", "1");
    const store = createCockpitStore();
    render(<HudCockpitSvsSwitch store={store} />);
    expect(store.getState().svsEnabled).toBe(true);
  });

  test("persists svsEnabled to localStorage on change", () => {
    const store = createCockpitStore();
    render(<HudCockpitSvsSwitch store={store} />);
    act(() => {
      store.setSvsEnabled(true);
    });
    expect(localStorage.getItem("intelhub.cockpit.svsEnabled")).toBe("1");
    act(() => {
      store.setSvsEnabled(false);
    });
    expect(localStorage.getItem("intelhub.cockpit.svsEnabled")).toBe("0");
  });

  test("toggle button gets hot class when svsEnabled", () => {
    const store = createCockpitStore();
    render(<HudCockpitSvsSwitch store={store} />);
    const btn = screen.getByTestId("hud-cockpit-svs-switch");
    expect(btn.className).not.toContain("hot");
    act(() => {
      store.setSvsEnabled(true);
    });
    expect(btn.className).toContain("hot");
  });

  test("clicking outside closes the popover", () => {
    const store = createCockpitStore();
    render(
      <div>
        <HudCockpitSvsSwitch store={store} />
        <button data-testid="outside">outside</button>
      </div>,
    );
    act(() => {
      screen.getByTestId("hud-cockpit-svs-switch").click();
    });
    expect(screen.queryByRole("dialog")).toBeInTheDocument();
    act(() => {
      fireEvent.pointerDown(screen.getByTestId("outside"));
    });
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });
});