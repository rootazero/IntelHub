// HudRecordingControls — recording-mode toggle presentation.
//
// Tests cover: testids, active/inactive class state, button click toggle,
// mode-active badge visibility, and aria-pressed attribute.
import "@testing-library/jest-dom/vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, test, vi } from "vitest";
import { HudRecordingControls } from "../HudRecordingControls";

afterEach(() => cleanup());

describe("HudRecordingControls", () => {
  test("renders with hud-recording-toggle testid in inactive state", () => {
    render(<HudRecordingControls active={false} />);
    const toggle = screen.getByTestId("hud-recording-toggle");
    expect(toggle).toBeInTheDocument();
    expect(toggle.getAttribute("aria-pressed")).toBe("false");
    expect(toggle.className).not.toContain("active");
    // No mode-active badge when inactive.
    expect(
      screen.queryByTestId("hud-recording-mode-active"),
    ).not.toBeInTheDocument();
  });

  test("shows hud-recording-mode-active badge when active", () => {
    render(<HudRecordingControls active />);
    expect(screen.getByTestId("hud-recording-mode-active")).toBeInTheDocument();
    const toggle = screen.getByTestId("hud-recording-toggle");
    expect(toggle.getAttribute("aria-pressed")).toBe("true");
    expect(toggle.className).toContain("active");
  });

  test("click fires onToggle with the inverted active value", () => {
    const onToggle = vi.fn();
    render(<HudRecordingControls active={false} onToggle={onToggle} />);
    const toggle = screen.getByTestId("hud-recording-toggle");
    fireEvent.click(toggle);
    expect(onToggle).toHaveBeenCalledWith(true);
  });

  test("click on active toggle fires onToggle with false", () => {
    const onToggle = vi.fn();
    render(<HudRecordingControls active onToggle={onToggle} />);
    fireEvent.click(screen.getByTestId("hud-recording-toggle"));
    expect(onToggle).toHaveBeenCalledWith(false);
  });

  test("custom labels render when active/inactive", () => {
    const { rerender } = render(
      <HudRecordingControls
        active
        activeLabel="REC ON"
        inactiveLabel="REC OFF"
      />,
    );
    expect(
      screen.getByTestId("hud-recording-toggle").textContent,
    ).toContain("REC ON");
    rerender(
      <HudRecordingControls
        active={false}
        activeLabel="REC ON"
        inactiveLabel="REC OFF"
      />,
    );
    expect(
      screen.getByTestId("hud-recording-toggle").textContent,
    ).toContain("REC OFF");
  });

  test("container exposes data-active attribute for CSS hooks", () => {
    const { container, rerender } = render(
      <HudRecordingControls active={false} />,
    );
    expect(
      container.querySelector(".hud-recording-controls")?.getAttribute(
        "data-active",
      ),
    ).toBe("false");
    rerender(<HudRecordingControls active />);
    expect(
      container.querySelector(".hud-recording-controls")?.getAttribute(
        "data-active",
      ),
    ).toBe("true");
  });
});