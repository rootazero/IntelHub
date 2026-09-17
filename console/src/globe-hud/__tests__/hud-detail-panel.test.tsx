// T10: HUD detail panel (right edge) + selection bridge.
//   describe 1 — useGlobeSelection bridge against a mocked contextStore: the
//     vendor store has NO subscribe API — selection changes arrive as window
//     CustomEvents, and tracking layers (flights/satellites) fire their
//     awareness event BEFORE the store write, so the bridge must resync on a
//     microtask.
//   describe 2 — HudDetailPanel templates (flight / satellite / quake), empty
//     state, collapse handle, [进图谱] action link.
import { act, cleanup, fireEvent, render, renderHook, screen } from "@testing-library/react";
import { afterEach, describe, expect, test, vi } from "vitest";
import "@testing-library/jest-dom/vitest";

// The ONLY vendor API the bridge consumes (verified against
// console/gev-engine/src/data/contextStore.js — the store itself is a bare
// object on window with no subscribe; events are the change lane).
const harness = vi.hoisted(() => ({ record: null as unknown }));

vi.mock("gev-engine/src/data/contextStore.js", () => ({
  getSelectedEntityContext: () => harness.record,
}));

import { useGlobeSelection } from "../../gev-boot/context-bridge";
import { HudDetailPanel } from "../HudDetailPanel";

afterEach(() => {
  cleanup();
  harness.record = null;
});

// The bridge resyncs on a microtask after each selection event (vendor
// ordering quirk — see describe 1) — flush it inside act().
async function flushSelection() {
  await act(async () => {
    await Promise.resolve();
  });
}

function select(record: unknown, event = "gev:entity-selected") {
  harness.record = record;
  act(() => {
    window.dispatchEvent(new CustomEvent(event));
  });
  return flushSelection();
}

// Vendor-shaped records (console/gev-engine/src/layers/*/tracking.js):
// flat display STRINGS in properties — altitude "12,345 ft", speed "450 kt",
// heading "123°". The bridge normalizes to metric numbers for the panel.
const FLIGHT_RECORD = {
  id: "a1b2c3",
  layerId: "flights",
  layerName: "Live Flights",
  label: "CA1234",
  latitude: 31.2,
  longitude: 121.4,
  properties: {
    name: "CA1234",
    operator: "Air China",
    callsign: "CA1234",
    registration: "B-1234",
    type: "A320",
    altitude: "32,000 ft",
    speed: "450 kt",
    heading: "271°",
    route: "ZSPD → ZBAA",
    icao24: "a1b2c3",
    status: "live",
  },
};

const SATELLITE_RECORD = {
  id: "25544",
  layerId: "satellites",
  layerName: "Satellites",
  label: "ISS (ZARYA)",
  latitude: 1.2,
  longitude: 2.3,
  properties: {
    name: "ISS (ZARYA)",
    operator: "",
    noradId: "25544",
    class: "ISS",
    altitude: "420 km",
  },
};

const QUAKE_RECORD = {
  id: "us7000abcd",
  layerId: "earthquakes",
  layerName: "Earthquakes (24h)",
  label: "M5.2 新疆喀什",
  latitude: 39.5,
  longitude: 76,
  properties: { mag: 5.2, place: "新疆喀什", time: 1750000000000 },
};

// P3 vendor context shapes (grep-verified, 2026-09-17):
//   vessels/selection.js:195-203 — layerId 'ais-live-vessels', properties
//     mmsi/type/speedKt/course/destination, name via record.label.
const VESSEL_RECORD = {
  id: "ais-477123400",
  layerId: "ais-live-vessels",
  layerName: "Live AIS Vessels",
  label: "COSCO PACIFIC",
  latitude: 31.2,
  longitude: 122.8,
  properties: {
    mmsi: "477123400",
    type: "Container Ship",
    speedKt: 12.345,
    course: 271,
    destination: "Shanghai",
  },
};

//   installations/rendering.js:121-137 — layerId 'military-installations'
//     (policy.js:1), id 'osm:<type>:<id>', properties class/validation/
//     retrievedAt (ISO).
const INSTALLATION_RECORD = {
  id: "osm:way:123",
  layerId: "military-installations",
  layerName: "Mapped Military Installations",
  label: "Nellis AFB",
  latitude: 36.24,
  longitude: -115.03,
  properties: {
    class: "airfield",
    primaryType: null,
    placeTypes: [],
    validation: "unreviewed",
    retrievedAt: new Date(Date.now() - 3 * 3_600_000).toISOString(),
  },
};

