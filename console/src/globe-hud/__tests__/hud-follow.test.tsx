import "@testing-library/jest-dom/vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, test, vi } from "vitest";

// Mock the selection hook BEFORE importing the panel: drives kind/data per test.
const selection = { current: { kind: null as string | null, data: null as any } };
vi.mock("../../gev-boot/context-bridge", async (importOriginal) => {
  const orig = await importOriginal<any>();
  return { ...orig, useGlobeSelection: () => selection.current };
});

import { HudDetailPanel } from "../HudDetailPanel";
import type { FollowHandle } from "../../gev-visual/follow-controller";
import type { CameraOrientationHandle } from "../../gev-visual/camera-orientation";

function fakeFollow(): FollowHandle & { follow: ReturnType<typeof vi.fn> } {
  let tracked: string | null = null;
  return {
    follow: vi.fn((_k: string, id: string) => { tracked = id; return true; }),
    unfollow: vi.fn(() => { tracked = null; }),
    trackedId: () => tracked,
  } as any;
}

function fakeCamera(): CameraOrientationHandle & { toggleTilt: ReturnType<typeof vi.fn> } {
  return {
    toggleTilt: vi.fn(() => "down" as const),
    resetNorth: vi.fn(() => true),
    isTilted: vi.fn(() => false),
    destroy: vi.fn(),
  };
}

afterEach(() => { cleanup(); selection.current = { kind: null, data: null }; });

describe("HudDetailPanel follow buttons", () => {
  test("flight selection shows 跟随; click tracks with icao24", () => {
    selection.current = { kind: "flight", data: { id: "abc123", callsign: "TEST1" } };
    const follow = fakeFollow();
    render(<HudDetailPanel follow={follow} camera={fakeCamera()} />);
    fireEvent.click(screen.getByTestId("hud-follow-button"));
    expect(follow.follow).toHaveBeenCalledWith("flight", "abc123");
  });

  test("tracked flight shows 解除跟随 + 斜视 buttons; tilt delegates to camera", () => {
    selection.current = { kind: "flight", data: { id: "abc123" } };
    const follow = fakeFollow();
    const camera = fakeCamera();
    render(<HudDetailPanel follow={follow} camera={camera} />);
    fireEvent.click(screen.getByTestId("hud-follow-button")); // now tracked
    fireEvent.click(screen.getByTestId("hud-tilt-button"));
    expect(camera.toggleTilt).toHaveBeenCalled();
    fireEvent.click(screen.getByTestId("hud-unfollow-button"));
    expect(follow.unfollow).toHaveBeenCalled();
  });

  test("satellite selection follows with stringified noradId", () => {
    selection.current = { kind: "satellite", data: { noradId: "25544", name: "ISS" } };
    const follow = fakeFollow();
    render(<HudDetailPanel follow={follow} camera={fakeCamera()} />);
    fireEvent.click(screen.getByTestId("hud-follow-button"));
    expect(follow.follow).toHaveBeenCalledWith("satellite", "25544");
  });

  test("quake selection shows no follow button", () => {
    selection.current = { kind: "quake", data: { id: "us7000" } };
    render(<HudDetailPanel follow={fakeFollow()} camera={fakeCamera()} />);
    expect(screen.queryByTestId("hud-follow-button")).not.toBeInTheDocument();
  });

  test("follow failure surfaces 图层未启用 feedback", () => {
    selection.current = { kind: "flight", data: { id: "abc123" } };
    const follow = fakeFollow();
    follow.follow.mockReturnValue(false as any);
    render(<HudDetailPanel follow={follow} camera={fakeCamera()} />);
    fireEvent.click(screen.getByTestId("hud-follow-button"));
    expect(screen.getByTestId("hud-follow-button")).toHaveTextContent("图层未启用");
  });
});
