import "@testing-library/jest-dom/vitest";
import { render } from "@testing-library/react";
import { describe, expect, test } from "vitest";
import { HudCockpitTcasOverlay } from "../HudCockpitTcasOverlay";
import type { TcasTarget } from "../../gev-visual/cockpit/tcas-client";

function mkTarget(
  hex: string,
  threat: TcasTarget["threat"],
  distance_nm: number,
  bearing_deg: number,
  alt_m: number,
): TcasTarget {
  return {
    hex,
    flight: hex.toUpperCase(),
    lat: 0,
    lon: 0,
    alt_m,
    gs: 300,
    track: 90,
    squawk: null,
    mil: false,
    distance_nm,
    bearing_deg,
    closure_kts: 0,
    threat,
    age_s: 0,
  };
}

describe("HudCockpitTcasOverlay (GEV P20)", () => {
  test("renders nothing meaningful when targets empty", () => {
    const { container } = render(
      <HudCockpitTcasOverlay targets={[]} agentAltitudeM={1000} />,
    );
    const overlay = container.querySelector(
      '[data-testid="hud-cockpit-tcas-overlay"]',
    );
    expect(overlay).not.toBeNull();
    // No diamond polygons (polygons besides the agent triangle).
    const polygons = container.querySelectorAll("polygon");
    // Only the agent triangle should be present.
    expect(polygons.length).toBe(1);
  });

  test("renders diamonds for active (non-none) targets", () => {
    const targets: TcasTarget[] = [
      mkTarget("a", "warning", 0.5, 0, 1100),
      mkTarget("b", "caution", 1.5, 90, 900),
      mkTarget("c", "monitor", 4, 180, 1000),
    ];
    const { container } = render(
      <HudCockpitTcasOverlay targets={targets} agentAltitudeM={1000} />,
    );
    const polygons = container.querySelectorAll("polygon");
    // 1 agent triangle + 3 target diamonds = 4 polygons.
    expect(polygons.length).toBe(4);
  });

  test("skips 'none' threat targets", () => {
    const targets: TcasTarget[] = [
      mkTarget("a", "none", 1, 0, 1000),
      mkTarget("b", "monitor", 4, 90, 1000),
    ];
    const { container } = render(
      <HudCockpitTcasOverlay targets={targets} agentAltitudeM={1000} />,
    );
    const polygons = container.querySelectorAll("polygon");
    // Only agent + 1 monitor target.
    expect(polygons.length).toBe(2);
  });

  test("skips targets beyond RADIUS_MAX_NM (5 nm)", () => {
    const targets: TcasTarget[] = [
      mkTarget("a", "monitor", 10, 0, 1000),
      mkTarget("b", "caution", 2, 0, 1000),
    ];
    const { container } = render(
      <HudCockpitTcasOverlay targets={targets} agentAltitudeM={1000} />,
    );
    const polygons = container.querySelectorAll("polygon");
    expect(polygons.length).toBe(2); // agent + 1 caution only
  });

  test("renders warning threat tag when any warning present", () => {
    const targets: TcasTarget[] = [
      mkTarget("a", "warning", 0.5, 0, 1000),
      mkTarget("b", "caution", 1.5, 90, 1000),
    ];
    const { container } = render(
      <HudCockpitTcasOverlay targets={targets} agentAltitudeM={1000} />,
    );
    expect(container.querySelector(".hud-cockpit-tcas-threat-tag"))
      .not.toBeNull();
    expect(container.textContent).toContain("WARNING");
  });

  test("renders CAUTION tag (no WARNING) when only caution", () => {
    const targets: TcasTarget[] = [
      mkTarget("a", "caution", 1.5, 0, 1000),
    ];
    const { container } = render(
      <HudCockpitTcasOverlay targets={targets} agentAltitudeM={1000} />,
    );
    expect(container.textContent).toContain("CAUTION");
    expect(container.textContent).not.toContain("WARNING");
  });

  test("renders summary counts", () => {
    const targets: TcasTarget[] = [
      mkTarget("a", "warning", 0.5, 0, 1000),
      mkTarget("b", "caution", 1.5, 90, 1000),
      mkTarget("c", "caution", 2, 180, 1000),
      mkTarget("d", "monitor", 4, 270, 1000),
    ];
    const { container } = render(
      <HudCockpitTcasOverlay targets={targets} agentAltitudeM={1000} />,
    );
    expect(container.textContent).toContain("1"); // warning
    expect(container.textContent).toContain("2"); // caution
    expect(container.textContent).toContain("1"); // monitor (also)
  });

  test("relative altitude ladder renders text when altitude provided", () => {
    const targets: TcasTarget[] = [
      mkTarget("a", "monitor", 1, 0, 2000), // agent=1000 → +1000m → +3281ft
    ];
    const { container } = render(
      <HudCockpitTcasOverlay targets={targets} agentAltitudeM={1000} />,
    );
    expect(container.textContent).toMatch(/\+3\.\d/);
  });

  test("does not render altitude ladder text when agentAltitudeM null", () => {
    const targets: TcasTarget[] = [
      mkTarget("a", "monitor", 1, 0, 2000),
    ];
    const { container } = render(
      <HudCockpitTcasOverlay targets={targets} agentAltitudeM={null} />,
    );
    // No "+Xk" text in this case.
    const texts = Array.from(container.querySelectorAll("text"));
    expect(texts.length).toBe(0);
  });

  test("threatColor returns distinct colors per threat", async () => {
    const mod = await import("../HudCockpitTcasOverlay");
    expect(mod.threatColor("warning")).not.toBe(mod.threatColor("caution"));
    expect(mod.threatColor("caution")).not.toBe(mod.threatColor("monitor"));
    expect(mod.threatColor("monitor")).not.toBe(mod.threatColor("none"));
  });
});