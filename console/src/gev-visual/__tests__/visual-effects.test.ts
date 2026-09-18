import { describe, expect, test, vi } from "vitest";
import { mountVisualEffects, GLOBE_STYLES } from "../visual-effects";

// Fake shape mirrors vendor visualEffects.test.mjs:9-58, plus the P3
// lenient-mock guard: the factory asserts the shape it is handed.
function fakeViewer() {
  const stages: any[] = [];
  const bloom = {
    enabled: true,
    uniforms: { glowOnly: false, contrast: 128, brightness: -0.6, delta: 1.0, sigma: 2.0, stepSize: 5.0 },
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
        get length() { return stages.length; },
        byName(name: string) { return stages.filter((s) => s.name === name); },
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
      createStage: (options: any) => {
        if (!options || typeof options.fragmentShader !== "string") {
          throw new TypeError("createStage: options.fragmentShader must be a string");
        }
        return { ...options, enabled: true, uniforms: { ...(options.uniforms ?? {}) } };
      },
      requestFrame: (cb: (t: number) => void) => { queue.push({ id: nextId, cb }); return nextId++; },
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

describe("mountVisualEffects", () => {
  test("rejects a viewer without postProcessStages.add (constructor contract)", () => {
    expect(() => mountVisualEffects({ scene: {} } as any)).toThrow(TypeError);
  });

  test("init creates one stage per style plus sharpen, all style stages disabled", () => {
    const viewer = fakeViewer();
    const frames = manualFrames();
    const handle = mountVisualEffects(viewer as any, frames.deps);
    for (const name of ["retro", "surveillance", "thermal", "anime", "noir", "snow"]) {
      const [stage] = viewer.scene.postProcessStages.byName(`godsEyeView_${name}`);
      expect(stage, name).toBeTruthy();
      expect(stage.enabled).toBe(false);
    }
    expect(viewer.scene.postProcessStages.byName("godsEyeView_sharpen")).toHaveLength(1);
    handle.destroy();
  });

  test("setStyle crossfades: previous fades out, next fades in over 500ms", () => {
    const viewer = fakeViewer();
    const frames = manualFrames();
    const handle = mountVisualEffects(viewer as any, frames.deps);
    handle.setStyle("thermal");
    frames.tick(600); // > TRANSITION_DURATION_MS
    let [thermal] = viewer.scene.postProcessStages.byName("godsEyeView_thermal");
    expect(thermal.uniforms.intensity).toBe(1.0);
    expect(thermal.enabled).toBe(true);
    handle.setStyle("retro");
    frames.tick(600);
    [thermal] = viewer.scene.postProcessStages.byName("godsEyeView_thermal");
    const [retro] = viewer.scene.postProcessStages.byName("godsEyeView_retro");
    expect(thermal.uniforms.intensity).toBe(0.0);
    expect(thermal.enabled).toBe(false);
    expect(retro.uniforms.intensity).toBe(1.0);
    expect(handle.getStyle()).toBe("retro");
    handle.destroy();
  });

  test("setStyle('normal') fades everything out", () => {
    const viewer = fakeViewer();
    const frames = manualFrames();
    const handle = mountVisualEffects(viewer as any, frames.deps, "noir");
    handle.setStyle("normal");
    frames.tick(600);
    const [noir] = viewer.scene.postProcessStages.byName("godsEyeView_noir");
    expect(noir.uniforms.intensity).toBe(0.0);
    expect(handle.getStyle()).toBe("normal");
    handle.destroy();
  });

  test("initialStyle applies instantly (no transition needed)", () => {
    const viewer = fakeViewer();
    const frames = manualFrames();
    const handle = mountVisualEffects(viewer as any, frames.deps, "surveillance");
    const [nvg] = viewer.scene.postProcessStages.byName("godsEyeView_surveillance");
    expect(nvg.uniforms.intensity).toBe(1.0);
    expect(nvg.enabled).toBe(true);
    expect(handle.getStyle()).toBe("surveillance");
    handle.destroy();
  });

  test("style preset defaults drive bloom/sharpen; anime/noir/snow use local fallbacks", () => {
    const viewer = fakeViewer();
    const frames = manualFrames();
    const handle = mountVisualEffects(viewer as any, frames.deps);
    handle.setStyle("retro"); // STYLE_PRESET_DEFAULTS has retro
    frames.tick(600);
    const bloom = viewer.scene.postProcessStages.bloom;
    const [sharpen] = viewer.scene.postProcessStages.byName("godsEyeView_sharpen");
    // REAL retro preset default is bloom {enabled:false} (visualPresets.js:70-72),
    // NOT enabled:true. The fake bloom starts at enabled:true, so the flip to
    // false is what proves the preset was actually applied (not left untouched).
    expect(bloom.enabled).toBe(false);
    // Sharpen is the second half of the preset: retro default is
    // {enabled:true,intensity:49}. initPostProcess leaves the sharpen stage
    // disabled, so enabled:true here also proves the preset landed.
    expect(sharpen.enabled).toBe(true);
    handle.setStyle("anime"); // no upstream preset default → local fallback: bloom off
    frames.tick(600);
    expect(bloom.enabled).toBe(false);
    handle.destroy();
  });

  test("destroy removes all self-created stages and restores bloom snapshot; idempotent", () => {
    const viewer = fakeViewer();
    const frames = manualFrames();
    const originalContrast = viewer.scene.postProcessStages.bloom.uniforms.contrast;
    const handle = mountVisualEffects(viewer as any, frames.deps, "snow");
    expect(viewer.scene.postProcessStages.length).toBe(7);
    handle.destroy();
    expect(viewer.scene.postProcessStages.length).toBe(0);
    expect(viewer.scene.postProcessStages.bloom.uniforms.contrast).toBe(originalContrast);
    expect(() => handle.destroy()).not.toThrow();
  });
});
