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

// P12 T8 — manual navigation grace (5s) + fade + rotation progress bar.
// Mechanics: the 9s rotation interval is armed at fake-clock T0, so a tick
// lands every T0 + 9s. Clicking "next" at T0 + 4001 puts the following tick
// 4999ms later — just inside the 5000ms grace window — while a click at
// T0 + 4000 puts the boundary tick exactly at 5000ms (grace already expired).
describe("HudCockpitBriefingPanel briefing manual grace (P12)", () => {
  async function mountSummary() {
    const briefing = mountCockpitBriefing(router(okRoutes));
    const view = render(
      <HudCockpitBriefingPanel
        briefing={briefing}
        getTrackedInfo={getTracked}
        paused
      />,
    );
    await screen.findByText("TEMP");
    fireEvent.click(screen.getByTestId("hud-cockpit-tab-summary"));
    expect(await screen.findByText("first bullet")).toBeInTheDocument();
    // Arm the rotation under fake timers (the effect re-runs when `paused`
    // flips and creates a faked interval), mirroring the cadence test above.
    vi.useFakeTimers();
    view.rerender(
      <HudCockpitBriefingPanel briefing={briefing} getTrackedInfo={getTracked} />,
    );
    return { briefing, view };
  }

  const nextButton = () => screen.getByLabelText("下一页 / Next");
  const bullet = () => screen.getByTestId("hud-cockpit-summary-bullet");
  const bar = () => screen.getByTestId("hud-cockpit-progress-bar");

  test("manual nav arms a 5000ms grace window that drops an in-grace tick", async () => {
    const { briefing } = await mountSummary();
    const nextSpy = vi.spyOn(briefing, "next");

    act(() => {
      vi.advanceTimersByTime(4001);
    });
    fireEvent.click(nextButton());
    expect(nextSpy).toHaveBeenCalledTimes(1); // the manual click itself
    expect(screen.getByText("second bullet")).toBeInTheDocument();
    expect(bullet()).toHaveClass("fading"); // fade kicks in on the swap

    act(() => {
      vi.advanceTimersByTime(4999);
    }); // rotation tick at T0+9000, 4999ms after the manual nav
    expect(nextSpy).toHaveBeenCalledTimes(1); // dropped — inside the grace
    expect(bullet()).not.toHaveClass("fading"); // fade cleared after 200ms
  });

  test("during grace period auto-rotate is skipped (bullet + index hold)", async () => {
    const { briefing } = await mountSummary();

    act(() => {
      vi.advanceTimersByTime(4001);
    });
    fireEvent.click(nextButton());
    expect(briefing.index()).toBe(1);
    expect(screen.getByText("second bullet")).toBeInTheDocument();

    act(() => {
      vi.advanceTimersByTime(4999);
    });
    expect(briefing.index()).toBe(1); // not advanced
    expect(screen.getByText("second bullet")).toBeInTheDocument();
    expect(screen.getByText("2/2")).toBeInTheDocument();

    // The progress bar keeps moving during the grace (it reflects the cycle),
    // so it is not pinned at the start-of-cycle full width.
    expect(bar().style.width).not.toBe("100%");
  });

  test("grace expires after 5000ms — the boundary tick rotates again", async () => {
    const { briefing } = await mountSummary();
    const nextSpy = vi.spyOn(briefing, "next");

    act(() => {
      vi.advanceTimersByTime(4000);
    });
    fireEvent.click(nextButton());
    expect(nextSpy).toHaveBeenCalledTimes(1);
    act(() => {
      vi.advanceTimersByTime(5000);
    }); // T0+9000 === manualUntil: grace has expired
    expect(nextSpy).toHaveBeenCalledTimes(2);
    expect(screen.getByText("first bullet")).toBeInTheDocument();
    expect(screen.getByText("1/2")).toBeInTheDocument();
    // A rotation resets the cycle, so the bar is back at full width.
    expect(bar().style.width).toBe("100%");
  });
});
