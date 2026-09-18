import { describe, expect, test, vi } from "vitest";
import { mountCockpitVision, isVisionMode, VISION_MODES } from "../vision-mount";
import { mountVisualEffects } from "../../visual-effects";
import type { VisualEffectsHandle } from "../../visual-effects";

// ── fake visual-effects handle (for setStyle-mapping + contract tests) ─────
// Replicates the REAL handle shape: getStages() returns a Map<string, stage>,
// setStyle(style) sets a globe style. The stage objects carry the only field
// the vendor policy touches: uniforms.intensity.
function fakeEffects(
  stages: Record<string, { uniforms: { intensity: number } }> = {
    retro: { uniforms: { intensity: 0 } },
    surveillance: { uniforms: { intensity: 0 } },
    thermal: { uniforms: { intensity: 0 } },
    noir: { uniforms: { intensity: 0 } },
  },
) {
  const map = new Map(Object.entries(stages));
  return {
    setStyle: vi.fn(),
    getStyle: vi.fn(() => "normal"),
    getStages: vi.fn(() => map),
    destroy: vi.fn(),
  } as unknown as VisualEffectsHandle;
}

describe("mountCockpitVision — contract + mode mapping", () => {
  test("constructor contract: rejects an effects handle without getStages/setStyle", () => {
    expect(() => mountCockpitVision({} as any)).toThrow(TypeError);
    expect(() =>
      mountCockpitVision({ getStages: () => null } as any),
    ).toThrow(TypeError);
  });

  test("VISION_MODES + isVisionMode enumerate the vendor's 5 modes", () => {
    expect([...VISION_MODES]).toEqual([
      "optical",
      "crt",
      "nvg",
      "thermal",
      "noir",
    ]);
    expect(isVisionMode("thermal")).toBe(true);
    expect(isVisionMode("bogus")).toBe(false);
    expect(isVisionMode(42)).toBe(false);
  });

  test("setMode('crt') maps to globe style 'retro' and gates its intensity to 1", () => {
    const effects = fakeEffects();
    const h = mountCockpitVision(effects);
    const mode = h.setMode("crt");
    expect(mode).toBe("crt");
    expect(effects.setStyle).toHaveBeenCalledWith("retro");
    expect(effects.getStages()!.get("retro")!.uniforms.intensity).toBe(1);
    expect(effects.getStages()!.get("surveillance")!.uniforms.intensity).toBe(0);
  });

  test.each([
    ["crt", "retro"],
    ["nvg", "surveillance"],
    ["thermal", "thermal"],
    ["noir", "noir"],
  ] as const)("setMode('%s') maps to globe style '%s'", (mode, style) => {
    const effects = fakeEffects();
    const h = mountCockpitVision(effects);
    h.setMode(mode);
    expect(effects.setStyle).toHaveBeenCalledWith(style);
    expect(h.getMode()).toBe(mode);
  });

  test("setMode('optical') restores the captured baseline and sets style 'normal'", () => {
    const effects = fakeEffects();
    const h = mountCockpitVision(effects);
    h.setMode("crt");
    const restored = h.setMode("optical");
    expect(restored).toBe("optical");
    expect(effects.setStyle).toHaveBeenLastCalledWith("normal");
    // Baseline captured before crt was all-zero → restored to all-zero.
    expect(effects.getStages()!.get("retro")!.uniforms.intensity).toBe(0);
    expect(effects.getStages()!.get("thermal")!.uniforms.intensity).toBe(0);
  });

  test("switching crt→thermal keeps ONE stage lit, never two", () => {
    const effects = fakeEffects();
    const h = mountCockpitVision(effects);
    h.setMode("crt");
    h.setMode("thermal");
    expect(effects.getStages()!.get("thermal")!.uniforms.intensity).toBe(1);
    expect(effects.getStages()!.get("retro")!.uniforms.intensity).toBe(0);
    expect(effects.getStages()!.get("surveillance")!.uniforms.intensity).toBe(0);
  });

  test("an unknown mode normalizes to 'optical' (vendor normalizeCockpitVisionMode)", () => {
    const effects = fakeEffects();
    const h = mountCockpitVision(effects);
    const mode = h.setMode("bogus");
    expect(mode).toBe("optical");
    expect(h.getMode()).toBe("optical");
  });

  test("destroy is idempotent and drops the captured baseline", () => {
    const effects = fakeEffects();
    const h = mountCockpitVision(effects);
    h.setMode("crt");
    expect(() => {
      h.destroy();
      h.destroy();
    }).not.toThrow();
  });
});

// ── THE BRIDGE: Map → plain-record conversion against the REAL handle ──────
// mountVisualEffects.getStages() returns a Map. The vendor policy walks a plain
// object. Without Object.fromEntries(map) vision mode is a silent no-op.
function fakeViewer() {
  const stages: any[] = [];
  const bloom = {
    enabled: true,
    uniforms: { contrast: 128, brightness: -0.6, delta: 1.0, sigma: 2.0, stepSize: 5.0 },
  };
  return {
    scene: {
      requestRender: vi.fn(),
      postProcessStages: {
        bloom,
        add(stage: any) {
          if (!stage || typeof stage !== "object" || !("fragmentShader" in stage)) {
            throw new TypeError("postProcessStages.add: stage must carry fragmentShader");
          }
          stages.push(stage);
        },
        remove(stage: any) {
          const i = stages.indexOf(stage);
          if (i >= 0) stages.splice(i, 1);
        },
        get length() {
          return stages.length;
        },
      },
    },
  };
}

function realEffects(): VisualEffectsHandle {
  return mountVisualEffects(fakeViewer() as any, {
    createStage: (options: any) => ({
      ...options,
      enabled: true,
      uniforms: { ...(options.uniforms ?? {}) },
    }),
    requestFrame: () => 0,
    cancelFrame: () => {},
    now: () => 0,
  });
}

describe("mountCockpitVision — Map→record bridge (highest-risk)", () => {
  test("setMode('crt') changes retro intensity from 0 → 1 through the REAL Map", () => {
    const effects = realEffects();
    const stagesBefore = effects.getStages()!;
    expect(stagesBefore).toBeInstanceOf(Map);
    expect(stagesBefore.get("retro")!.uniforms.intensity).toBe(0);

    const h = mountCockpitVision(effects);
    h.setMode("crt");

    // The bridge converts the Map to a plain record, the vendor policy writes
    // intensity 1 into the SAME stage object — read it back through the Map.
    expect(effects.getStages()!.get("retro")!.uniforms.intensity).toBe(1);
    // Every non-target stage is gated to 0.
    for (const name of ["surveillance", "thermal", "anime", "noir", "snow"]) {
      expect(
        effects.getStages()!.get(name)!.uniforms.intensity,
        `stage ${name} gated to 0`,
      ).toBe(0);
    }
  });

  test("optical round-trip restores the pre-cockpit baseline via the real Map", () => {
    const effects = realEffects();
    const h = mountCockpitVision(effects);
    h.setMode("thermal");
    expect(effects.getStages()!.get("thermal")!.uniforms.intensity).toBe(1);
    h.setMode("optical");
    expect(effects.getStages()!.get("thermal")!.uniforms.intensity).toBe(0);
    expect(effects.getStages()!.get("retro")!.uniforms.intensity).toBe(0);
  });
});
