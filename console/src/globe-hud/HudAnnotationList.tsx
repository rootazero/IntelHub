// GEV P8 — saved-annotation list overlay. Pure presentational: renders the
// latest annotations (capped at 10), routes row-click to onSelect and the
// delete button to onDelete. The `viewer` prop is accepted but unused today —
// it is the reserved hook for P9/P10 flyTo framing (centroid + bbox radius);
// the onSelect flight lives in GlobeV2's handler, not here.
import type { AnnotationSpec } from "../gev-visual/annotations";

interface HudAnnotationListProps {
  viewer: unknown;
  annotations: ReadonlyArray<AnnotationSpec>;
  onSelect: (spec: AnnotationSpec) => void;
  onDelete: (id: string) => void;
  visible: boolean;
}

export function HudAnnotationList({
  annotations,
  onSelect,
  onDelete,
  visible,
}: HudAnnotationListProps) {
  if (!visible) return null;
  return (
    <div
      data-testid="hud-annotation-list"
      className="hud-annotation-list"
    >
      {annotations.slice(0, 10).map((a) => (
        <div
          key={a.id}
          data-testid="hud-annotation-row"
          className="hud-annotation-row"
        >
          <span
            className="hud-annotation-label"
            onClick={() => onSelect(a)}
          >
            {a.shape === "pin" ? "📍" : a.shape === "line" ? "〰️" : "⬡"}{" "}
            {a.label ?? "(未命名)"}
          </span>
          <button
            type="button"
            data-testid="hud-annotation-delete"
            aria-label={`删除 ${a.label ?? "(未命名)"}`}
            onClick={() => onDelete(a.id)}
          >
            ×
          </button>
        </div>
      ))}
    </div>
  );
}
