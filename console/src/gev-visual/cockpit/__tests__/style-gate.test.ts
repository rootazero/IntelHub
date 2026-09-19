// GEV P9 R5 — globe style picker gate (highest-risk rule).
//
// Verifies the exact 4-step contract from the brief:
//   1. pick 'anime'            → stage 'anime' intensity > 0
//   2. enter cockpit + crt     → stage 'retro' intensity > 0
//   3. setStyle('anime') while cockpit active → style does NOT change
//   4. exit cockpit            → style 'anime' restored
//
// Everything runs against the REAL mountVisualEffects (manual-frame driver) +
// REAL mountCockpitVision (Map→record bridge) + REAL createCockpitStore, so
// the gate is proven end-to-end rather than against lenient mocks.
import { describe, expect, test } from "vitest";
import { gateStyleWhileCockpitActive } from "../style-gate";
import { createCockpitStore } from "../cockpit-store";
import { mountCockpitVision } from "../vision-mount";
import { mountVisualEffects } from "../../visual-effects";

function fakeViewer() {
  const stages: any[] = [];
  const bloom = {
    enabled: true,
    uniforms: {
      contrast: 128,
      brightness: -0.6,
      delta: 1.0,
      sigma: 2.0,
      stepSize: 5.0,
    },
  };
  return {
    scene: {
      requestRender: () => {},
      postProcessStages: {
        bloom,
        add(stage: any) {
          if (!stage || typeof stage !== "object" || !("fragmentShader" in stage)) {
            throw new TypeError(
              "postProcessStages.add: stage must carry fragmentShader",
            );
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

function manualFrames() {
  const queue: Array<{ id: number; cb: (t: number) => void }> = [];
  let nextId = 1;
  let now = 0;
  return {
    deps: {
      createStage: (options: any) => ({
        ...options,
        enabled: true,
        uniforms: { ...(options.uniforms ?? {}) },
      }),
      requestFrame: (cb: (t: number) => void) => {
        queue.push({ id: nextId, cb });
        return nextId++;
      },
      cancelFrame: (id: number) => {
        const i = queue.findIndex((f) => f.id === id);
        if (i >= 0) queue.splice(i, 1);
      },
      now: () => now,
    },
    tick(ms: number) {
      now += ms;
      const pending = queue.splice(0, queue.length);
      for (const f of pending) f.cb(now);
    },
  };
}

describe("gateStyleWhileCockpitActive (R5)", () => {
  test("picker is gated while cockpit active and replayed on exit (4-step contract)", () => {
    const frames = manualFrames();
    const effects = mountVisualEffects(fakeViewer() as any, frames.deps);
    const store = createCockpitStore();
    const vision = mountCockpitVision(effects);
    const gate = gateStyleWhileCockpitActive(effects, store);

    const intensity = (name: string) =>
      effects.getStages()!.get(name)!.uniforms.intensity as number;

    // 1. pick 'anime' → anime lights.
    gate.setStyle("anime");
    frames.tick(600);
    expect(intensity("anime")).toBeGreaterThan(0);

    // 2. enter cockpit + crt → retro lights (vision owns the stages).
    store.enter("abc123");
    vision.setMode("crt");
    expect(intensity("retro")).toBeGreaterThan(0);

    // 3. pick 'anime' while cockpit active → NO-OP (retro stays lit).
    gate.setStyle("anime");
    expect(intensity("retro")).toBeGreaterThan(0);
    expect(intensity("anime")).toBe(0);

    // 4. exit cockpit → the user's last pick ('anime') is restored.
    store.exit();
    gate.restorePicked();
    frames.tick(600);
    expect(intensity("anime")).toBeGreaterThan(0);
    expect(intensity("retro")).toBe(0);
  });

  test("records the user pick while gated so restorePicked replays it", () => {
    const frames = manualFrames();
    const effects = mountVisualEffects(fakeViewer() as any, frames.deps);
    const store = createCockpitStore();
    const gate = gateStyleWhileCockpitActive(effects, store);

    store.enter("x");
    gate.setStyle("thermal"); // gated: recorded, not applied
    expect(gate.getPicked()).toBe("thermal");
    store.exit();
    gate.restorePicked();
    frames.tick(600);
    expect(
      effects.getStages()!.get("thermal")!.uniforms.intensity,
    ).toBeGreaterThan(0);
  });

  test("when cockpit is inactive the gate applies immediately", () => {
    const frames = manualFrames();
    const effects = mountVisualEffects(fakeViewer() as any, frames.deps);
    const store = createCockpitStore();
    const gate = gateStyleWhileCockpitActive(effects, store);

    gate.setStyle("noir");
    frames.tick(600);
    expect(effects.getStages()!.get("noir")!.uniforms.intensity).toBeGreaterThan(
      0,
    );
  });
});
