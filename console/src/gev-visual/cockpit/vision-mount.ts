// GEV P9 cockpit vision adapter — bridges cockpitVisionPolicy's stage-intensity
// gating onto the P6 visual-effects handle (D2).
//
// THE HIGHEST-RISK BRIDGE: `mountVisualEffects.getStages()` returns a
// `Map<string, PostProcessStage>`. The vendor's
// `applyCockpitVisionStageIntensities(stages, mode, restore?)` consumes a PLAIN
// object — it walks stages with `Object.entries(stages)` /
// `Object.values(stages)` and indexes `stages[target]`, all of which are NO-OPS
// on a Map (a Map has no enumerable own properties). Without converting the Map
// to a record via `Object.fromEntries(map.entries())`, vision mode is a silent
// no-op. That conversion is the single line this adapter exists to get right.
//
// `TARGET_STYLE_BY_MODE` is module-PRIVATE (cockpitVisionPolicy.js), so the
// mode→globe-style mapping is hardcoded here (crt→retro, nvg→surveillance,
// thermal→thermal, noir→noir; optical has no style — restore baseline). The
// intensity gating itself is done by `applyCockpitVisionStageIntensities`,
// which owns the private mapping internally — never hand-rolled on our side.
import {
  normalizeCockpitVisionMode,
  captureCockpitVisionBaseline,
  applyCockpitVisionStageIntensities,
} from "gev-engine/src/cockpitVisionPolicy.js";
import type { PostProcessStage } from "cesium";
import type { GlobeStyle, VisualEffectsHandle } from "../visual-effects";

export const VISION_MODES = [
  "optical",
  "crt",
  "nvg",
  "thermal",
  "noir",
] as const;
export type VisionMode = (typeof VISION_MODES)[number];

/** localStorage key holding the user's last vision pick (spec §3.2). Exported
 *  so every writer/reader (HudCockpitVisionSwitch, the T5 shortcut hook)
 *  shares one literal instead of re-typing it. */
export const VISION_MODE_STORAGE_KEY = "intelhub.cockpit.visionMode";

export function isVisionMode(value: unknown): value is VisionMode {
  return (
    typeof value === "string" &&
    (VISION_MODES as readonly string[]).includes(value)
  );
}

/** Mirror of the vendor's private TARGET_STYLE_BY_MODE (4 non-optical modes). */
const VISION_STYLE_BY_MODE: Record<Exclude<VisionMode, "optical">, GlobeStyle> =
  {
    crt: "retro",
    nvg: "surveillance",
    thermal: "thermal",
    noir: "noir",
  };

type StageRecord = Record<string, PostProcessStage>;

export interface VisionMountHandle {
  /** Normalize + apply the vision mode. Returns the normalized mode. */
  setMode(mode: VisionMode | string): VisionMode;
  getMode(): VisionMode;
  destroy(): void;
}

export function mountCockpitVision(
  effects: VisualEffectsHandle,
): VisionMountHandle {
  // Constructor contract (P3 lesson): a lenient mock must not hide a handle
  // that lacks the two seams vision mode depends on.
  if (
    !effects ||
    typeof effects.getStages !== "function" ||
    typeof effects.setStyle !== "function"
  ) {
    throw new TypeError(
      "mountCockpitVision: effects must provide getStages() and setStyle()",
    );
  }

  let mode: VisionMode = "optical";
  let baseline: Record<string, number> | null = null;
  let destroyed = false;

  /** Map → plain record (the bridge). getStages() returns a Map keyed by bare
   *  style name (retro/surveillance/thermal/anime/noir/snow). */
  const recordOf = (): StageRecord | null => {
    const stages = effects.getStages();
    return stages ? (Object.fromEntries(stages.entries()) as StageRecord) : null;
  };

  return {
    setMode(next) {
      const normalized = normalizeCockpitVisionMode(next) as VisionMode;
      const record = recordOf();
      if (record) {
        // Capture the pre-cockpit intensities once, on first non-optical entry.
        if (mode === "optical" && normalized !== "optical") {
          baseline = captureCockpitVisionBaseline(record, null as never);
        }
        const style = VISION_STYLE_BY_MODE[
          normalized as Exclude<VisionMode, "optical">
        ];
        if (style) {
          effects.setStyle(style);
        } else if (normalized === "optical") {
          effects.setStyle("normal");
        }
        // The intensity gate: zeroes every stage, lights exactly the target
        // style (or restores `baseline` for optical). Returns the target style
        // name or null — the mapping lives here, not in the adapter.
        applyCockpitVisionStageIntensities(record, normalized, baseline ?? {});
        if (normalized === "optical") baseline = null;
      }
      mode = normalized;
      return normalized;
    },
    getMode: () => mode,
    destroy() {
      if (destroyed) return;
      destroyed = true;
      baseline = null;
    },
  };
}
