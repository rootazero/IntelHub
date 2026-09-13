// SP10 EdgePanel — shown when an edge is selected. Resolves source/target
// names from the current neighbors context and lists evidence rows.

import { useEffect, useState } from "react";
import { getEvidenceForEntity, type EntitySummary } from "../api/graph";

export interface EdgeSelection {
  key: string;
  source: string;
  target: string;
  rel_type: string;
  confidence: number | null;
}

interface Props {
  edge: EdgeSelection;
  context: EntitySummary[];
  onClose: () => void;
}

function confBar(c: number | null): { pct: number; label: string; color: string } {
  if (c == null) return { pct: 100, label: "n/a", color: "#9aa4b2" };
  const pct = Math.round(Math.max(0, Math.min(1, c)) * 100);
  let color = "#34d399";
  if (c < 0.5) color = "#ff3355";
  else if (c < 0.8) color = "#ffd60a";
  return { pct, label: c.toFixed(2), color };
}

export default function EdgePanel({ edge, context, onClose }: Props) {
  const [evidence, setEvidence] = useState<unknown[] | null>(null);

  useEffect(() => {
    let cancelled = false;
    setEvidence(null);
    (async () => {
      try {
        // Edges bind evidence to their source endpoint by convention.
        const rows = await getEvidenceForEntity(edge.source, edge.rel_type);
        if (!cancelled) setEvidence(rows);
      } catch {
        if (!cancelled) setEvidence([]);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [edge.source, edge.rel_type]);

  const sourceName =
    context.find((e) => e.entity_id === edge.source)?.name ?? edge.source.slice(0, 8);
  const targetName =
    context.find((e) => e.entity_id === edge.target)?.name ?? edge.target.slice(0, 8);

  const conf = confBar(edge.confidence);

  return (
    <div className="border border-edge bg-base p-2 text-xs">
      <div className="flex items-start justify-between mb-2">
        <p className="font-bold text-sm">
          <span className="mono">{sourceName}</span>
          <span className="mx-1 text-yellow-400">→</span>
          <span className="text-yellow-400">{edge.rel_type}</span>
          <span className="mx-1">→</span>
          <span className="mono">{targetName}</span>
        </p>
        <button type="button" onClick={onClose} className="text-dim hover:text-white px-1">
          ×
        </button>
      </div>

      <div className="mb-3">
        <p className="text-dim uppercase mb-1">confidence</p>
        <div className="flex items-center gap-2">
          <div className="flex-1 h-2 bg-edge">
            <div style={{ width: `${conf.pct}%`, height: "100%", background: conf.color }} />
          </div>
          <span className="mono w-10 text-right">{conf.label}</span>
        </div>
      </div>

      <div className="mb-2">
        <p className="text-dim text-[10px]">
          source <span className="mono">{edge.source.slice(0, 8)}…</span>
          <br />
          target <span className="mono">{edge.target.slice(0, 8)}…</span>
          <br />
          key <span className="mono">{edge.key.slice(0, 16)}…</span>
        </p>
      </div>

      <div>
        <p className="text-dim uppercase mb-1">evidence ({evidence ? evidence.length : "…"})</p>
        {evidence === null ? (
          <p className="text-dim">loading…</p>
        ) : evidence.length === 0 ? (
          <p className="text-dim italic">no supporting documents</p>
        ) : (
          <ul className="max-h-40 overflow-y-auto">
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
                  <span className="text-dim">{e.relation ?? edge.rel_type}</span>
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
    </div>
  );
}