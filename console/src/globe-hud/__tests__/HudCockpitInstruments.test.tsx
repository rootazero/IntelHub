import "@testing-library/jest-dom/vitest";
import { cleanup, render, screen, act } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { HudCockpitInstruments } from "../HudCockpitInstruments";
import { mountCockpitInstruments } from "../../gev-visual/cockpit/instruments-mount";
import type { CockpitTrackedInfo } from "../../gev-visual/cockpit/instruments-mount";
import {
  DEFAULT_ELEMENT_VISIBILITY,
  type ElementVisibility,
} from "../../gev-visual/cockpit/element-visibility";

const allVisible: ElementVisibility = { ...DEFAULT_ELEMENT_VISIBILITY };
const allHidden: ElementVisibility = {
  compass: false,
  altimeter: false,
  speedRuler: false,
  altitudeLadder: false,
  speedTape: false,
  pitchLadder: false,
  bankIndicator: false,
  vsiChevron: false,
};

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
    const handle = mountCockpitInstruments({
      viewer: fakeViewer(),
      flights: { getTrackedInfo: () => tracked },
    } as any);
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
    const handle = mountCockpitInstruments({
      viewer: fakeViewer(),
      flights: { getTrackedInfo: () => null },
    } as any);
    render(<HudCockpitInstruments instruments={handle} />);
    expect(screen.getByText("000")).toBeInTheDocument(); // heading
    expect(screen.getByText("----- FT")).toBeInTheDocument(); // altitude
    expect(screen.getByText("--- KT")).toBeInTheDocument(); // speed
  });

  test("RAF loop re-reads the adapter at 4 Hz", () => {
    const handle = mountCockpitInstruments({
      viewer: fakeViewer(),
      flights: { getTrackedInfo: () => tracked },
    } as any);
    const spy = vi.spyOn(handle, "update");
    render(<HudCockpitInstruments instruments={handle} />);
    const initialCalls = spy.mock.calls.length; // immediate first read
    flush(0); // skipFirst frames the throttle baseline
    flush(300); // ≥250ms → one more update
    flush(600); // ≥250ms → another update
    expect(spy.mock.calls.length).toBe(initialCalls + 2);
  });

  // GEV P16 T5: 5 new SVG groups consume pitch/bank/vsi from the frame.
  // Baseline altitudes ≈1000m → 3281ft and speed 100 m/s → 194 kt.

  test("renders altitude ladder with 9 ticks when a frame is present", () => {
    const handle = mountCockpitInstruments({
      viewer: fakeViewer(),
      flights: { getTrackedInfo: () => tracked },
    } as any);
    render(<HudCockpitInstruments instruments={handle} />);
    expect(screen.getByTestId("altitude-ladder")).toBeInTheDocument();
    expect(screen.getAllByTestId("altitude-tick")).toHaveLength(9);
  });

  test("renders speed tape with 9 ticks when a frame is present", () => {
    const handle = mountCockpitInstruments({
      viewer: fakeViewer(),
      flights: { getTrackedInfo: () => tracked },
    } as any);
    render(<HudCockpitInstruments instruments={handle} />);
    expect(screen.getByTestId("speed-tape")).toBeInTheDocument();
    expect(screen.getAllByTestId("speed-tick")).toHaveLength(9);
  });

  test("pitch ladder applies bankRad rotation", () => {
    const handle = mountCockpitInstruments({
      viewer: fakeViewer(),
      flights: { getTrackedInfo: () => tracked },
      chaseCam: {
        getResolvedState: () => ({
          pitch: 0,
          bankRad: Math.PI / 4, // 45°
          vsiMps: 0,
        }),
      },
    } as any);
    render(<HudCockpitInstruments instruments={handle} />);
    const ladder = screen.getByTestId("pitch-ladder");
    const inner = ladder.querySelector("g");
    expect(inner?.getAttribute("transform")).toContain("rotate(45");
  });

  test("bank indicator renders 9 division marks", () => {
    const handle = mountCockpitInstruments({
      viewer: fakeViewer(),
      flights: { getTrackedInfo: () => tracked },
    } as any);
    render(<HudCockpitInstruments instruments={handle} />);
    expect(screen.getByTestId("bank-indicator")).toBeInTheDocument();
  });

  test("VSI chevron points up for positive vsiMps", () => {
    const handle = mountCockpitInstruments({
      viewer: fakeViewer(),
      flights: { getTrackedInfo: () => tracked },
      chaseCam: {
        getResolvedState: () => ({
          pitch: 0,
          bankRad: 0,
          vsiMps: 5.0,
        }),
      },
    } as any);
    render(<HudCockpitInstruments instruments={handle} />);
    expect(screen.getByTestId("vsi-chevron")).toBeInTheDocument();
    expect(screen.getByTestId("vsi-up")).toBeInTheDocument();
    expect(screen.queryByTestId("vsi-down")).not.toBeInTheDocument();
  });

  test("VSI chevron points down for negative vsiMps", () => {
    const handle = mountCockpitInstruments({
      viewer: fakeViewer(),
      flights: { getTrackedInfo: () => tracked },
      chaseCam: {
        getResolvedState: () => ({
          pitch: 0,
          bankRad: 0,
          vsiMps: -5.0,
        }),
      },
    } as any);
    render(<HudCockpitInstruments instruments={handle} />);
    expect(screen.getByTestId("vsi-down")).toBeInTheDocument();
    expect(screen.queryByTestId("vsi-up")).not.toBeInTheDocument();
  });

  test("neutral (vsi=0) VSI shows neither chevron", () => {
    const handle = mountCockpitInstruments({
      viewer: fakeViewer(),
      flights: { getTrackedInfo: () => tracked },
    } as any);
    render(<HudCockpitInstruments instruments={handle} />);
    expect(screen.queryByTestId("vsi-up")).not.toBeInTheDocument();
    expect(screen.queryByTestId("vsi-down")).not.toBeInTheDocument();
  });
});

