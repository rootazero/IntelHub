// Four-edge HUD frame for the GEV globe page (T8 skeleton).
//
// DOM contract (consumed by the engine bootstrap + T9 rail):
//   #cesiumContainer        — engine viewer mounts here (viewer.js)
//   #loading-screen>.loader-status — scene.js writes boot progress text
//   #data-toggles           — engine LayerPresentation mount point (hidden;
//                             IntelHub owns its own layer UI, but the element
//                             must exist for engine code that queries it)
//   [data-hud="top|left|right|bottom"] — four edge rails (T9 fills them)
import type { ReactNode } from "react";

export function HudFrame({ children }: { children?: ReactNode }) {
  return (
    <div className="hud-root">
      <div id="cesiumContainer" className="hud-canvas" />
      <div id="loading-screen">
        <span className="loader-status" />
      </div>
      <div id="data-toggles" hidden />
      <div className="hud-edge hud-top" data-hud="top" />
      <div className="hud-edge hud-left" data-hud="left" />
      <div className="hud-edge hud-right" data-hud="right" />
      <div className="hud-edge hud-bottom" data-hud="bottom" />
      {children}
    </div>
  );
}
