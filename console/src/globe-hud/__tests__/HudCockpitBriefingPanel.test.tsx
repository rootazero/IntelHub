import "@testing-library/jest-dom/vitest";
import {
  cleanup,
  render,
  screen,
  fireEvent,
  act,
} from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { HudCockpitBriefingPanel } from "../HudCockpitBriefingPanel";
import { mountCockpitBriefing } from "../../gev-visual/cockpit/briefing-mount";
import type { ApiFetch } from "../../gev-adapters/http";
import type { CockpitTrackedInfo } from "../../gev-visual/cockpit/instruments-mount";

const weatherBody = {
  source: "noaa",
  fetched_at: "2026-09-18T00:00:00Z",
  temperature_c: 21.5,
  wind_speed_kts: 12.3,
  wind_direction_deg: 250,
  precipitation_mm: 0.4,
  visibility_m: 10000,
};
const summaryBody = {
  entity_id: "flight:UAL123",
  sources: ["cache"],
  bullets: [
    { text: "first bullet", source_url: "https://example.com/a", age_hours: 3 },
    { text: "second bullet", age_hours: 10 },
  ],
};

function router(
  routes: Array<{ match: string; status: number; body: unknown }>,
): ApiFetch {
  return vi.fn(async (path: string) => {
    const route = routes.find((r) => path.includes(r.match));
    if (!route) return new Response("not found", { status: 404 });
    return new Response(JSON.stringify(route.body), { status: route.status });
  });
}

const okRoutes = [
  { match: "/gev/weather", status: 200, body: weatherBody },
  { match: "/gev/summary", status: 200, body: summaryBody },
];

const tracked: CockpitTrackedInfo = {
  icao24: "abc123",
  callsign: "UAL123",
  latitude: 39.9,
  longitude: 116.4,
  altitudeM: 1000,
  velocityMps: 100,
  track: 90,
};
// Stable reference so the fetch effect does not re-fire on every re-render.
const getTracked = () => tracked;

beforeEach(() => localStorage.clear());
afterEach(() => {
  vi.useRealTimers();
  cleanup();
});

describe("HudCockpitBriefingPanel", () => {
  test("renders both tabs and the four weather metrics by default", async () => {
    render(
      <HudCockpitBriefingPanel
        briefing={mountCockpitBriefing(router(okRoutes))}
        getTrackedInfo={getTracked}
      />,
    );
    expect(screen.getByTestId("hud-cockpit-briefing")).toBeInTheDocument();
    expect(screen.getByTestId("hud-cockpit-tab-weather")).toBeInTheDocument();
    expect(screen.getByTestId("hud-cockpit-tab-summary")).toBeInTheDocument();
    expect(await screen.findByText("TEMP")).toBeInTheDocument();
    expect(screen.getByText("22°C")).toBeInTheDocument();
    expect(screen.getByText("12 KTS")).toBeInTheDocument();
    expect(screen.getByText("0.4 MM")).toBeInTheDocument();
    expect(screen.getByText("10 KM")).toBeInTheDocument();
  });

  test("summary tab shows bullets and manual nav advances", async () => {
    render(
      <HudCockpitBriefingPanel
        briefing={mountCockpitBriefing(router(okRoutes))}
        getTrackedInfo={getTracked}
      />,
    );
    await screen.findByText("TEMP");
    fireEvent.click(screen.getByTestId("hud-cockpit-tab-summary"));
    expect(await screen.findByText("first bullet")).toBeInTheDocument();
    fireEvent.click(screen.getByLabelText("下一页 / Next"));
    expect(screen.getByText("second bullet")).toBeInTheDocument();
    fireEvent.click(screen.getByLabelText("上一页 / Previous"));
    expect(screen.getByText("first bullet")).toBeInTheDocument();
  });

  test("hover pauses the rotation via onPause / onResume", async () => {
    const onPause = vi.fn();
    const onResume = vi.fn();
    render(
      <HudCockpitBriefingPanel
        briefing={mountCockpitBriefing(router(okRoutes))}
        getTrackedInfo={getTracked}
        onPause={onPause}
        onResume={onResume}
      />,
    );
    await screen.findByText("TEMP");
    const panel = screen.getByTestId("hud-cockpit-briefing");
    fireEvent.mouseEnter(panel);
    expect(onPause).toHaveBeenCalledTimes(1);
    fireEvent.mouseLeave(panel);
    expect(onResume).toHaveBeenCalledTimes(1);
  });

  test("auto-rotates summary bullets at the vendor cadence (9s)", async () => {
    const briefing = mountCockpitBriefing(router(okRoutes));
    const { rerender } = render(
      <HudCockpitBriefingPanel
        briefing={briefing}
        getTrackedInfo={getTracked}
        paused
      />,
    );
    await screen.findByText("TEMP");
    fireEvent.click(screen.getByTestId("hud-cockpit-tab-summary"));
    expect(screen.getByText("first bullet")).toBeInTheDocument();

    // Arm the rotation under fake timers by unpausing (the effect re-runs on
    // the paused prop flip and creates a faked interval).
    vi.useFakeTimers();
    rerender(
      <HudCockpitBriefingPanel briefing={briefing} getTrackedInfo={getTracked} />,
    );
    act(() => {
      vi.advanceTimersByTime(9000);
    });
    expect(screen.getByText("second bullet")).toBeInTheDocument();
  });

  test("weather 503 degrades to a grey banner without throwing (D3)", async () => {
    render(
      <HudCockpitBriefingPanel
        briefing={mountCockpitBriefing(
          router([
            { match: "/gev/weather", status: 503, body: { error: "down" } },
            { match: "/gev/summary", status: 200, body: summaryBody },
          ]),
        )}
        getTrackedInfo={getTracked}
      />,
    );
    expect(
      await screen.findByText("简报暂不可用 — 天气数据源离线"),
    ).toBeInTheDocument();
  });
});