//   cctv — NO vendor contextStore write exists (grep 0 hits under
//     layers/cctv/); record shape is forward-compat per contracts §3.
const CCTV_RECORD = {
  id: "cctv-1",
  layerId: "cctv",
  layerName: "CCTV",
  label: "QEW West of Thompson Road",
  latitude: 42.91,
  longitude: -78.96,
  properties: {
    city: "Ontario",
    provider: "RWIS (MTO)",
    feedType: "image",
    headingDeg: 90,
    fovDeg: 74,
    pitchDeg: -17,
    frameUrl: "/api/cctv/frame/cctv-1",
    live: true,
  },
};

// ---- describe 1: the bridge ------------------------------------------------

describe("useGlobeSelection", () => {
  test("returns kind:null with nothing selected", () => {
    const { result } = renderHook(() => useGlobeSelection());
    expect(result.current).toEqual({ kind: null, data: null });
  });

  test("click-selection event maps a flight record to kind 'flight' with metric data", async () => {
    const { result } = renderHook(() => useGlobeSelection());
    await select(FLIGHT_RECORD);
    expect(result.current.kind).toBe("flight");
    const data = result.current.data as {
      callsign: string;
      altitudeM: number;
      speedKmh: number;
      headingDeg: number;
    };
    // 32,000 ft → m; 450 kt → km/h; "271°" → 271.
    expect(data.callsign).toBe("CA1234");
    expect(data.altitudeM).toBe(Math.round(32000 * 0.3048));
    expect(data.speedKmh).toBe(Math.round(450 * 1.852));
    expect(data.headingDeg).toBe(271);
  });

  test("tracking lane: awareness event fires BEFORE the store write — still resolves", async () => {
    const { result } = renderHook(() => useGlobeSelection());
    // Vendor order in layers/flights/tracking.js::_publishTrackedSelection:
    // dispatch awareness event FIRST, then write the store.
    act(() => {
      window.dispatchEvent(new CustomEvent("gev:awareness-subject-selected"));
    });
    harness.record = FLIGHT_RECORD;
    await flushSelection();
    expect(result.current.kind).toBe("flight");
  });

  test("clear event returns to the empty selection", async () => {
    const { result } = renderHook(() => useGlobeSelection());
    await select(FLIGHT_RECORD);
    expect(result.current.kind).toBe("flight");
    harness.record = null;
    act(() => {
      window.dispatchEvent(
        new CustomEvent("gev:entity-selection-cleared"),
      );
    });
    await flushSelection();
    expect(result.current).toEqual({ kind: null, data: null });
  });

  test("unmapped layerIds report kind null", async () => {
    const { result } = renderHook(() => useGlobeSelection());
    await select({ ...FLIGHT_RECORD, layerId: "weather" });
    expect(result.current.kind).toBeNull();
  });
});

// ---- describe 2: the panel -------------------------------------------------

