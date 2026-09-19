import "@testing-library/jest-dom/vitest";
import { cleanup, render, screen, act } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { HudCockpitContext } from "../HudCockpitContext";
import type { CockpitTrackedInfo } from "../../gev-visual/cockpit/instruments-mount";

const tracked: CockpitTrackedInfo = {
  icao24: "abc123",
  callsign: "UAL123",
  altitudeM: 1000,
  velocityMps: 100,
  track: 90,
};

beforeEach(() => {
  vi.useFakeTimers();
});
afterEach(() => {
  vi.useRealTimers();
  cleanup();
});

describe("HudCockpitContext", () => {
  test("renders callsign / ICAO / altitude / heading / speed from tracked info", () => {
    render(<HudCockpitContext getTrackedInfo={() => tracked} />);
    expect(screen.getByTestId("hud-cockpit-context")).toBeInTheDocument();
    expect(screen.getByText("UAL123")).toBeInTheDocument();
    expect(screen.getByText("ICAO abc123")).toBeInTheDocument();
    expect(screen.getByText("1,000 M")).toBeInTheDocument();
    expect(screen.getByText("090°")).toBeInTheDocument();
    // 100 m/s × 1.94384 = 194.384 → "194 KT"
    expect(screen.getByText("194 KT")).toBeInTheDocument();
  });

  test("poll re-reads tracked info every 250ms", () => {
    let info = tracked;
    render(<HudCockpitContext getTrackedInfo={() => info} />);
    expect(screen.getByText("UAL123")).toBeInTheDocument();
    info = { ...tracked, callsign: "DAL99", altitudeM: 2000 };
    act(() => {
      vi.advanceTimersByTime(250);
    });
    expect(screen.getByText("DAL99")).toBeInTheDocument();
    expect(screen.getByText("2,000 M")).toBeInTheDocument();
  });

  test("neutral readout when getTrackedInfo returns null", () => {
    render(<HudCockpitContext getTrackedInfo={() => null} />);
    expect(screen.getByText("AIRCRAFT")).toBeInTheDocument();
    expect(screen.getByText("ICAO -----")).toBeInTheDocument();
    expect(screen.getByText("-----")).toBeInTheDocument(); // altitude
    expect(screen.getByText("---°")).toBeInTheDocument(); // heading
    expect(screen.getByText("--- KT")).toBeInTheDocument(); // speed
  });

  test("no interval is scheduled when getTrackedInfo is absent", () => {
    render(<HudCockpitContext />);
    expect(screen.getByTestId("hud-cockpit-context")).toBeInTheDocument();
    expect(screen.getByText("AIRCRAFT")).toBeInTheDocument();
  });
});
