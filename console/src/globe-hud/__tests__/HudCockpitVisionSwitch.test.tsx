import "@testing-library/jest-dom/vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import {
  HudCockpitVisionSwitch,
  VISION_LABELS,
  readPersistedVisionMode,
} from "../HudCockpitVisionSwitch";
import { VISION_MODES, type VisionMountHandle } from "../../gev-visual/cockpit/vision-mount";

function fakeVision(): VisionMountHandle {
  return {
    setMode: vi.fn((m: string) => m as never),
    getMode: () => "optical",
    destroy: vi.fn(),
  };
}

beforeEach(() => localStorage.clear());
afterEach(() => cleanup());

describe("HudCockpitVisionSwitch", () => {
  test("renders all five modes with verbatim testids", () => {
    render(
      <HudCockpitVisionSwitch
        vision={fakeVision()}
        mode="optical"
        onSelect={() => {}}
      />,
    );
    expect(screen.getByTestId("hud-cockpit-vision-switch")).toBeInTheDocument();
    for (const mode of VISION_MODES) {
      const button = screen.getByTestId(`hud-cockpit-vision-${mode}`);
      expect(button).toHaveTextContent(VISION_LABELS[mode]);
      expect(button).toHaveAttribute("aria-pressed", mode === "optical" ? "true" : "false");
    }
  });

  test("clicking a mode drives vision.setMode + onSelect + localStorage", () => {
    const vision = fakeVision();
    const onSelect = vi.fn();
    render(
      <HudCockpitVisionSwitch vision={vision} mode="optical" onSelect={onSelect} />,
    );
    fireEvent.click(screen.getByTestId("hud-cockpit-vision-crt"));
    expect(vision.setMode).toHaveBeenCalledWith("crt");
    expect(onSelect).toHaveBeenCalledWith("crt");
    expect(localStorage.getItem("intelhub.cockpit.visionMode")).toBe("crt");
  });

  test("a null vision handle still renders (setMode no-op)", () => {
    const onSelect = vi.fn();
    render(
      <HudCockpitVisionSwitch vision={null} mode="thermal" onSelect={onSelect} />,
    );
    fireEvent.click(screen.getByTestId("hud-cockpit-vision-noir"));
    expect(onSelect).toHaveBeenCalledWith("noir");
  });

  test("readPersistedVisionMode falls back to optical on missing/invalid", () => {
    expect(readPersistedVisionMode()).toBe("optical");
    localStorage.setItem("intelhub.cockpit.visionMode", "bogus");
    expect(readPersistedVisionMode()).toBe("optical");
    localStorage.setItem("intelhub.cockpit.visionMode", "nvg");
    expect(readPersistedVisionMode()).toBe("nvg");
  });
});
