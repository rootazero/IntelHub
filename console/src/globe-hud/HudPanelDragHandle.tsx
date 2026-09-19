// GEV P10 T3 — visual drag handle for a HUD panel.
//
// The actual drag logic lives in the panel-drag adapter
// (`gev-visual/tail/panel-drag.ts` → `mountPanelDrag` wraps the vendored
// `PanelPositionControls`). This component is the PRESENTATION handle — a
// grip-icon span the operator clicks/drags. GlobeV2 owns the drag wiring
// (mouse listeners go on the parent panel; the handle is the affordance).
//
// testid: `hud-panel-drag-handle` (brief §testids).
// data-panelId: lets the parent identify which panel started the drag.
import { type PointerEvent as ReactPointerEvent, useCallback } from "react";

export interface HudPanelDragHandleProps {
  panelId: string;
  /** Called when a drag starts (pointerdown). */
  onDragStart?: (panelId: string, event: ReactPointerEvent) => void;
  /** Called when a drag ends (pointerup). */
  onDragEnd?: (panelId: string, event: ReactPointerEvent) => void;
  /** Whether the panel is currently collapsed (handle shrinks). */
  collapsed?: boolean;
  /** Override the grip glyph (rare). */
  glyph?: string;
  /** Override the localized tooltip text. */
  title?: string;
  /** Extra className applied to the outer span (rare; lets the parent
   *  theme the handle without rewriting CSS). */
  className?: string;
}

export function HudPanelDragHandle({
  panelId,
  onDragStart,
  onDragEnd,
  collapsed,
  glyph = "⋮⋮",
  title = "拖拽面板 / Drag panel",
  className,
}: HudPanelDragHandleProps) {
  const handleStart = useCallback(
    (event: ReactPointerEvent<HTMLSpanElement>) => {
      onDragStart?.(panelId, event);
    },
    [panelId, onDragStart],
  );
  const handleEnd = useCallback(
    (event: ReactPointerEvent<HTMLSpanElement>) => {
      onDragEnd?.(panelId, event);
    },
    [panelId, onDragEnd],
  );

  return (
    <span
      className={`hud-panel-drag-handle${collapsed ? " collapsed" : ""}${
        className ? ` ${className}` : ""
      }`}
      data-testid="hud-panel-drag-handle"
      data-panel-id={panelId}
      role="button"
      aria-label={title}
      title={title}
      onPointerDown={handleStart}
      onPointerUp={handleEnd}
    >
      <span aria-hidden>{glyph}</span>
    </span>
  );
}