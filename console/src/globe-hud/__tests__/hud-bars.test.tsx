// T11: HUD top bar (UTC clock / alerts / P5 search placeholder) + bottom
// status bar (globe collector health dots / layer + object counts / basemap
// label).
//   describe 1 — top bar: pure UTC formatter, 1 s tick, alert count, disabled
//     P5 search placeholder.
//   describe 2 — bottom bar: one dot per globe collector (adsb/celestrak/
//     usgs/opensky), state→tone mapping, «name links the monitor page (route "/")»,
//     enabled-layer readout driven by the dataManager (getAll/isEffectivelyEnabled/
//     subscribe — the T9 RailManager surface), 24 h event count, key-derived
//     basemap label, static P3 lon/lat placeholder.
//   describe 3 — useOverview: 15 s poll + cleanup + error lane (the page owns
//     ONE poll and hands the snapshot to both bars).
//   describe 4 — HudFrame mounts the bars in the data-hud top/bottom slots.
import { act, cleanup, render, renderHook, screen } from "@testing-library/react";
import { afterEach, describe, expect, test, vi } from "vitest";
import "@testing-library/jest-dom/vitest";

// The hook is the only consumer of the console API client — mock it so no
// test touches the network.
const apiMock = vi.hoisted(() => ({ api: vi.fn() }));
vi.mock("../../api", () => ({ api: apiMock.api }));

import {
  HudBottomBar,
  GLOBE_SOURCES,
  basemapStyleName,
  sourceTone,
} from "../HudBottomBar";
import type { BarManager } from "../HudBottomBar";
import { HudFrame } from "../HudFrame";
import { formatUtcClock, HudTopBar } from "../HudTopBar";
import { useOverview } from "../useOverview";
import type { OverviewData } from "../useOverview";

afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.clearAllMocks();
});

// ---- fixtures --------------------------------------------------------------

const SOURCES = [
  { name: "adsb", state: "ok", last_new: 812, ts: "2026-09-17T08:00:00Z" },
  { name: "celestrak", state: "ok", last_new: 440 },
  { name: "usgs", state: "error", last_new: 0, detail: "503 upstream" },
  // opensky intentionally absent → unknown lane.
];

const OVERVIEW: OverviewData = {
  alerts: { open: 7 },
  radar: {
    geo_events_24h: 1234,
    monitor: { up: true, sources_ok: 2, sources_total: 3, sources: SOURCES },
  },
};

// ---- mock dataManager (same structural surface as T9's RailManager) --------

function mockManager(ids: string[], on: string[] = []) {
  const listeners = new Set<
    (change: { type: string; layerId?: string }) => void
  >();
  const enabled = new Set(on);
  return {
    getAll: () => ids.map((id) => ({ id, name: id, showInTogglePanel: true })),
    isEffectivelyEnabled: (id: string) => enabled.has(id),
    subscribe: (
      callback: (change: { type: string; layerId?: string }) => void,
    ) => {
      listeners.add(callback);
      return () => {
        listeners.delete(callback);
      };
    },
    // test helpers
    settle: (id: string, next: boolean) => {
      if (next) enabled.add(id);
      else enabled.delete(id);
      for (const listener of listeners)
        listener({ type: "visibility", layerId: id });
    },
  };
}

// ---- describe 1: top bar ---------------------------------------------------

describe("HudTopBar", () => {
  test("formatUtcClock renders HH:MM:SS in UTC", () => {
    expect(formatUtcClock(new Date("2026-09-17T08:38:12Z"))).toBe("08:38:12");
    expect(formatUtcClock(new Date("2026-09-17T00:00:05Z"))).toBe("00:00:05");
    // Not local time: 23:00Z is 23:00 UTC regardless of the host zone.
    expect(formatUtcClock(new Date("2026-09-17T23:59:59Z"))).toBe("23:59:59");
  });

  test("UTC clock is present and ticks once per second", () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-09-17T08:38:12Z"));
    render(<HudTopBar overview={OVERVIEW} />);
    const clock = screen.getByTestId("hud-utc-clock");
    expect(clock).toHaveTextContent("08:38:12 UTC");
    act(() => vi.advanceTimersByTime(1000));
    expect(clock).toHaveTextContent("08:38:13 UTC");
    act(() => vi.advanceTimersByTime(2000));
    expect(clock).toHaveTextContent("08:38:15 UTC");
  });

  test("shows the open-alert count and links to /alerts", () => {
    render(<HudTopBar overview={OVERVIEW} />);
    expect(screen.getByTestId("hud-alert-count")).toHaveTextContent("7");
    expect(screen.getByTestId("hud-alert-bell")).toHaveAttribute(
      "href",
      "/alerts",
    );
  });

  test("clock ticks and counts degrade gracefully without an overview", () => {
    render(<HudTopBar overview={null} />);
    expect(screen.getByTestId("hud-alert-count")).toHaveTextContent("—");
    expect(screen.getByTestId("hud-utc-clock")).toHaveTextContent("UTC");
  });

  test("search is a disabled P5 placeholder (never a live input)", () => {
    render(<HudTopBar overview={OVERVIEW} />);
    const search = screen.getByTestId("hud-search-p5");
    expect(search).toBeDisabled();
    expect(search.getAttribute("aria-label")).toContain("P5");
    expect(search.getAttribute("placeholder")).toContain("P5");
  });
});

