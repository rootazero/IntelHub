import "@testing-library/jest-dom/vitest";
import { cleanup, render, screen, act } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { HudCockpitInstruments } from "../HudCockpitInstruments";
import { mountCockpitInstruments } from "../../gev-visual/cockpit/instruments-mount";
import type { CockpitTrackedInfo } from "../../gev-visual/cockpit/instruments-mount";

const tracked: CockpitTrackedInfo = {
  icao24: "abc123",
  callsign: "UAL123",
  altitudeM: 1000,
  velocityMps: 100,
  track: 90,
};

function fakeViewer() {
  return { scene: { canvas: {} } };
}

// rAF control: the instruments loop throttles to one update per 250ms, so
// tests drive frames manually (mirrors hud-live-lanes.test.tsx).
let rafQueue: Array<(t: number) => void>;
let rafSeq: number;

beforeEach(() => {
  rafQueue = [];
  rafSeq = 0;
  vi.stubGlobal("requestAnimationFrame", (cb: (t: number) => void) => {
    rafQueue.push(cb);
    return ++rafSeq;
  });
  vi.stubGlobal("cancelAnimationFrame", (id: number) => {
    if (rafQueue[id - 1]) rafQueue[id - 1] = () => {};
  });
});
afterEach(() => {
  vi.unstubAllGlobals();
  cleanup();
});

function flush(t: number) {
  const cb = rafQueue.shift();
  if (cb) act(() => cb(t));
}

describe("HudCockpitInstruments", () => {
  test("renders nothing when the handle is null", () => {
    const { container } = render(<HudCockpitInstruments instruments={null} />);
    expect(container).toBeEmptyDOMElement();
  });

  test("renders the compass/altimeter/speed gauges with live data", () => {
    const handle = mountCockpitInstruments(fakeViewer() as any, {
      getTrackedInfo: () => tracked,
    });
    render(<HudCockpitInstruments instruments={handle} />);
    expect(screen.getByTestId("hud-cockpit-instruments")).toBeInTheDocument();
    expect(screen.getByTestId("hud-cockpit-compass")).toBeInTheDocument();
    expect(screen.getByTestId("hud-cockpit-altimeter")).toBeInTheDocument();
    expect(screen.getByTestId("hud-cockpit-speed")).toBeInTheDocument();
    // Immediate first read: heading 090, altitude 3,281 ft, speed 194 kt.
    expect(screen.getByText("090")).toBeInTheDocument();
    expect(screen.getByText("3,281 FT")).toBeInTheDocument();
    expect(screen.getByText("194 KT")).toBeInTheDocument();
  });

  test("neutral dashed gauges when nothing is tracked", () => {
    const handle = mountCockpitInstruments(fakeViewer() as any, {
      getTrackedInfo: () => null,
    });
    render(<HudCockpitInstruments instruments={handle} />);
    expect(screen.getByText("000")).toBeInTheDocument(); // heading
    expect(screen.getByText("----- FT")).toBeInTheDocument(); // altitude
    expect(screen.getByText("--- KT")).toBeInTheDocument(); // speed
  });

  test("RAF loop re-reads the adapter at 4 Hz", () => {
    const handle = mountCockpitInstruments(fakeViewer() as any, {
      getTrackedInfo: () => tracked,
    });
    const spy = vi.spyOn(handle, "update");
    render(<HudCockpitInstruments instruments={handle} />);
    const initialCalls = spy.mock.calls.length; // immediate first read
    flush(0); // skipFirst frames the throttle baseline
    flush(300); // ≥250ms → one more update
    flush(600); // ≥250ms → another update
    expect(spy.mock.calls.length).toBe(initialCalls + 2);
  });
});
