// HudPanelDragHandle — visual drag handle presentation.
//
// The vendor PanelPositionControls (via mountPanelDrag adapter) owns the
// actual drag listener logic; this test only covers the presentational
// shape: testid, panelId attribute, click callback surface, and the
// collapsed-state class flip.
import "@testing-library/jest-dom/vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, test, vi } from "vitest";
import { HudPanelDragHandle } from "../HudPanelDragHandle";

afterEach(() => cleanup());

describe("HudPanelDragHandle", () => {
  test("renders with hud-panel-drag-handle testid and data-panel-id", () => {
    render(<HudPanelDragHandle panelId="cctv-panel" />);
    const handle = screen.getByTestId("hud-panel-drag-handle");
    expect(handle).toBeInTheDocument();
    expect(handle.dataset.panelId).toBe("cctv-panel");
  });

  test("fires onDragStart with the panel id on pointerdown", () => {
    const onDragStart = vi.fn();
    render(
      <HudPanelDragHandle
        panelId="detail-panel"
        onDragStart={onDragStart}
      />,
    );
    fireEvent.pointerDown(screen.getByTestId("hud-panel-drag-handle"));
    expect(onDragStart).toHaveBeenCalledTimes(1);
    expect(onDragStart.mock.calls[0][0]).toBe("detail-panel");
  });

  test("fires onDragEnd on pointerup", () => {
    const onDragEnd = vi.fn();
    render(
      <HudPanelDragHandle
        panelId="scene-panel"
        onDragEnd={onDragEnd}
      />,
    );
    fireEvent.pointerUp(screen.getByTestId("hud-panel-drag-handle"));
    expect(onDragEnd).toHaveBeenCalledTimes(1);
    expect(onDragEnd.mock.calls[0][0]).toBe("scene-panel");
  });

  test("collapsed prop adds the collapsed class", () => {
    render(<HudPanelDragHandle panelId="x" collapsed />);
    const handle = screen.getByTestId("hud-panel-drag-handle");
    expect(handle.className).toContain("collapsed");
  });

  test("custom glyph renders in the inner span", () => {
    render(<HudPanelDragHandle panelId="x" glyph="≡" />);
    expect(screen.getByTestId("hud-panel-drag-handle").textContent).toContain(
      "≡",
    );
  });
});