// ---- describe 2: bottom bar ------------------------------------------------

describe("sourceTone", () => {
  test("ok→ok, absent/unknown→warn, anything else→bad", () => {
    expect(sourceTone({ state: "ok" })).toBe("ok");
    expect(sourceTone({ state: "absent" })).toBe("warn");
    expect(sourceTone({ state: "unknown" })).toBe("warn");
    expect(sourceTone(undefined)).toBe("warn");
    expect(sourceTone({})).toBe("warn");
    expect(sourceTone({ state: "error" })).toBe("bad");
    expect(sourceTone({ state: "down" })).toBe("bad");
    expect(sourceTone({ state: "degraded" })).toBe("bad");
  });
});

describe("HudBottomBar", () => {
  test("renders exactly one health dot per globe collector", () => {
    render(<HudBottomBar overview={OVERVIEW} manager={null} />);
    for (const name of GLOBE_SOURCES)
      expect(screen.getByTestId(`hud-src-${name}`)).toBeInTheDocument();
    expect(document.querySelectorAll(".hud-src-dot")).toHaveLength(
      GLOBE_SOURCES.length,
    );
    expect(GLOBE_SOURCES).toEqual(["adsb", "celestrak", "usgs", "opensky"]);
  });

  test("dot tone follows the collector state (ok/warn/bad)", () => {
    render(<HudBottomBar overview={OVERVIEW} manager={null} />);
    expect(
      screen.getByTestId("hud-src-adsb").querySelector(".hud-src-dot"),
    ).toHaveClass("ok");
    expect(
      screen.getByTestId("hud-src-usgs").querySelector(".hud-src-dot"),
    ).toHaveClass("bad");
    // opensky has no health cell → unproven, not failed.
    expect(
      screen.getByTestId("hud-src-opensky").querySelector(".hud-src-dot"),
    ).toHaveClass("warn");
  });

  test("collector names link to the monitor page (route /)", () => {
    render(<HudBottomBar overview={OVERVIEW} manager={null} />);
    for (const name of GLOBE_SOURCES)
      expect(screen.getByTestId(`hud-src-${name}`)).toHaveAttribute(
        "href",
        "/",
      );
  });

  test("basemap label follows Google-key presence (photoreal vs esri)", () => {
    // Fallback lane (no engine handle): the GEV globe is Google photoreal
    // when keyed, keyless Esri otherwise — never the P1 2D radar's CARTO
    // DARK style. The live mapStack lane is covered in hud-live-lanes.test.ts.
    const { unmount } = render(
      <HudBottomBar overview={OVERVIEW} manager={null} googleKey="AIza-key" />,
    );
    expect(screen.getByTestId("hud-basemap-style")).toHaveTextContent(
      "GOOGLE PHOTOREAL",
    );
    unmount();
    render(<HudBottomBar overview={OVERVIEW} manager={null} googleKey="" />);
    expect(screen.getByTestId("hud-basemap-style")).toHaveTextContent(
      "ESRI IMAGERY",
    );
    // Pure resolver both lanes.
    expect(basemapStyleName("AIza-key")).toBe("GOOGLE PHOTOREAL");
    expect(basemapStyleName("")).toBe("ESRI IMAGERY");
    expect(basemapStyleName(undefined)).toBe("ESRI IMAGERY");
  });

  test("renders the cursor readout empty until the viewer handle exists", () => {
    render(<HudBottomBar overview={OVERVIEW} manager={null} viewer={null} />);
    const coords = screen.getByTestId("hud-coords-p3");
    expect(coords).toHaveTextContent("lon —");
    expect(coords).toHaveTextContent("lat —");
    expect(coords.getAttribute("aria-label")).toContain("实时");
    expect(coords.getAttribute("title")).toContain("pickEllipsoid");
    expect(screen.getByTestId("hud-cursor-lon")).toHaveTextContent("—");
    expect(screen.getByTestId("hud-cursor-lat")).toHaveTextContent("—");
  });

  test("shows the 24h object count", () => {
    render(<HudBottomBar overview={OVERVIEW} manager={null} />);
    expect(screen.getByTestId("hud-object-count")).toHaveTextContent("1,234");
  });

  test("enabled-layer readout follows the dataManager snapshot + events", () => {
    const manager = mockManager(
      ["flights", "military", "satellites", "earthquakes", "vessels", "radio"],
      ["flights", "satellites"],
    );
    render(
      <HudBottomBar overview={OVERVIEW} manager={manager as BarManager} />,
    );
    const counter = screen.getByTestId("hud-layer-count");
    expect(counter).toHaveTextContent("2/6");
    // Manager settles a new enable outside React → subscribe() re-renders.
    act(() => manager.settle("military", true));
    expect(counter).toHaveTextContent("3/6");
    act(() => manager.settle("flights", false));
    expect(counter).toHaveTextContent("2/6");
  });

  test("layers opted out of toggle panels are excluded from the readout", () => {
    const manager = {
      getAll: () => [
        { id: "flights", name: "flights", showInTogglePanel: true },
        { id: "military-awareness", showInTogglePanel: false, enabled: true },
      ],
      isEffectivelyEnabled: () => true,
      subscribe: () => () => {},
    };
    render(
      <HudBottomBar overview={OVERVIEW} manager={manager as BarManager} />,
    );
    expect(screen.getByTestId("hud-layer-count")).toHaveTextContent("1/1");
  });

  test("degrades to dashes + warn dots without an overview", () => {
    render(<HudBottomBar overview={null} manager={null} />);
    expect(screen.getByTestId("hud-object-count")).toHaveTextContent("—");
    expect(screen.getByTestId("hud-layer-count")).toHaveTextContent("—/—");
    expect(
      screen.getByTestId("hud-src-adsb").querySelector(".hud-src-dot"),
    ).toHaveClass("warn");
  });

  test("surfaces a failed overview poll", () => {
    render(<HudBottomBar overview={null} manager={null} error />);
    expect(screen.getByTestId("hud-overview-state")).toHaveTextContent("离线");
  });
});

