// T9: HUD layer rail.
//   describe 1 — domain mapping covers the vendor LAYER_STATE_REGISTRY exactly
//     (every registry id in exactly one domain; every domain id real).
//   describe 2 — rail behavior against a mock manager (icons, flyout, toggle
//     call shape, collapse handle, 3s flyout auto-close).
//   describe 3 — stub-layer degradation through the REAL vendored
//     LayerLifecycle: enabling a layer whose update path fails (the stub
//     "503/empty" reality) settles to a truthful OFF and the rail checkbox
//     converges back via subscribe().
import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
} from "@testing-library/react";
import { afterEach, describe, expect, test, vi } from "vitest";
import "@testing-library/jest-dom/vitest";

// Real vendor registry + lifecycle (no mocks — this is the contract under
// test; the gev-engine/* alias resolves via vitest.config.ts).
import { LAYER_STATE_REGISTRY } from "gev-engine/src/data/layerState.js";
import { LayerLifecycle } from "gev-engine/src/data/lifecycle.js";

import { DOMAINS, DOMAIN_BY_LAYER_ID } from "../domains";
import { HudLayerRail } from "../HudLayerRail";
import type { RailLayerInfo, RailManager } from "../HudLayerRail";

afterEach(cleanup);

// ---- Mock manager factory (describe 2) -------------------------------------

const REGISTRY_IDS: string[] = (LAYER_STATE_REGISTRY as { id: string }[]).map(
  (entry) => entry.id,
);

function mockManager(overrides: Partial<RailManager> = {}) {
  const listeners = new Set<
    (change: { type: string; layerId?: string }) => void
  >();
  const enabled = new Set<string>();
  return {
    getAll: vi.fn((): RailLayerInfo[] =>
      REGISTRY_IDS.map((id) => ({ id, name: id, showInTogglePanel: true })),
    ),
    isEffectivelyEnabled: vi.fn((id: string) => enabled.has(id)),
    setEnabled: vi.fn((id: string, on: boolean) => {
      if (on) enabled.add(id);
      else enabled.delete(id);
      for (const listener of listeners)
        listener({ type: "visibility", layerId: id });
      return Promise.resolve(true);
    }),
    subscribe: vi.fn(
      (callback: (change: { type: string; layerId?: string }) => void) => {
        listeners.add(callback);
        return () => listeners.delete(callback);
      },
    ),
    fire: (change: { type: string; layerId?: string }) => {
      for (const listener of listeners) listener(change);
    },
    ...overrides,
  } as RailManager & {
    setEnabled: ReturnType<typeof vi.fn>;
    fire: (change: { type: string; layerId?: string }) => void;
  };
}

// ---- describe 1: domain mapping vs the real registry ------------------------

describe("DOMAINS covers LAYER_STATE_REGISTRY exactly", () => {
  test("7 domains with zh/en labels and icons", () => {
    expect(DOMAINS).toHaveLength(7);
    for (const domain of DOMAINS) {
      expect(domain.icon).toBeTruthy();
      expect(domain.label.zh).toBeTruthy();
      expect(domain.label.en).toBeTruthy();
      expect(domain.layers.length).toBeGreaterThan(0);
    }
  });

  test("every registry id belongs to exactly one domain", () => {
    const claimed = DOMAINS.flatMap((domain) => domain.layers);
    for (const id of REGISTRY_IDS) {
      expect(claimed.filter((layerId) => layerId === id)).toHaveLength(1);
    }
  });

  test("every domain-listed id exists in the registry (no drift)", () => {
    for (const domain of DOMAINS) {
      for (const layerId of domain.layers) {
        expect(REGISTRY_IDS, `unknown layer id: ${layerId}`).toContain(layerId);
      }
    }
  });

  test("DOMAIN_BY_LAYER_ID is the flat inverse map", () => {
    for (const domain of DOMAINS) {
      for (const layerId of domain.layers) {
        expect(DOMAIN_BY_LAYER_ID[layerId]?.id).toBe(domain.id);
      }
    }
  });
});