describe("HudDetailPanel", () => {
  test("empty state prompts the operator to click a target", () => {
    render(<HudDetailPanel />);
    expect(screen.getByTestId("hud-detail-panel")).toBeInTheDocument();
    expect(screen.getByText("点击地球上的目标查看详情")).toBeInTheDocument();
  });

  test("flight template: callsign, altitude m, speed km/h, heading, graph link", async () => {
    render(<HudDetailPanel />);
    await select(FLIGHT_RECORD);
    expect(screen.getByText("CA1234")).toBeInTheDocument();
    expect(
      screen.getByText(`${Math.round(32000 * 0.3048).toLocaleString("en-US")} m`),
    ).toBeInTheDocument();
    expect(
      screen.getByText(`${Math.round(450 * 1.852).toLocaleString("en-US")} km/h`),
    ).toBeInTheDocument();
    expect(screen.getByText("271°")).toBeInTheDocument();
    const link = screen.getByRole("link", { name: "进图谱" });
    expect(link).toHaveAttribute("href", "/graph?q=CA1234");
  });

  test("satellite template: name, NORAD, group", async () => {
    render(<HudDetailPanel />);
    await select(SATELLITE_RECORD);
    expect(screen.getByText("ISS (ZARYA)")).toBeInTheDocument();
    expect(screen.getByText("25544")).toBeInTheDocument();
    expect(screen.getByText("ISS")).toBeInTheDocument();
  });

  test("quake template: magnitude, place, relative time", async () => {
    render(<HudDetailPanel />);
    await select(QUAKE_RECORD);
    expect(screen.getByText("M5.2")).toBeInTheDocument();
    expect(screen.getByText("新疆喀什")).toBeInTheDocument();
    // 1750000000000 is far in the past relative to now → absolute date, not
    // a minutes-ago string.
    expect(screen.getByText(/2025/)).toBeInTheDocument();
  });

  test("vessel template: name, mmsi, kn speed, course, destination", async () => {
    render(<HudDetailPanel />);
    await select(VESSEL_RECORD);
    expect(screen.getByText("COSCO PACIFIC")).toBeInTheDocument();
    expect(screen.getByText("477123400")).toBeInTheDocument();
    // speedKt passes through as knots (12.345 → "12.3 kn").
    expect(screen.getByText("12.3 kn")).toBeInTheDocument();
    expect(screen.getByText("271°")).toBeInTheDocument();
    expect(screen.getByText("Shanghai")).toBeInTheDocument();
    // Vendor context does not publish imo/heading/observedAt → honest dashes.
    expect(screen.getByText("船舶 VESSEL")).toBeInTheDocument();
  });

  test("vessel bridge converts a full VesselRecord speedMps to knots", async () => {
    const { result } = renderHook(() => useGlobeSelection());
    await select({
      ...VESSEL_RECORD,
      properties: { ...VESSEL_RECORD.properties, speedKt: undefined, speedMps: 5.14444 },
    });
    expect(result.current.kind).toBe("vessel");
    // 5.14444 m/s ÷ 0.514444 = 10 kn exactly.
    expect((result.current.data as { speedKn: number }).speedKn).toBeCloseTo(
      10,
      3,
    );
  });

  test("installation template: name, class label, osm ref, relative retrieval", async () => {
    render(<HudDetailPanel />);
    await select(INSTALLATION_RECORD);
    expect(screen.getByText("Nellis AFB")).toBeInTheDocument();
    expect(screen.getByText("军用机场 Airfield")).toBeInTheDocument();
    expect(screen.getByText("way / 123")).toBeInTheDocument();
    expect(screen.getByText("unreviewed")).toBeInTheDocument();
    // retrievedAt 3h ago → relative time (the bridge parses the ISO string).
    expect(screen.getByText("3 小时前")).toBeInTheDocument();
  });

  test("cctv template: metadata rows + LIVE link, never an embedded media element", async () => {
    render(<HudDetailPanel />);
    await select(CCTV_RECORD);
    expect(screen.getByText("QEW West of Thompson Road")).toBeInTheDocument();
    expect(screen.getByText("Ontario")).toBeInTheDocument();
    expect(screen.getByText("RWIS (MTO)")).toBeInTheDocument();
    expect(screen.getByText("image")).toBeInTheDocument();
    expect(screen.getByText("90° / 74° / -17°")).toBeInTheDocument();
    expect(screen.getByText("在线")).toBeInTheDocument();
    const live = screen.getByRole("link", { name: /实时画面/ });
    expect(live).toHaveAttribute("href", "/api/cctv/frame/cctv-1");
    expect(live).toHaveAttribute("target", "_blank");
    // GPU texture rendering is the engine pipeline's job — the HUD must not
    // embed img/video DOM.
    expect(document.querySelector(".hud-detail img, .hud-detail video")).toBeNull();
  });

  test("cctv template without a feed renders the disabled entry", async () => {
    render(<HudDetailPanel />);
    await select({
      ...CCTV_RECORD,
      properties: { ...CCTV_RECORD.properties, frameUrl: "", live: false },
    });
    expect(screen.getByText("离线")).toBeInTheDocument();
    expect(screen.getByText("实时画面 不可用")).toBeInTheDocument();
    expect(screen.queryByRole("link", { name: /实时画面/ })).toBeNull();
  });

  test("collapse handle hides the body and expands again", async () => {
    render(<HudDetailPanel />);
    await select(FLIGHT_RECORD);
    const handle = screen.getByRole("button", { name: "折叠详情面板" });
    fireEvent.click(handle);
    expect(screen.queryByText("CA1234")).not.toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "展开详情面板" }),
    ).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "展开详情面板" }));
    expect(screen.getByText("CA1234")).toBeInTheDocument();
  });

  test("selection cleared while panel is up returns to the empty state", async () => {
    render(<HudDetailPanel />);
    await select(FLIGHT_RECORD);
    expect(screen.getByText("CA1234")).toBeInTheDocument();
    harness.record = null;
    act(() => {
      window.dispatchEvent(new CustomEvent("gev:awareness-subject-cleared"));
    });
    await flushSelection();
    expect(screen.getByText("点击地球上的目标查看详情")).toBeInTheDocument();
  });
});