// ---- describe 3: useOverview (single page-level poll) ----------------------

describe("useOverview", () => {
  test("fetches immediately, then every 15 s, and stops on unmount", async () => {
    vi.useFakeTimers();
    apiMock.api.mockResolvedValue(OVERVIEW);
    const { result, unmount } = renderHook(() => useOverview());
    expect(apiMock.api).toHaveBeenCalledWith("/api/v1/overview");
    await act(async () => {
      await Promise.resolve();
    });
    expect(result.current.overview).toEqual(OVERVIEW);
    expect(result.current.error).toBe(false);

    await act(async () => {
      vi.advanceTimersByTime(14_999);
    });
    expect(apiMock.api).toHaveBeenCalledTimes(1);
    await act(async () => {
      vi.advanceTimersByTime(1);
      await Promise.resolve();
    });
    expect(apiMock.api).toHaveBeenCalledTimes(2);

    unmount();
    await act(async () => {
      vi.advanceTimersByTime(60_000);
    });
    expect(apiMock.api).toHaveBeenCalledTimes(2);
  });

  test("a rejected poll latches the error lane and keeps polling", async () => {
    vi.useFakeTimers();
    apiMock.api.mockRejectedValue(new Error("network down"));
    const { result } = renderHook(() => useOverview());
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(result.current.overview).toBeNull();
    expect(result.current.error).toBe(true);
    apiMock.api.mockResolvedValue(OVERVIEW);
    await act(async () => {
      vi.advanceTimersByTime(15_000);
      await Promise.resolve();
    });
    expect(result.current.overview).toEqual(OVERVIEW);
    expect(result.current.error).toBe(false);
  });
});

// ---- describe 4: frame slots ----------------------------------------------

describe("HudFrame bar slots", () => {
  test("top/bottom props mount inside the data-hud slots", () => {
    render(
      <HudFrame
        top={<HudTopBar overview={OVERVIEW} />}
        bottom={<HudBottomBar overview={OVERVIEW} manager={null} />}
      />,
    );
    expect(
      document.querySelector('[data-hud="top"] > [data-testid="hud-top-bar"]'),
    ).toBeInTheDocument();
    expect(
      document.querySelector(
        '[data-hud="bottom"] > [data-testid="hud-bottom-bar"]',
      ),
    ).toBeInTheDocument();
  });
});