// ---- describe 2: rail behavior against a mock manager -----------------------

describe("HudLayerRail", () => {
  test("renders one icon button per domain in the left rail", () => {
    render(<HudLayerRail manager={mockManager()} />);
    expect(screen.getByTestId("hud-layer-rail")).toBeInTheDocument();
    // 7 domains → 7 icon menuitems + 1 collapse handle.
    expect(screen.getAllByRole("menuitem")).toHaveLength(7);
    expect(screen.getByRole("button", { name: "折叠图层栏" })).toBeInTheDocument();
  });

  test("hover opens the domain flyout with its layer checkboxes", () => {
    render(<HudLayerRail manager={mockManager()} />);
    fireEvent.mouseEnter(screen.getByRole("menuitem", { name: "海洋 Sea" }));
    const flyout = document.querySelector(".hud-rail-flyout");
    expect(flyout).not.toBeNull();
    expect(flyout).toHaveAttribute("data-domain", "sea");
    // Sea = ais-live-vessels + telegeography-submarine-cables.
    const boxes = flyout!.querySelectorAll('input[type="checkbox"]');
    expect(boxes).toHaveLength(2);
  });

  test("toggle calls manager.setEnabled(id, next, {origin: 'user'})", () => {
    const manager = mockManager();
    render(<HudLayerRail manager={manager} />);
    fireEvent.mouseEnter(screen.getByRole("menuitem", { name: "太空 Space" }));
    const flyout = document.querySelector(".hud-rail-flyout")!;
    const sat = screen.getByRole("checkbox", { name: /satellites/ });
    fireEvent.click(sat);
    expect(manager.setEnabled).toHaveBeenCalledWith("satellites", true, {
      origin: "user",
    });
    // Manager published the settled event → checkbox reflects the new state.
    expect(sat).toBeChecked();
  });

  test("layers opted out via showInTogglePanel are hidden from the flyout", () => {
    const manager = mockManager({
      getAll: () => [
        { id: "flights", name: "flights", showInTogglePanel: true },
        { id: "military-awareness", name: "awareness", showInTogglePanel: false },
      ],
    });
    render(<HudLayerRail manager={manager} />);
    fireEvent.mouseEnter(screen.getByRole("menuitem", { name: "航空 Air" }));
    const flyout = document.querySelector(".hud-rail-flyout")!;
    const names = [...flyout.querySelectorAll(".hud-rail-layer-name")].map(
      (node) => node.textContent,
    );
    expect(names).toContain("flights");
    expect(names).not.toContain("awareness");
  });

  test("domain icon shows active state when a layer is enabled", () => {
    const manager = mockManager();
    render(<HudLayerRail manager={manager} />);
    expect(
      screen.getByRole("menuitem", { name: "航空 Air" }).className,
    ).not.toContain("active");
    // Manager-published event outside React — flush via act, then re-query
    // (the epoch bump re-renders with a fresh element).
    act(() => void manager.setEnabled("flights", true));
    expect(screen.getByRole("menuitem", { name: "航空 Air" }).className).toContain(
      "active",
    );
  });

  test("collapse handle toggles .collapsed on the rail", () => {
    render(<HudLayerRail manager={mockManager()} />);
    const rail = screen.getByTestId("hud-layer-rail");
    expect(rail.className).not.toContain("collapsed");
    fireEvent.click(screen.getByRole("button", { name: "折叠图层栏" }));
    expect(rail.className).toContain("collapsed");
    fireEvent.click(screen.getByRole("button", { name: "展开图层栏" }));
    expect(rail.className).not.toContain("collapsed");
  });

  test("flyout auto-closes 3s after the mouse leaves the rail", () => {
    vi.useFakeTimers();
    try {
      render(<HudLayerRail manager={mockManager()} />);
      fireEvent.mouseEnter(screen.getByRole("menuitem", { name: "环境 Environment" }));
      expect(document.querySelector(".hud-rail-flyout")).not.toBeNull();
      fireEvent.mouseLeave(screen.getByTestId("hud-layer-rail"));
      act(() => vi.advanceTimersByTime(2999));
      expect(document.querySelector(".hud-rail-flyout")).not.toBeNull();
      act(() => vi.advanceTimersByTime(2));
      expect(document.querySelector(".hud-rail-flyout")).toBeNull();
      // Re-entering cancels a pending auto-close.
      fireEvent.mouseEnter(screen.getByRole("menuitem", { name: "环境 Environment" }));
      fireEvent.mouseLeave(screen.getByTestId("hud-layer-rail"));
      act(() => vi.advanceTimersByTime(1000));
      fireEvent.mouseEnter(screen.getByTestId("hud-layer-rail"));
      act(() => vi.advanceTimersByTime(5000));
      expect(document.querySelector(".hud-rail-flyout")).not.toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });

  test("clicking an open domain icon toggles the flyout closed", () => {
    render(<HudLayerRail manager={mockManager()} />);
    const sea = screen.getByRole("menuitem", { name: "海洋 Sea" });
    fireEvent.click(sea);
    expect(document.querySelector(".hud-rail-flyout")).not.toBeNull();
    fireEvent.click(sea);
    expect(document.querySelector(".hud-rail-flyout")).toBeNull();
  });
});

// ---- describe 3: stub degradation through the REAL LayerLifecycle -----------

/** Minimal module mimicking a T3 stub-backed layer (radio/vessels shape). */
function stubModule(id: string, update: () => Promise<unknown>) {
  return {
    id,
    name: id,
    init: vi.fn(async () => true),
    enable: vi.fn(async () => true),
    disable: vi.fn(async () => true),
    update: vi.fn(update),
    getStats: () => ({ count: 0 }),
  };
}

describe("stub layer toggles through the real LayerLifecycle", () => {
  test("stub returning empty data (200-empty): enable settles ON, checkbox stays on", async () => {
    const manager = new LayerLifecycle({} as never);
    // Register under the canonical registry id — the rail's flyout matches
    // domain layers by LAYER_STATE_REGISTRY id, which is what the real
    // catalog provides (createApplicationVessels → 'ais-live-vessels').
    manager.register(stubModule("ais-live-vessels", async () => true));
    const rail = render(<HudLayerRail manager={manager as never} />);
    fireEvent.mouseEnter(screen.getByRole("menuitem", { name: "海洋 Sea" }));
    const box = screen.getByRole("checkbox", { name: /ais-live-vessels/ });
    expect(box).not.toBeChecked();
    fireEvent.click(box);
    // Serialized queue: wait until the transition actually settles.
    await vi.waitFor(() => expect(box).toBeChecked());
    expect(manager.isEnabled("ais-live-vessels")).toBe(true);
    rail.unmount();
  });

  test("stub in 503-degraded state (update throws): settle is truthfully OFF and the rail reverts", async () => {
    const manager = new LayerLifecycle({} as never);
    manager.register(
      stubModule("radio", async () => {
        throw new Error("temporarily unavailable (stub 503)");
      }),
    );
    render(<HudLayerRail manager={manager as never} />);
    fireEvent.mouseEnter(screen.getByRole("menuitem", { name: "地面 Ground" }));
    const box = screen.getByRole("checkbox", { name: /radio/ });
    fireEvent.click(box);
    // Enable fails at first update → manager fails closed to OFF and the
    // subscribe() epoch bump reverts the checkbox — never a stuck ON.
    await vi.waitFor(() => expect(box).not.toBeChecked());
    expect(manager.isEnabled("radio")).toBe(false);
    expect(manager.getLayerLifecycleState("radio")).toMatchObject({
      enabled: false,
    });
  });

  test("setEnabled on an unknown layer id resolves silently", async () => {
    const manager = new LayerLifecycle({} as never);
    await expect(
      manager.setEnabled("no-such-layer", true, { origin: "user" }),
    ).resolves.toBeUndefined();
  });
});
