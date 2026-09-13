// SP10 EntityPanel — shown when a node is selected. Loads timeline + evidence.
// Side-panel-only component; mounted by SidePanel.

import { useEffect, useState } from "react";
import {
  getEntityTimeline,
  getEvidenceForEntity,
  type EntitySummary,
  type TimelineEntry,
} from "../api/graph";
import { kindColor } from "../kindmeta";

interface Props {
  entity: { entity_id: string; name: string; kind: string };
  /** All neighbors + their edges so we can show this entity's full degree list. */
  context: EntitySummary[];
  onClose: () => void;
  onPickNeighbor: (id: string) => void;
}

export default function EntityPanel({ entity, context, onClose, onPickNeighbor }: Props) {
  const [timeline, setTimeline] = useState<TimelineEntry[] | null>(null);
  const [evidence, setEvidence] = useState<unknown[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  // SP10: every time the user picks a different entity, fetch its details.
  useEffect(() => {
    let cancelled = false;
    setTimeline(null);
    setEvidence(null);
    setError(null);
    (async () => {
      try {
        const [tl, ev] = await Promise.all([
          getEntityTimeline(entity.entity_id),
          getEvidenceForEntity(entity.entity_id),
        ]);
        if (cancelled) return;
        setTimeline(tl);
        setEvidence(ev);
      } catch (e) {
        if (!cancelled) setError(e instanceof Error ? e.message : String(e));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [entity.entity_id]);

  // Connections list = incidents on this entity across the full neighbors payload.
  const connections = context.filter((e) =>
    e.edges.some((ed) => ed.source_id === entity.entity_id || ed.target_id === entity.entity_id),
  );

  return (
    <div className="border border-edge bg-base p-2 text-xs">
      <div className="flex items-start justify-between mb-2">
        <div>
          <p className="font-bold text-sm" style={{ color: kindColor(entity.kind) }}>
            {entity.name}
          </p>
          <p className="text-dim">
            <span
              className="inline-block px-1 py-0.5 mr-1 text-[10px]"
              style={{ background: kindColor(entity.kind), color: "#000" }}
            >
              {entity.kind}
            </span>
            <span className="mono">{entity.entity_id.slice(0, 8)}…</span>
          </p>
        </div>
        <button type="button" onClick={onClose} className="text-dim hover:text-white px-1">
          ×
        </button>
      </div>

      <div className="mb-3">
        <p className="text-dim uppercase mb-1">connections ({connections.length})</p>
        {connections.length === 0 ? (
          <p className="text-dim italic">no incident edges in current view</p>
        ) : (
          <ul className="max-h-32 overflow-y-auto">
            {connections.map((c) => (
              <li key={c.entity_id}>
                <button
                  type="button"
                  onClick={() => onPickNeighbor(c.entity_id)}
                  className="block w-full text-left px-1 py-0.5 hover:bg-edge truncate"
                >
                  <span style={{ color: kindColor(c.kind) }}>●</span> {c.name}{" "}
                  <span className="text-dim">({c.kind})</span>
                </button>
              </li>
            ))}
          </ul>
        )}
      </div>

      <div className="mb-3">
        <p className="text-dim uppercase mb-1">timeline ({timeline ? timeline.length : "…"})</p>
        {timeline === null ? (
          <p className="text-dim">loading…</p>
        ) : timeline.length === 0 ? (
          <p className="text-dim italic">no events</p>
        ) : (
          <ul className="max-h-40 overflow-y-auto">
            {timeline.slice(0, 30).map((t, i) => (
              <li key={i} className="border-l border-edge pl-2 mb-1">
                <span className="mono text-dim text-[10px]">
                  {String(t.occurred_at ?? t.event_id ?? i)}
                </span>
                <pre className="whitespace-pre-wrap break-words text-[10px]">
                  {JSON.stringify(t, null, 0).slice(0, 220)}
                </pre>
              </li>
            ))}
          </ul>
        )}
      </div>

      <div>
        <p className="text-dim uppercase mb-1">evidence ({evidence ? evidence.length : "…"})</p>
        {evidence === null ? (
          <p className="text-dim">loading…</p>
        ) : evidence.length === 0 ? (
          <p className="text-dim italic">no supporting documents</p>
        ) : (
          <ul className="max-h-32 overflow-y-auto">
            {evidence.map((ev, i) => {
              const e = ev as {
                document_id?: string;
                base_url?: string;
                retrieved_at?: string;
                relation?: string;
              };
              return (
                <li key={i} className="mb-1">
                  <span className="mono text-[10px]">{e.document_id?.slice(0, 8) ?? "—"}</span>
                  {" · "}
                  <span className="text-dim">{e.relation ?? "—"}</span>
                  {e.base_url && (
                    <a
                      href={e.base_url}
                      target="_blank"
                      rel="noopener noreferrer"
                      className="block truncate text-[10px] text-blue-400 hover:underline"
                    >
                      {e.base_url}
                    </a>
                  )}
                </li>
              );
            })}
          </ul>
        )}
      </div>

      {error && <p className="mt-2 text-red-500 text-[10px]">{error}</p>}
    </div>
  );
}