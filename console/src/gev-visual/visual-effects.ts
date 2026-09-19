// P6 visual presets adapter — the ONLY seam between the vendored GEV
// post-process render core (VisualEffects + visualPresets) and the IntelHub
// React HUD. Vendor modules are imported read-only; all DOM/event wiring
// lives on the React side.
//
// Deliberately NOT used: VisualSettings (DOM/shell-coupled), renderGovernor
// (not installed in our boot — hold/release are noops), detection/scopeMask
// (removed from scope, see spec §0).
import { VisualEffects } from "gev-engine/src/ui/visualEffects.js";
import { STYLES, STYLE_PRESET_DEFAULTS } from "gev-engine/src/ui/visualPresets.js";
// Type-only: the vendor owns the runtime import (visualEffects.js:1). Cesium
// ships its typings at cesium/Source/Cesium.d.ts, so nothing heavier is pulled
// in than the class the vendor already uses.
import type { PostProcessStage } from "cesium";

export const GLOBE_STYLES = [
  "normal", "retro", "surveillance", "thermal", "anime", "noir", "snow",
] as const;
export type GlobeStyle = (typeof GLOBE_STYLES)[number];

export function isGlobeStyle(value: unknown): value is GlobeStyle {
  return typeof value === "string" && (GLOBE_STYLES as readonly string[]).includes(value);
}

// Upstream STYLE_PRESET_DEFAULTS only covers retro/surveillance/thermal.
// Local fallbacks for anime/noir/snow mirror GLOBAL_POST_DEFAULTS
// (sharpen on at 49%, bloom off) so switching never leaves stale bloom.
const LOCAL_PRESET_FALLBACKS: Record<
  string,
  { bloom: { enabled: boolean; intensity: number }; sharpen: { enabled: boolean; intensity: number } }
> = {
  anime: { bloom: { enabled: false, intensity: 0 }, sharpen: { enabled: true, intensity: 49 } },
  noir: { bloom: { enabled: false, intensity: 0 }, sharpen: { enabled: true, intensity: 49 } },
  snow: { bloom: { enabled: false, intensity: 0 }, sharpen: { enabled: true, intensity: 49 } },
  normal: { bloom: { enabled: false, intensity: 0 }, sharpen: { enabled: true, intensity: 49 } },
};

export interface ViewerLike {
  scene: {
    requestRender?: () => void;
    postProcessStages: {
      add(stage: unknown): void;
      remove(stage: unknown): void;
      bloom?: unknown;
    };
  };
}

export interface VisualEffectsDeps {
  createStage?: (options: unknown) => unknown;
  requestFrame?: (cb: (t: number) => void) => number;
  cancelFrame?: (id: number) => void;
  now?: () => number;
}

export interface VisualEffectsHandle {
  setStyle(style: GlobeStyle): void;
  getStyle(): GlobeStyle;
  /**
   * Snapshot of the vendor's style-name → PostProcessStage map (P9 cockpit
   * vision gating), or null once the mount is destroyed.
   *
   * NOTE: the vendor's `applyCockpitVisionStageIntensities(stages, mode,
   * restore?)` consumes a PLAIN style-name→stage object (`Object.entries` /
   * `Object.values` — cockpitVisionPolicy.js), not a Map. Bridge with
   * `Object.fromEntries(handle.getStages() ?? [])` before calling it.
   *
   * Reading `viewer.scene.postProcessStages` would be wrong here: that is the
   * Cesium collection, whose entries are named `godsEyeView_<style>` while the
   * vision policy looks stages up by bare style name.
   */
  getStages(): Map<string, PostProcessStage> | null;
  destroy(): void;
}

export function mountVisualEffects(
  viewer: ViewerLike,
  deps: VisualEffectsDeps = {},
  initialStyle: GlobeStyle = "normal",
): VisualEffectsHandle {
  if (
    !viewer?.scene?.postProcessStages ||
    typeof viewer.scene.postProcessStages.add !== "function" ||
    typeof viewer.scene.postProcessStages.remove !== "function"
  ) {
    throw new TypeError(
      "mountVisualEffects: viewer.scene.postProcessStages.{add,remove} missing",
    );
  }

  const effects = new VisualEffects({
    viewer,
    requestRender: () => viewer.scene.requestRender?.(),
    holdRender: () => {},
    releaseRender: () => {},
    ...(deps.createStage ? { createStage: deps.createStage } : {}),
    ...(deps.requestFrame ? { requestFrame: deps.requestFrame } : {}),
    ...(deps.cancelFrame ? { cancelFrame: deps.cancelFrame } : {}),
    ...(deps.now ? { now: deps.now } : {}),
  });
  effects.initStyles();
  effects.initPostProcess();

  let current: GlobeStyle = "normal";

  const stageOf = (name: string) => (effects as any).stages?.[name];

  function applyPresetDefaults(style: GlobeStyle): void {
    const preset =
      (STYLE_PRESET_DEFAULTS as Record<string, any>)[style] ?? LOCAL_PRESET_FALLBACKS[style];
    if (!preset) return;
    effects.setBloomEnabled(Boolean(preset.bloom?.enabled));
    if (preset.bloom?.enabled) effects.applyBloomIntensity(Number(preset.bloom.intensity ?? 0));
    effects.setSharpenEnabled(Boolean(preset.sharpen?.enabled));
    if (preset.sharpen?.enabled) {
      effects.applySharpenIntensity(Number(preset.sharpen.intensity ?? 49) / 100);
    }
  }

  function setStyle(next: GlobeStyle): void {
    if (next === current) return;
    const prev = current;
    current = next;
    if (prev !== "normal") {
      effects.startTransition(prev, stageOf(prev)?.uniforms?.intensity ?? 1.0, 0.0);
    }
    if (next !== "normal") {
      effects.startTransition(next, stageOf(next)?.uniforms?.intensity ?? 0.0, 1.0);
    }
    applyPresetDefaults(next);
  }

  // Preset defaults (bloom/sharpen) are the vendor baseline for EVERY style,
  // normal included (GLOBAL_POST_DEFAULTS = sharpen ON@49, bloom OFF). Apply
  // them unconditionally on mount, otherwise a fresh page load under "normal"
  // renders unsharpened while a style round-trip back to normal flips sharpen
  // ON. Only the instant stage-intensity apply stays gated: "normal" has no
  // style stage of its own.
  applyPresetDefaults(initialStyle);
  if (initialStyle !== "normal" && (STYLES as Record<string, unknown>)[initialStyle]) {
    const stage = stageOf(initialStyle);
    if (stage) effects.setStageIntensity(stage, 1.0);
    current = initialStyle;
  }

  let destroyed = false;
  return {
    setStyle,
    getStyle: () => current,
    getStages(): Map<string, PostProcessStage> | null {
      if (destroyed) return null;
      const stages = (effects as any).stages as
        | Record<string, PostProcessStage>
        | undefined;
      if (!stages || typeof stages !== "object") return null;
      return new Map(Object.entries(stages));
    },
    destroy() {
      if (destroyed) return;
      destroyed = true;
      effects.destroy();
    },
  };
}
