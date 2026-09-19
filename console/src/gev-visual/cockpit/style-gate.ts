// GEV P9 R5 — globe style picker gate.
//
// The cockpit vision policy (`vision-mount.setMode`) writes stage intensities
// through the SAME `mountVisualEffects` handle the P6 globe style picker
// (`HudStyleSwitcher`) drives. Both paths own `PostProcessStage.uniforms.
// intensity`: vision gating zeroes every stage and lights exactly the
// mode-mapped style (crt→retro …), while the picker crossfades the user's
// chosen style back in. Without a gate the two fight every frame.
//
// Fix (D2/R5): wrap the picker's setStyle so it is a NO-OP while cockpit is
// active, recording the user's pick so it can be replayed when the cockpit is
// exited. The cockpit's own vision switching calls the RAW effects handle, so
// it is never gated.
import type { GlobeStyle } from "../visual-effects";
import type { CockpitStore } from "./cockpit-store";

/** The slice of the P6 visual-effects handle the gate needs. */
export interface StyleEffects {
  setStyle(style: GlobeStyle): void;
  getStyle(): GlobeStyle;
}

export interface GatedStyleControl {
  /** Record the user pick; apply only while cockpit is INACTIVE (R5). */
  setStyle(style: GlobeStyle): void;
  /** Replay the last user-picked style (cockpit-exit restore path). */
  restorePicked(): void;
  /** The user's latest pick — even while gated (never reflects vision mode). */
  getPicked(): GlobeStyle;
}

export function gateStyleWhileCockpitActive(
  effects: StyleEffects,
  store: Pick<CockpitStore, "getState">,
): GatedStyleControl {
  // Seed from the live handle so a page that already applied a persisted style
  // restores that same style (not "normal") on the first cockpit exit.
  let picked: GlobeStyle = effects.getStyle();

  return {
    setStyle(style) {
      picked = style;
      // R5: while cockpit is active the vision policy owns the stages. The
      // user's pick is recorded but NOT applied until cockpit exit.
      if (store.getState().active) return;
      effects.setStyle(style);
    },
    restorePicked() {
      effects.setStyle(picked);
    },
    getPicked: () => picked,
  };
}
