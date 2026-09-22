import "@testing-library/jest-dom/vitest";
import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
} from "@testing-library/react";
import { afterEach, describe, expect, test } from "vitest";
import { HudCockpitTcasSwitch } from "../HudCockpitTcasSwitch";
import { createCockpitStore } from "../../gev-visual/cockpit/cockpit-store";

describe("HudCockpitTcasSwitch (GEV P20)", () => {
  afterEach(() => {
    cleanup();
    localStorage.clear();
  });

  test("renders the toggle button", () => {
    const store = createCockpitStore();
    render(<HudCockpitTcasSwitch store={store} />);
    expect(
      screen.getByTestId("hud-cockpit-tcas-switch"),
    ).toBeInTheDocument();
  });

  test("does not render popover by default", () => {
    const store = createCockpitStore();
    render(<HudCockpitTcasSwitch store={store} />);
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  test("opens popover on click", () => {
    const store = createCockpitStore();
    render(<HudCockpitTcasSwitch store={store} />);
    act(() => {
      screen.getByTestId("hud-cockpit-tcas-switch").click();
    });
    expect(screen.getByRole("dialog")).toBeInTheDocument();
    expect(
      screen.getByTestId("hud-cockpit-tcas-checkbox"),
    ).toBeInTheDocument();
  });

  test("checkbox dispatches setTcasEnabled", () => {
    const store = createCockpitStore();
    render(<HudCockpitTcasSwitch store={store} />);
    act(() => {
      screen.getByTestId("hud-cockpit-tcas-switch").click();
    });
    const cb = screen.getByTestId(
      "hud-cockpit-tcas-checkbox",
    ) as HTMLInputElement;
    act(() => {
      cb.click();
    });
    expect(store.getState().tcasEnabled).toBe(true);
  });

  test("hydrates tcasEnabled from localStorage on mount", () => {
    localStorage.setItem("intelhub.cockpit.tcasEnabled", "1");
    const store = createCockpitStore();
    render(<HudCockpitTcasSwitch store={store} />);
    expect(store.getState().tcasEnabled).toBe(true);
  });

  test("persists tcasEnabled to localStorage on change", () => {
    const store = createCockpitStore();
    render(<HudCockpitTcasSwitch store={store} />);
    act(() => {
      store.setTcasEnabled(true);
    });
    expect(localStorage.getItem("intelhub.cockpit.tcasEnabled")).toBe("1");
    act(() => {
      store.setTcasEnabled(false);
    });
    expect(localStorage.getItem("intelhub.cockpit.tcasEnabled")).toBe("0");
  });

  test("toggle button gets hot class when tcasEnabled", () => {
    const store = createCockpitStore();
    render(<HudCockpitTcasSwitch store={store} />);
    const btn = screen.getByTestId("hud-cockpit-tcas-switch");
    expect(btn.className).not.toContain("hot");
    act(() => {
      store.setTcasEnabled(true);
    });
    expect(btn.className).toContain("hot");
  });

  test("clicking outside closes the popover", () => {
    const store = createCockpitStore();
    render(
      <div>
        <HudCockpitTcasSwitch store={store} />
        <button data-testid="outside">outside</button>
      </div>,
    );
    act(() => {
      screen.getByTestId("hud-cockpit-tcas-switch").click();
    });
    expect(screen.queryByRole("dialog")).toBeInTheDocument();
    act(() => {
      fireEvent.pointerDown(screen.getByTestId("outside"));
    });
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });
});