// ── GEV P17: visibility map conditional rendering ────────────────────

describe("HudCockpitInstruments visibility (GEV P17)", () => {
  test("hides pitch-ladder when visibility.pitchLadder is false", () => {
    const handle = mountCockpitInstruments({
      viewer: fakeViewer(),
      flights: { getTrackedInfo: () => tracked },
    } as any);
    render(
      <HudCockpitInstruments
        instruments={handle}
        visibility={{ ...allVisible, pitchLadder: false }}
      />,
    );
    expect(screen.queryByTestId("pitch-ladder")).not.toBeInTheDocument();
    expect(screen.getByTestId("bank-indicator")).toBeInTheDocument();
  });

  test("hides compass when visibility.compass is false", () => {
    const handle = mountCockpitInstruments({
      viewer: fakeViewer(),
      flights: { getTrackedInfo: () => tracked },
    } as any);
    render(
      <HudCockpitInstruments
        instruments={handle}
        visibility={{ ...allVisible, compass: false }}
      />,
    );
    expect(screen.queryByTestId("hud-cockpit-compass")).not.toBeInTheDocument();
    expect(screen.getByTestId("hud-cockpit-altimeter")).toBeInTheDocument();
  });

  test("hides altimeter + speed-ruler when both visibility flags are false", () => {
    const handle = mountCockpitInstruments({
      viewer: fakeViewer(),
      flights: { getTrackedInfo: () => tracked },
    } as any);
    render(
      <HudCockpitInstruments
        instruments={handle}
        visibility={{
          ...allVisible,
          altimeter: false,
          speedRuler: false,
        }}
      />,
    );
    expect(screen.queryByTestId("hud-cockpit-altimeter")).not.toBeInTheDocument();
    expect(screen.queryByTestId("hud-cockpit-speed")).not.toBeInTheDocument();
    expect(screen.getByTestId("hud-cockpit-compass")).toBeInTheDocument();
  });

  test("all-hidden still renders the wrapper element", () => {
    const handle = mountCockpitInstruments({
      viewer: fakeViewer(),
      flights: { getTrackedInfo: () => tracked },
    } as any);
    render(
      <HudCockpitInstruments
        instruments={handle}
        visibility={allHidden}
      />,
    );
    expect(screen.getByTestId("hud-cockpit-instruments")).toBeInTheDocument();
    expect(screen.queryByTestId("hud-cockpit-compass")).not.toBeInTheDocument();
    expect(screen.queryByTestId("altitude-ladder")).not.toBeInTheDocument();
    expect(screen.queryByTestId("vsi-chevron")).not.toBeInTheDocument();
  });

  test("no visibility prop defaults to all-visible (backwards compat)", () => {
    const handle = mountCockpitInstruments({
      viewer: fakeViewer(),
      flights: { getTrackedInfo: () => tracked },
    } as any);
    render(<HudCockpitInstruments instruments={handle} />);
    expect(screen.getByTestId("hud-cockpit-compass")).toBeInTheDocument();
    expect(screen.getByTestId("pitch-ladder")).toBeInTheDocument();
    expect(screen.getByTestId("vsi-chevron")).toBeInTheDocument();
  });
});
