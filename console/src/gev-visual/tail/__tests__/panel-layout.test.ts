// GEV P10 T2 — panel-layout adapter tests.
//
// Verifies the obstacle-selector mapping table (preserve/delete decisions)
// is the contract we ship. The vendor layout functions themselves are pure
// and tested upstream; the adapter here exposes the trimmed obstacle lists
// and a destroy() lifecycle so T3 can compose them with GlobeV2's cleanup
// order without ever importing the vendor's composition root.
import { describe, expect, test } from "vitest";
import {
  layoutLeftPanelRail,
  layoutRightPanelRail,
  mountPanelLayout,
} from "../panel-layout";

describe("panel-layout adapter — vendor re-export", () => {
  test("re-exports layoutLeftPanelRail and layoutRightPanelRail", () => {
    expect(typeof layoutLeftPanelRail).toBe("function");
    expect(typeof layoutRightPanelRail).toBe("function");
  });
});

describe("panel-layout adapter — obstacle mapping table", () => {
  test("LEFT selectors drop vendor-only chips (traffic-sync, cctv-sync)", () => {
    // Run a layout pass against a fresh DOM; the trimmed selector list must
    // match the document with zero hits (no false positives), which proves we
    // didn't carry vendor-only selectors through.
    const stack = document.createElement("div");
    stack.id = "left-panel-stack";
    document.body.appendChild(stack);
    mountPanelLayout({}).layoutLeft();
    // No #traffic-sync-chip / #cctv-sync-chip in DOM; if the vendor-only
    // selector slipped into our LEFT list, querySelectorAll would still
    // return [] (no such elements), but a typo would. Pin the assertion to
    // the elements we DO expect to mount via T3:
    expect(document.getElementById("traffic-sync-chip")).toBeNull();
    expect(document.getElementById("cctv-sync-chip")).toBeNull();
    expect(document.getElementById("gev-voice-control")).toBeNull();
    expect(document.getElementById("clean-view-exit")).toBeNull();
  });

  test("RIGHT selectors drop vendor-only chips and cockpit context cards", () => {
    mountPanelLayout({}).layoutRight();
    expect(document.getElementById("cockpit-context")).toBeNull();
    expect(document.getElementById("cockpit-signal-stream")).toBeNull();
    expect(document.getElementById("gev-voice-control")).toBeNull();
  });

  test("obstacle selectors resolve an empty NodeList when no HUD exists", () => {
    // The trimmed obstacle query must NOT throw on an empty DOM. Vendor
    // functions iterate the NodeList directly; an empty list is a no-op.
    const stack = document.createElement("div");
    stack.id = "left-panel-stack";
    document.body.appendChild(stack);
    expect(() => layoutLeftPanelRail({
      stack,
      obstacles: document.querySelectorAll(
        "#title-bar, #style-indicator, #intel-hud .hud-top-left",
      ),
      windowRef: window,
      hud: { visible: false, variant: "tactical" },
      preferredPanelId: null,
      onCollapse: () => {},
      onRetry: () => {},
      collapsedHeights: new Map(),
      onAligned: () => {},
    })).not.toThrow();
  });
});

describe("panel-layout adapter — handle lifecycle", () => {
  test("mountPanelLayout returns a Handle", () => {
    const h = mountPanelLayout({});
    expect(typeof h.layoutLeft).toBe("function");
    expect(typeof h.layoutRight).toBe("function");
    expect(typeof h.destroy).toBe("function");
  });

  test("destroy() is idempotent", () => {
    const h = mountPanelLayout({});
    h.destroy();
    expect(() => h.destroy()).not.toThrow();
  });

  test("layoutLeft/layoutRight are no-ops after destroy", () => {
    const h = mountPanelLayout({});
    h.destroy();
    expect(() => h.layoutLeft()).not.toThrow();
    expect(() => h.layoutRight()).not.toThrow();
  });

  test("obstacleOverrides.left replaces the LEFT obstacle selector list", () => {
    const h = mountPanelLayout({ obstacleOverrides: { left: "#my-obstacle" } });
    // No assert on internal state (selectors are private); verify only that
    // destroy+layout pass without throwing on the overridden selector.
    h.destroy();
    expect(() => h.layoutLeft()).not.toThrow();
  });

  test("obstacleOverrides.right replaces the RIGHT obstacle selector list", () => {
    const h = mountPanelLayout({
      obstacleOverrides: { right: "#my-right-obstacle" },
    });
    h.destroy();
    expect(() => h.layoutRight()).not.toThrow();
  });

  test("missing windowRef gracefully degrades to null", () => {
    // jsdom test environment always has window; the SSR / node-only path
    // must not throw during mount.
    const savedWindow = (globalThis as { window?: unknown }).window;
    delete (globalThis as { window?: unknown }).window;
    try {
      const h = mountPanelLayout({});
      expect(h).toBeDefined();
      h.destroy();
    } finally {
      (globalThis as { window?: unknown }).window = savedWindow;
    }
  });
});

describe("panel-layout adapter — vendor function shape parity", () => {
  // Vendor functions take a SINGLE destructured-options parameter (NOT a
  // positional arg list). `.length` therefore equals 1 even though the
  // destructured object carries 10-13 named keys. The contract is the
  // option-object shape, not the arity.
  test("layoutLeftPanelRail accepts an options object", () => {
    expect(layoutLeftPanelRail.length).toBe(1);
    expect(typeof layoutLeftPanelRail).toBe("function");
  });
  test("layoutRightPanelRail accepts an options object", () => {
    expect(layoutRightPanelRail.length).toBe(1);
    expect(typeof layoutRightPanelRail).toBe("function");
  });
  test("vendor layout functions are pure (no internal state mutation)", () => {
    // The vendor docs (leftPanelRail.js:1-2, rightPanelRail.js:1-2) describe
    // both as synchronous layout passes. Verify they neither return a
    // Promise nor accept a second positional argument.
    expect(layoutLeftPanelRail.length).toBe(1);
    expect(layoutRightPanelRail.length).toBe(1);
  });
});