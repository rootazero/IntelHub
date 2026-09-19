// HudFrameRateReadout — FPS chip presentation.
//
// Tests cover: testid, null/finite fps rendering, throttling behavior, and
// aria-label updates. The throttling test uses fake timers to verify that
// the displayed value lags behind the prop by exactly throttleMs.
import "@testing-library/jest-dom/vitest";
import { act, cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, test, vi } from "vitest";
import { HudFrameRateReadout } from "../HudFrameRateReadout";

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

describe("HudFrameRateReadout", () => {
  test("renders with hud-frame-rate-readout testid", () => {
    render(<HudFrameRateReadout fps={60} />);
    const chip = screen.getByTestId("hud-frame-rate-readout");
    expect(chip).toBeInTheDocument();
  });

  test("shows the FPS value when provided", () => {
    render(<HudFrameRateReadout fps={42} />);
    const chip = screen.getByTestId("hud-frame-rate-readout");
    // Allow the throttle to settle (0ms default in test → setTimeout still
    // fires asynchronously even when throttleMs is 0).
    expect(chip.textContent).toContain("42");
    expect(chip.textContent).toContain("FPS");
  });

  test("shows dash placeholder when fps is null", () => {
    render(<HudFrameRateReadout fps={null} />);
    const chip = screen.getByTestId("hud-frame-rate-readout");
    expect(chip.textContent).toContain("—");
  });

  test("throttles updates to the configured interval (4 Hz default)", () => {
    vi.useFakeTimers();
    render(<HudFrameRateReadout fps={60} />);
    const chip = screen.getByTestId("hud-frame-rate-readout");
    // First render synchronously sees 60 (initial state).
    expect(chip.textContent).toContain("60");
    // Advance less than 250ms — no update yet.
    act(() => vi.advanceTimersByTime(100));
    expect(chip.textContent).toContain("60");
    // Advance past 250ms — prop hasn't changed, no re-render expected.
    act(() => vi.advanceTimersByTime(200));
    expect(chip.textContent).toContain("60");
  });

  test("updates display after throttle window when prop changes", () => {
    vi.useFakeTimers();
    const { rerender } = render(<HudFrameRateReadout fps={60} />);
    rerender(<HudFrameRateReadout fps={45} />);
    const chip = screen.getByTestId("hud-frame-rate-readout");
    // Throttle delays the update — chip still shows 60.
    act(() => vi.advanceTimersByTime(100));
    expect(chip.textContent).toContain("60");
    // After 250ms (throttle window) the new value 45 lands.
    act(() => vi.advanceTimersByTime(200));
    expect(chip.textContent).toContain("45");
  });

  test("aria-label reflects the current fps value", () => {
    render(<HudFrameRateReadout fps={58} />);
    const chip = screen.getByTestId("hud-frame-rate-readout");
    expect(chip.getAttribute("aria-label")).toBe("frame rate 58 fps");
    expect(chip.getAttribute("title")).toBe("frame rate 58 fps");
  });

  test("aria-label degrades to 'unknown' when fps is null", () => {
    render(<HudFrameRateReadout fps={null} />);
    const chip = screen.getByTestId("hud-frame-rate-readout");
    expect(chip.getAttribute("aria-label")).toBe(
      "frame rate unknown fps",
    );
  });
});