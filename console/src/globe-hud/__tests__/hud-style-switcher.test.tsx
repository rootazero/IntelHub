import "@testing-library/jest-dom/vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { HudStyleSwitcher, readPersistedStyle } from "../HudStyleSwitcher";
import type { VisualEffectsHandle } from "../../gev-visual/visual-effects";

function fakeHandle(): VisualEffectsHandle & { setStyle: ReturnType<typeof vi.fn> } {
  let style: any = "normal";
  return {
    setStyle: vi.fn((s: any) => { style = s; }),
    getStyle: () => style,
    // P9 extended the handle with getStages(); the switcher never reads it.
    getStages: () => null,
    destroy: vi.fn(),
  };
}

beforeEach(() => localStorage.clear());
afterEach(() => cleanup());

describe("HudStyleSwitcher", () => {
  test("renders nothing while handle is null (engine not started)", () => {
    const { container } = render(<HudStyleSwitcher handle={null} />);
    expect(container).toBeEmptyDOMElement();
  });

  test("shows current style label and lists all seven options on click", () => {
    render(<HudStyleSwitcher handle={fakeHandle()} />);
    const button = screen.getByTestId("hud-style-switcher");
    expect(button).toHaveTextContent("NORMAL");
    fireEvent.click(button);
    for (const [id, label] of [
      ["normal", "NORMAL"], ["retro", "CRT"], ["surveillance", "NVG"],
      ["thermal", "FLIR"], ["anime", "ANIME"], ["noir", "NOIR"], ["snow", "SNOW"],
    ]) {
      expect(screen.getByTestId(`hud-style-option-${id}`)).toHaveTextContent(label);
    }
  });

  test("selecting a style calls handle.setStyle and persists to localStorage", () => {
    const handle = fakeHandle();
    render(<HudStyleSwitcher handle={handle} />);
    fireEvent.click(screen.getByTestId("hud-style-switcher"));
    fireEvent.click(screen.getByTestId("hud-style-option-thermal"));
    expect(handle.setStyle).toHaveBeenCalledWith("thermal");
    expect(localStorage.getItem("intelhub.globe.style")).toBe("thermal");
    expect(screen.getByTestId("hud-style-switcher")).toHaveTextContent("FLIR");
  });

  test("readPersistedStyle falls back to normal on missing/invalid values", () => {
    expect(readPersistedStyle()).toBe("normal");
    localStorage.setItem("intelhub.globe.style", "not-a-style");
    expect(readPersistedStyle()).toBe("normal");
    localStorage.setItem("intelhub.globe.style", "noir");
    expect(readPersistedStyle()).toBe("noir");
  });

  test("menu closes after selection", () => {
    render(<HudStyleSwitcher handle={fakeHandle()} />);
    fireEvent.click(screen.getByTestId("hud-style-switcher"));
    fireEvent.click(screen.getByTestId("hud-style-option-snow"));
    expect(screen.queryByTestId("hud-style-option-snow")).not.toBeInTheDocument();
  });
});
