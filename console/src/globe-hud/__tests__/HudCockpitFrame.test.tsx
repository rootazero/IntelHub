import "@testing-library/jest-dom/vitest";
import { cleanup, render, screen, act, fireEvent } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { HudCockpitFrame, useCockpitStore } from "../HudCockpitFrame";
import { createCockpitStore } from "../../gev-visual/cockpit/cockpit-store";
import { mountCockpitInstruments } from "../../gev-visual/cockpit/instruments-mount";
import { mountPanelDrag } from "../../gev-visual/tail/panel-drag";
import { I18nProvider } from "../../i18n";

beforeEach(() => {
  // The instruments child schedules a rAF poll; jsdom has no rAF.
  vi.stubGlobal("requestAnimationFrame", () => 1);
  vi.stubGlobal("cancelAnimationFrame", () => {});
});
afterEach(() => {
  vi.unstubAllGlobals();
  cleanup();
});

function fakeInstruments() {
  return mountCockpitInstruments({
    viewer: { scene: { canvas: {} } },
    flights: { getTrackedInfo: () => null },
  } as never);
}

describe("HudCockpitFrame", () => {
  test("renders nothing while the store is inactive", () => {
    const store = createCockpitStore();
    const { container } = render(
      <HudCockpitFrame
        store={store}
        instruments={null}
        briefing={null}
        vision={null}
      />,
    );
    expect(container).toBeEmptyDOMElement();
    expect(screen.queryByTestId("hud-cockpit-frame")).not.toBeInTheDocument();
  });

  test("composes all four sub-components + exit button when active", () => {
    const store = createCockpitStore();
    act(() => store.enter("abc123"));
    render(
      <HudCockpitFrame
        store={store}
        getTrackedInfo={() => null}
        instruments={fakeInstruments()}
        briefing={null}
        vision={null}
      />,
    );
    expect(screen.getByTestId("hud-cockpit-frame")).toBeInTheDocument();
    expect(screen.getByTestId("hud-cockpit-context")).toBeInTheDocument();
    expect(screen.getByTestId("hud-cockpit-instruments")).toBeInTheDocument();
    expect(screen.getByTestId("hud-cockpit-compass")).toBeInTheDocument();
    expect(screen.getByTestId("hud-cockpit-altimeter")).toBeInTheDocument();
    expect(screen.getByTestId("hud-cockpit-speed")).toBeInTheDocument();
    expect(screen.getByTestId("hud-cockpit-briefing")).toBeInTheDocument();
    expect(screen.getByTestId("hud-cockpit-tab-weather")).toBeInTheDocument();
    expect(screen.getByTestId("hud-cockpit-tab-summary")).toBeInTheDocument();
    expect(screen.getByTestId("hud-cockpit-vision-switch")).toBeInTheDocument();
    expect(screen.getByTestId("hud-cockpit-vision-optical")).toBeInTheDocument();
    expect(screen.getByTestId("hud-cockpit-vision-noir")).toBeInTheDocument();
    const exit = screen.getByTestId("hud-cockpit-exit");
    expect(exit).toHaveTextContent("退出驾驶舱（继续跟随）");
  });

  test("exit button drives store.exit() and unmounts the frame", () => {
    const store = createCockpitStore();
    act(() => store.enter("abc123"));
    render(
      <HudCockpitFrame
        store={store}
        getTrackedInfo={() => null}
        instruments={fakeInstruments()}
        briefing={null}
        vision={null}
      />,
    );
    expect(screen.getByTestId("hud-cockpit-frame")).toBeInTheDocument();
    fireEvent.click(screen.getByTestId("hud-cockpit-exit"));
    expect(store.getState().active).toBe(false);
    expect(screen.queryByTestId("hud-cockpit-frame")).not.toBeInTheDocument();
  });

  test("vision switch reflects + drives the store's visionMode", () => {
    const store = createCockpitStore();
    act(() => store.enter("abc123"));
    render(
      <HudCockpitFrame
        store={store}
        getTrackedInfo={() => null}
        instruments={fakeInstruments()}
        briefing={null}
        vision={null}
      />,
    );
    fireEvent.click(screen.getByTestId("hud-cockpit-vision-thermal"));
    expect(store.getState().visionMode).toBe("thermal");
    expect(screen.getByTestId("hud-cockpit-vision-thermal")).toHaveAttribute(
      "aria-pressed",
      "true",
    );
  });
  test("shows the keyboard-shortcut hint strip only while active", () => {
    const store = createCockpitStore();
    expect(
      screen.queryByTestId("hud-cockpit-shortcut-hint"),
    ).not.toBeInTheDocument();
    act(() => store.enter("abc123"));
    render(
      <I18nProvider>
        <HudCockpitFrame
          store={store}
          getTrackedInfo={() => null}
          instruments={null}
          briefing={null}
          vision={null}
        />
      </I18nProvider>,
    );
    const hint = screen.getByTestId("hud-cockpit-shortcut-hint");
    expect(hint).toHaveTextContent("Shift+C");
    act(() => store.exit());
    expect(
      screen.queryByTestId("hud-cockpit-shortcut-hint"),
    ).not.toBeInTheDocument();
  });

  test("Shift+C hides the panels and keeps a way back", () => {
    const store = createCockpitStore();
    act(() => store.enter("abc123"));
    render(
      <HudCockpitFrame
        store={store}
        getTrackedInfo={() => null}
        instruments={fakeInstruments()}
        briefing={null}
        vision={null}
      />,
    );
    expect(screen.getByTestId("hud-cockpit-briefing")).toBeInTheDocument();

    act(() => {
      window.dispatchEvent(
        new KeyboardEvent("keydown", {
          key: "C",
          shiftKey: true,
          bubbles: true,
          cancelable: true,
        }),
      );
    });
    expect(store.getState().hidden).toBe(true);
    expect(screen.getByTestId("hud-cockpit-frame")).toHaveAttribute(
      "data-hidden",
      "true",
    );
    expect(
      screen.queryByTestId("hud-cockpit-briefing"),
    ).not.toBeInTheDocument();
    // The hint strip is the affordance that gets you back out.
    expect(screen.getByTestId("hud-cockpit-shortcut-hint")).toBeInTheDocument();

    act(() => {
      window.dispatchEvent(
        new KeyboardEvent("keydown", {
          key: "C",
          shiftKey: true,
          bubbles: true,
          cancelable: true,
        }),
      );
    });
    expect(store.getState().hidden).toBe(false);
    expect(screen.getByTestId("hud-cockpit-briefing")).toBeInTheDocument();
  });

  test("enter flies the camera to the tracked aircraft; exit flies back", async () => {
    const flyTo = vi.fn().mockResolvedValue(true);
    const viewer = {
      scene: { canvas: {} },
      camera: {
        position: { clone: () => ({ x: 1, y: 2, z: 3 }) },
        heading: 0.5,
        pitch: -0.3,
        roll: 0,
        flyTo,
      },
    };
    // jsdom has no rAF: run the enter callback synchronously so the one-frame
    // deferral (spec R4) is observable in the test.
    vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => {
      cb(0);
      return 1;
    });
    const getTrackedInfo = () => ({
      longitude: -74.17,
      latitude: 40.69,
      altitudeM: 10000,
    });
    const store = createCockpitStore();
    render(
      <HudCockpitFrame
        store={store}
        viewer={viewer}
        getTrackedInfo={getTrackedInfo}
        instruments={null}
        briefing={null}
        vision={null}
      />,
    );
    // Viewer mounted but cockpit inactive → no flight yet.
    expect(flyTo).not.toHaveBeenCalled();

    act(() => store.enter("abc123"));
    await act(async () => {});
    expect(flyTo).toHaveBeenCalledTimes(1);
    const enterReq = flyTo.mock.calls[0][0] as { duration: number };
    expect(enterReq.duration).toBe(0.4);

    act(() => store.exit());
    await act(async () => {});
    expect(flyTo).toHaveBeenCalledTimes(2);
  });

  test("P12 T7: active cockpit locks the viewport and freezes panel drag", () => {
    const canvas = document.createElement("canvas");
    const controller = { enableInputs: true };
    const viewer = {
      scene: { screenSpaceCameraController: controller, canvas: {} },
      cesiumWidget: { canvas },
      camera: {
        position: { clone: () => ({ x: 0, y: 0, z: 0 }) },
        heading: 0,
        pitch: 0,
        roll: 0,
        flyTo: vi.fn().mockResolvedValue(true),
      },
    };
    const ppToggle = document.createElement("div");
    ppToggle.id = "pp-toggles";
    document.body.appendChild(ppToggle);
    const drag = mountPanelDrag(ppToggle, {
      syncPanelCollapseButton: vi.fn(),
      layoutRightPanels: vi.fn(),
      syncCctvPanelViewport: vi.fn(),
      showToast: vi.fn(),
    });
    const store = createCockpitStore();
    try {
      render(
        <HudCockpitFrame
          store={store}
          viewer={viewer}
          getTrackedInfo={() => null}
          instruments={null}
          briefing={null}
          vision={null}
        />,
      );
      expect(controller.enableInputs).toBe(true);
      expect(drag.isDisabled()).toBe(false);

      act(() => store.enter("abc123"));
      expect(controller.enableInputs).toBe(false);
      expect(canvas.style.cursor).toBe("none");
      expect(drag.isDisabled()).toBe(true);

      act(() => store.exit());
      expect(controller.enableInputs).toBe(true);
      expect(canvas.style.cursor).toBe("");
      expect(drag.isDisabled()).toBe(false);
    } finally {
      drag.destroy();
      ppToggle.remove();
    }
  });
});

