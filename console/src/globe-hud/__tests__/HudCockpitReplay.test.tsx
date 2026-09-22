import "@testing-library/jest-dom/vitest";
import "fake-indexeddb/auto";
import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
} from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { HudCockpitReplay } from "../HudCockpitReplay";
import { createCockpitStore } from "../../gev-visual/cockpit/cockpit-store";
import { mountCockpitReplayRecorder } from "../../gev-visual/cockpit/replay-recorder";
import { mountCockpitReplayPlayer } from "../../gev-visual/cockpit/replay-player";

const sleep = (ms: number) =>
  new Promise<void>((r) => setTimeout(r, ms));

function deleteIdb(): Promise<void> {
  return new Promise((resolve) => {
    const req = indexedDB.deleteDatabase("intelhub-cockpit-replay");
    req.onsuccess = () => resolve();
    req.onerror = () => resolve();
    req.onblocked = () => resolve();
  });
}

describe("HudCockpitReplay", () => {
  beforeEach(async () => {
    await deleteIdb();
    localStorage.clear();
  });
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  test("renders the toggle button", () => {
    const store = createCockpitStore();
    render(
      <HudCockpitReplay store={store} recorder={null} player={null} />,
    );
    expect(
      screen.getByTestId("hud-cockpit-replay-switch"),
    ).toBeInTheDocument();
  });

  test("does not render popover by default", () => {
    const store = createCockpitStore();
    render(
      <HudCockpitReplay store={store} recorder={null} player={null} />,
    );
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  test("opens popover with Record / Play / Speed buttons on click", () => {
    const store = createCockpitStore();
    render(
      <HudCockpitReplay store={store} recorder={null} player={null} />,
    );
    act(() => {
      screen.getByTestId("hud-cockpit-replay-switch").click();
    });
    expect(screen.getByRole("dialog")).toBeInTheDocument();
    expect(
      screen.getByTestId("hud-cockpit-replay-record"),
    ).toBeInTheDocument();
    expect(
      screen.getByTestId("hud-cockpit-replay-play"),
    ).toBeInTheDocument();
    expect(
      screen.getByTestId("hud-cockpit-replay-speed-1"),
    ).toBeInTheDocument();
  });

  test("record button dispatches startRecording when no segment exists", () => {
    const store = createCockpitStore();
    const recorder = mountCockpitReplayRecorder({
      getFrame: () => ({
        heading: 0,
        pitchRad: 0,
        bankRad: 0,
        altitudeFt: 1000,
        speedKt: 100,
        vsiMps: 0,
        callsign: "T",
      }),
    });
    render(
      <HudCockpitReplay store={store} recorder={recorder} player={null} />,
    );
    act(() => {
      screen.getByTestId("hud-cockpit-replay-switch").click();
    });
    act(() => {
      screen.getByTestId("hud-cockpit-replay-record").click();
    });
    expect(store.getState().replayState.isRecording).toBe(true);
    recorder.destroy();
  });

  test("speed selector dispatches setPlaybackSpeed", () => {
    const store = createCockpitStore();
    render(
      <HudCockpitReplay store={store} recorder={null} player={null} />,
    );
    act(() => {
      screen.getByTestId("hud-cockpit-replay-switch").click();
    });
    act(() => {
      screen.getByTestId("hud-cockpit-replay-speed-2").click();
    });
    expect(store.getState().replayState.playbackSpeed).toBe(2);
  });

  test("subscribes to store — record button text reflects state", () => {
    const store = createCockpitStore();
    const recorder = mountCockpitReplayRecorder({
      getFrame: () => ({
        heading: 0,
        pitchRad: 0,
        bankRad: 0,
        altitudeFt: 1000,
        speedKt: 100,
        vsiMps: 0,
        callsign: "T",
      }),
    });
    render(
      <HudCockpitReplay store={store} recorder={recorder} player={null} />,
    );
    act(() => {
      screen.getByTestId("hud-cockpit-replay-switch").click();
    });
    const recordBtn = screen.getByTestId("hud-cockpit-replay-record");
    expect(recordBtn.textContent).toMatch(/record/i);
    act(() => {
      store.startRecording("seg-1");
    });
    expect(recordBtn.textContent).toMatch(/stop/i);
    recorder.destroy();
  });

  test("clicking outside closes the popover", () => {
    const store = createCockpitStore();
    render(
      <div>
        <HudCockpitReplay store={store} recorder={null} player={null} />
        <button data-testid="outside">outside</button>
      </div>,
    );
    act(() => {
      screen.getByTestId("hud-cockpit-replay-switch").click();
    });
    expect(screen.queryByRole("dialog")).toBeInTheDocument();
    act(() => {
      fireEvent.pointerDown(screen.getByTestId("outside"));
    });
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  test("empty segments list shows the placeholder", async () => {
    const store = createCockpitStore();
    const recorder = mountCockpitReplayRecorder({
      getFrame: () => null,
    });
    render(
      <HudCockpitReplay store={store} recorder={recorder} player={null} />,
    );
    act(() => {
      screen.getByTestId("hud-cockpit-replay-switch").click();
    });
    // Wait for the useEffect that loads segments
    await act(async () => {
      await sleep(50);
    });
    expect(
      screen.getByTestId("hud-cockpit-replay-empty"),
    ).toBeInTheDocument();
    recorder.destroy();
  });

  test("record + stop updates lastSavedSegmentId in store", async () => {
    const store = createCockpitStore();
    const recorder = mountCockpitReplayRecorder({
      getFrame: () => ({
        heading: 0,
        pitchRad: 0,
        bankRad: 0,
        altitudeFt: 1000,
        speedKt: 100,
        vsiMps: 0,
        callsign: "T",
      }),
    });
    render(
      <HudCockpitReplay store={store} recorder={recorder} player={null} />,
    );
    act(() => {
      screen.getByTestId("hud-cockpit-replay-switch").click();
    });
    act(() => {
      screen.getByTestId("hud-cockpit-replay-record").click();
    });
    await act(async () => {
      await sleep(150);
    });
    act(() => {
      screen.getByTestId("hud-cockpit-replay-record").click();
    });
    await act(async () => {
      await sleep(50);
    });
    expect(store.getState().replayState.lastSavedSegmentId).not.toBeNull();
    recorder.destroy();
  });

  test("hot class shows on recording or playing", () => {
    const store = createCockpitStore();
    const recorder = mountCockpitReplayRecorder({
      getFrame: () => null,
    });
    render(
      <HudCockpitReplay store={store} recorder={recorder} player={null} />,
    );
    const button = screen.getByTestId("hud-cockpit-replay-switch");
    expect(button.className).not.toContain("hot");
    act(() => {
      store.startRecording("seg-1");
    });
    expect(button.className).toContain("hot");
    recorder.destroy();
  });
});