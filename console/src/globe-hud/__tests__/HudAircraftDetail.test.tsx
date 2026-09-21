// GEV P13 T3 — HudAircraftDetail tests.
//
// Two data paths are pinned here:
//   1. the REAL engine seam `getTrackedInfo()` (flights layer, queries.js:840)
//      — this is what production mounts (see GlobeV2.tsx);
//   2. the plan's `catalog.layers[flights].state` fixture shape — a QA-only
//      shape that the vendor layer object does NOT expose (the layer is built
//      by `Object.assign(layer, parts.*.methods)` in vendor index.js, and the
//      vendor engine is read-only), so it exists purely as a test/QA input.
import "@testing-library/jest-dom/vitest";
import type { ReactElement } from "react";
import { act, cleanup, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { I18nProvider } from "../../i18n";
import { HudAircraftDetail, type TrackedFlightInfo } from "../HudAircraftDetail";

/** Render inside the i18n provider so labels resolve to real English text. */
function renderPanel(node: ReactElement) {
  return render(<I18nProvider>{node}</I18nProvider>);
}

function catalogFixture(state: unknown) {
  return { layers: [{ id: "flights", state }] } as never;
}

function tracked(overrides: Partial<TrackedFlightInfo> = {}): TrackedFlightInfo {
  return {
    icao24: "780a5a",
    callsign: "CPA250",
    latitude: 22.3,
    longitude: 114.2,
    altitudeM: 10500,
    velocityMps: 230,
    track: 90,
    ...overrides,
  };
}

beforeEach(() => {
  document.body.classList.remove("cockpit-mode");
});

afterEach(() => {
  document.body.classList.remove("cockpit-mode");
  cleanup();
  vi.useRealTimers();
  vi.restoreAllMocks();
});

describe("HudAircraftDetail", () => {
  test("renders the empty state when nothing is tracked", () => {
    renderPanel(<HudAircraftDetail catalog={null} />);
    expect(screen.getByTestId("hud-aircraft-detail-empty")).toHaveTextContent(
      "Click a flight to inspect",
    );
    expect(
      screen.queryByTestId("hud-aircraft-detail"),
    ).not.toBeInTheDocument();
  });

  test("renders the evicted state when the tracked record is gone", () => {
    renderPanel(
      <HudAircraftDetail
        catalog={catalogFixture({
          _trackedIcao: "780a5a",
          records: { data: new Map() },
        })}
      />,
    );
    expect(screen.getByTestId("hud-aircraft-detail-empty")).toHaveTextContent(
      "Selection lost",
    );
  });

  test("shows the tracked hex + callsign from the catalog fixture path", () => {
    renderPanel(
      <HudAircraftDetail
        catalog={catalogFixture({
          _trackedIcao: "780a5a",
          records: {
            data: new Map([
              [
                "780a5a",
                {
                  hex: "780a5a",
                  callsign: "CPA250",
                  latitude: 22.3,
                  longitude: 114.2,
                  baroAltitudeM: 10500,
                  speedMps: 230,
                  courseDeg: 90,
                },
              ],
            ]),
          },
        })}
      />,
    );
    expect(screen.getByTestId("hud-aircraft-detail")).toBeInTheDocument();
    expect(screen.getByText("780A5A")).toBeInTheDocument();
    expect(screen.getByText("CPA250")).toBeInTheDocument();
  });

  test("shows type / registration / airline / route enrichment", () => {
    renderPanel(
      <HudAircraftDetail
        catalog={catalogFixture({
          _trackedIcao: "780a5a",
          records: {
            data: new Map([
              [
                "780a5a",
                {
                  hex: "780a5a",
                  typeCode: "A333",
                  typeName: "Airbus A330",
                  registration: "B-HWM",
                  airline: "Cathay Pacific",
                  route: {
                    origin: { code: "HKG" },
                    destination: { code: "JFK" },
                  },
                },
              ],
            ]),
          },
        })}
      />,
    );
    expect(screen.getByText("Airbus A330")).toBeInTheDocument();
    expect(screen.getByText("B-HWM")).toBeInTheDocument();
    expect(screen.getByText("Cathay Pacific")).toBeInTheDocument();
    expect(screen.getByText("HKG → JFK")).toBeInTheDocument();
  });

  test("prefers the live getTrackedInfo seam over the catalog fixture", () => {
    renderPanel(
      <HudAircraftDetail
        getTrackedInfo={() => tracked({ icao24: "a1b2c3", callsign: "SEAM01" })}
        catalog={catalogFixture({
          _trackedIcao: "780a5a",
          records: {
            data: new Map([["780a5a", { hex: "780a5a", callsign: "FIXTURE" }]]),
          },
        })}
      />,
    );
    expect(screen.getByText("A1B2C3")).toBeInTheDocument();
    expect(screen.getByText("SEAM01")).toBeInTheDocument();
    expect(screen.queryByText("FIXTURE")).not.toBeInTheDocument();
  });

  test("hides while the cockpit owns the panel (body class + prop)", () => {
    const seam = () => tracked();
    const first = renderPanel(<HudAircraftDetail getTrackedInfo={seam} />);
    expect(screen.getByTestId("hud-aircraft-detail")).toBeInTheDocument();
    first.unmount();

    document.body.classList.add("cockpit-mode");
    const { container } = renderPanel(
      <HudAircraftDetail getTrackedInfo={seam} />,
    );
    expect(container.firstChild).toBeNull();
    cleanup();
    document.body.classList.remove("cockpit-mode");

    const { container: byProp } = renderPanel(
      <HudAircraftDetail getTrackedInfo={seam} cockpitActive />,
    );
    expect(byProp.firstChild).toBeNull();
  });

  test("formats altitude in feet and speed in knots", () => {
    renderPanel(
      <HudAircraftDetail
        getTrackedInfo={() => tracked({ altitudeM: 10500, velocityMps: 230 })}
      />,
    );
    // 10 500 m × 3.280839895 = 34 448.8 → 34,449 ft
    expect(screen.getByText("34,449 ft")).toBeInTheDocument();
    // 230 m/s ÷ 0.514444 = 447.09 → 447 kt
    expect(screen.getByText("447 kt")).toBeInTheDocument();
  });

  test("formats latitude / longitude as 4 decimals", () => {
    renderPanel(
      <HudAircraftDetail
        getTrackedInfo={() => tracked({ latitude: 22.3, longitude: 114.2 })}
      />,
    );
    expect(screen.getByText("22.3000, 114.2000")).toBeInTheDocument();
  });

  test("shows an em-dash placeholder for every missing field", () => {
    renderPanel(
      <HudAircraftDetail
        getTrackedInfo={() =>
          tracked({
            callsign: null,
            latitude: null,
            longitude: null,
            altitudeM: null,
            velocityMps: null,
            track: null,
            typeName: null,
            registration: null,
            airline: null,
            route: null,
          })
        }
      />,
    );
    expect(screen.queryByText(/\d[\d,]* ft$/)).not.toBeInTheDocument();
    expect(screen.queryByText(/\d[\d,]* kt$/)).not.toBeInTheDocument();
    expect(screen.queryByText(/\.\d{4},/)).not.toBeInTheDocument();
    expect(screen.getAllByText("—").length).toBeGreaterThanOrEqual(5);
  });

  test("refreshes on a 1 s interval and clears it on unmount", () => {
    vi.useFakeTimers();
    const idleTimers = vi.getTimerCount();
    let current = tracked({ callsign: "AAA111" });
    const { unmount } = renderPanel(
      <HudAircraftDetail getTrackedInfo={() => current} />,
    );
    expect(screen.getByText("AAA111")).toBeInTheDocument();
    // Mount installed exactly one poll timer.
    expect(vi.getTimerCount()).toBe(idleTimers + 1);

    current = tracked({ callsign: "BBB222" });
    // No re-render before the tick lands.
    expect(screen.queryByText("BBB222")).not.toBeInTheDocument();

    act(() => {
      vi.advanceTimersByTime(1000);
    });
    expect(screen.getByText("BBB222")).toBeInTheDocument();

    unmount();
    expect(vi.getTimerCount()).toBe(idleTimers);
  });
});