// ── GEV P17: element visibility switch integration ──────────────────

describe("HudCockpitFrame (GEV P17 element visibility)", () => {
  test("renders HudCockpitElementSwitch when cockpit is active", () => {
    const store = createCockpitStore();
    act(() => store.enter("abc123"));
    render(
      <HudCockpitFrame
        store={store}
        getTrackedInfo={() => null}
        instruments={fakeInstruments()}
        briefing={null}
        vision={null}
      />,
    );
    expect(
      screen.getByTestId("hud-cockpit-element-switch"),
    ).toBeInTheDocument();
  });

  test("HudCockpitInstruments receives visibility from store (hidden elements disappear)", () => {
    const store = createCockpitStore();
    act(() => store.enter("abc123"));
    act(() =>
      store.setElementVisibility({
        compass: false,
        pitchLadder: false,
        bankIndicator: false,
        vsiChevron: false,
        altitudeLadder: false,
        speedTape: false,
        altimeter: false,
        speedRuler: false,
      }),
    );
    render(
      <HudCockpitFrame
        store={store}
        getTrackedInfo={() => null}
        instruments={fakeInstruments()}
        briefing={null}
        vision={null}
      />,
    );
    expect(screen.getByTestId("hud-cockpit-instruments")).toBeInTheDocument();
    expect(screen.queryByTestId("hud-cockpit-compass")).not.toBeInTheDocument();
    expect(screen.queryByTestId("pitch-ladder")).not.toBeInTheDocument();
    expect(screen.queryByTestId("vsi-chevron")).not.toBeInTheDocument();
  });
});

describe("useCockpitStore", () => {
  test("tracks store transitions through a React render", () => {
    const store = createCockpitStore();
    let seen: boolean | null = null;
    function Probe() {
      const state = useCockpitStore(store);
      seen = state.active;
      return null;
    }
    render(<Probe />);
    expect(seen).toBe(false);
    act(() => store.enter("x"));
    expect(seen).toBe(true);
    act(() => store.exit());
    expect(seen).toBe(false);
  });
});
