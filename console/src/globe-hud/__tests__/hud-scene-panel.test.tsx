// HudScenePanel — scene director panel presentation.
//
// Tests cover the surface area: visibility class, scene list rendering,
// capture / share buttons, status line, and the close button.
import "@testing-library/jest-dom/vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, test, vi } from "vitest";
import { HudScenePanel } from "../HudScenePanel";
import type { HudScene } from "../HudScenePanel";

afterEach(() => cleanup());

const SCENES: HudScene[] = [
  {
    id: "s1",
    title: "First Scene",
    shots: [
      { id: "sh1", title: "Opening" },
      { id: "sh2", title: "Mid" },
    ],
  },
  {
    id: "s2",
    title: "Second Scene",
    shots: [],
  },
];

describe("HudScenePanel", () => {
  test("renders with hud-scene-panel testid when visible", () => {
    render(
      <HudScenePanel
        visible
        scenes={SCENES}
        selectedSceneId={null}
        selectedShotId={null}
      />,
    );
    const panel = screen.getByTestId("hud-scene-panel");
    expect(panel).toBeInTheDocument();
    expect(panel.className).not.toContain("hidden");
  });

  test("applies hidden class when not visible", () => {
    render(
      <HudScenePanel
        visible={false}
        scenes={SCENES}
        selectedSceneId={null}
        selectedShotId={null}
      />,
    );
    const panel = screen.getByTestId("hud-scene-panel");
    expect(panel.className).toContain("hidden");
    expect(panel.getAttribute("aria-hidden")).toBe("true");
  });

  test("renders one <option> per scene + capture + share buttons", () => {
    render(
      <HudScenePanel
        visible
        scenes={SCENES}
        selectedSceneId="s1"
        selectedShotId={null}
      />,
    );
    const select = screen.getByTestId("hud-scene-select");
    expect(select.querySelectorAll("option")).toHaveLength(2);
    expect(screen.getByTestId("hud-scene-capture-button")).toBeEnabled();
    expect(screen.getByTestId("hud-scene-share-link")).toBeEnabled();
  });

  test("capture button disabled when no scene selected", () => {
    render(
      <HudScenePanel
        visible
        scenes={SCENES}
        selectedSceneId={null}
        selectedShotId={null}
      />,
    );
    expect(screen.getByTestId("hud-scene-capture-button")).toBeDisabled();
    expect(screen.getByTestId("hud-scene-share-link")).toBeDisabled();
  });

  test("capture button disabled while capturing", () => {
    render(
      <HudScenePanel
        visible
        scenes={SCENES}
        selectedSceneId="s1"
        selectedShotId="sh1"
        capturing
      />,
    );
    expect(screen.getByTestId("hud-scene-capture-button")).toBeDisabled();
  });

  test("onCapture fires when capture button clicked", () => {
    const onCapture = vi.fn();
    render(
      <HudScenePanel
        visible
        scenes={SCENES}
        selectedSceneId="s1"
        selectedShotId={null}
        onCapture={onCapture}
      />,
    );
    fireEvent.click(screen.getByTestId("hud-scene-capture-button"));
    expect(onCapture).toHaveBeenCalledTimes(1);
  });

  test("renders selected shot rows with active state", () => {
    render(
      <HudScenePanel
        visible
        scenes={SCENES}
        selectedSceneId="s1"
        selectedShotId="sh1"
      />,
    );
    const list = screen.getByTestId("hud-scene-shot-list");
    const activeRow = list.querySelector(".hud-scene-shot-row.active");
    expect(activeRow).not.toBeNull();
    expect(activeRow?.textContent).toContain("Opening");
  });

  test("status line renders when provided", () => {
    render(
      <HudScenePanel
        visible
        scenes={SCENES}
        selectedSceneId="s1"
        selectedShotId="sh1"
        status={<span data-testid="status-text">RUNNING 75%</span>}
      />,
    );
    expect(screen.getByTestId("hud-scene-status")).toBeInTheDocument();
  });

  test("close button invokes onClose", () => {
    const onClose = vi.fn();
    render(
      <HudScenePanel
        visible
        scenes={SCENES}
        selectedSceneId="s1"
        selectedShotId={null}
        onClose={onClose}
      />,
    );
    fireEvent.click(screen.getByLabelText(/关闭场景面板/));
    expect(onClose).toHaveBeenCalledTimes(1);
  });
});