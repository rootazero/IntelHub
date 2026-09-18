// Four-edge HUD frame for the GEV globe page (T8 skeleton).
//
// DOM contract (consumed by the engine bootstrap + T9 rail):
//   #cesiumContainer        — engine viewer mounts here (viewer.js)
//   #loading-screen>.loader-status — scene.js writes boot progress text
//   #data-toggles           — engine LayerPresentation mount point (hidden;
//                             IntelHub owns its own layer UI, but the element
//                             must exist for engine code that queries it)
//   [data-hud="top|left|right|bottom"] — four edge rails (T9 left, T10 right,
//                             T11 top + bottom bars)
import type { ReactNode } from "react";

export function HudFrame({
  top,
  left,
  right,
  bottom,
  children,
}: {
  /** T11 top bar content for the top edge slot. */
  top?: ReactNode;
  /** T9 layer-rail content for the left edge slot. */
  left?: ReactNode;
  /** T10 detail-panel content for the right edge slot. */
  right?: ReactNode;
  /** T11 bottom status-bar content for the bottom edge slot. */
  bottom?: ReactNode;
  children?: ReactNode;
}) {
  return (
    <div className="globe-root">
      <div id="cesiumContainer" className="hud-canvas" />
      <div id="loading-screen">
        <span className="loader-status" />
      </div>
      <div id="data-toggles" hidden />
      <div className="hud-edge hud-top" data-hud="top">
        {top}
      </div>
      <div className="hud-edge hud-left" data-hud="left">
        {left}
      </div>
      <div className="hud-edge hud-right" data-hud="right">
        {right}
      </div>
      <div className="hud-edge hud-bottom" data-hud="bottom">
        {bottom}
      </div>
      {children}
    </div>
  );
}
