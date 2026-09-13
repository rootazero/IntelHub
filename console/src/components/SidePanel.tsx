// SP10 SidePanel — dispatcher that shows EntityPanel / EdgePanel based on
// selection, plus the empty-state hint when nothing is selected.

import type { EntitySummary } from "../api/graph";
import EntityPanel from "./EntityPanel";
import EdgePanel, { type EdgeSelection } from "./EdgePanel";

export type SidePanelSelection =
  | { kind: "node"; entityId: string }
  | { kind: "edge"; edge: EdgeSelection }
  | null;

interface Props {
  selection: SidePanelSelection;
  context: EntitySummary[];
  onPickNeighbor: (id: string) => void;
  onClose: () => void;
}

export default function SidePanel({ selection, context, onPickNeighbor, onClose }: Props) {
  if (!selection) {
    return (
      <div className="border border-edge bg-base p-2 text-xs text-dim italic">
        click a node or edge to inspect details, timeline, and evidence.
      </div>
    );
  }
  if (selection.kind === "node") {
    // First try the explicit root or a direct neighbor; fall back to a stub if not.
    const hit =
      context.find((c) => c.entity_id === selection.entityId) ??
      ({
        entity_id: selection.entityId,
        name: selection.entityId.slice(0, 8) + "…",
        kind: "other",
        edges: [],
      } as EntitySummary);
    return (
      <EntityPanel
        entity={{ entity_id: hit.entity_id, name: hit.name, kind: hit.kind }}
        context={context}
        onClose={onClose}
        onPickNeighbor={onPickNeighbor}
      />
    );
  }
  return <EdgePanel edge={selection.edge} context={context} onClose={onClose} />;
}