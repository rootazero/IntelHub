// Entity detail: identity + relationships (§38 graph memory) + claims.

import { useCallback, useEffect, useState } from "react";
import { useParams } from "react-router-dom";
import { api } from "../api";
import { Empty, EpistemicBadge, ErrorBox, Loading, Panel, TimeAgo } from "../ui";

interface Entity {
  entity_id: string;
  kind: string;
  name: string;
  aliases: string[];
  attributes: Record<string, unknown>;
  created_by: string;
  created_at: string;
  relationships: { from: string; rel_type: string; to: string }[];
  claims: { claim_id: string; text: string; status: string; role: string }[];
}

export default function EntityDetail() {
  const { id } = useParams();
  const [entity, setEntity] = useState<Entity | null>(null);
  const [error, setError] = useState<Error | null>(null);

  const load = useCallback(() => {
    if (!id) return;
    api<Entity>(`/api/v1/entities/${id}`).then(setEntity).catch(setError);
  }, [id]);
  useEffect(load, [load]);

  if (error) return <div className="p-4"><ErrorBox error={error} /></div>;
  if (!entity) return <Loading />;

  return (
    <div className="space-y-3 p-4">
      <div className="rounded border border-edge bg-panel p-3">
        <div className="flex items-center gap-2">
          <EpistemicBadge kind="OBSERVATION" />
          <span className="rounded border border-accent/40 px-1.5 py-0 text-[10px] font-semibold uppercase text-accent">{entity.kind}</span>
          <h1 className="text-sm font-bold">{entity.name}</h1>
        </div>
        <div className="mt-1 text-[11px] text-dim">
          created by <span className="mono">{entity.created_by}</span> · <TimeAgo ts={entity.created_at} />
        </div>
        {entity.aliases.length > 0 && (
          <div className="mt-1 text-[11px]"><span className="text-dim">aliases: </span>{entity.aliases.join(", ")}</div>
        )}
        {Object.keys(entity.attributes).length > 0 && (
          <pre className="mt-2 overflow-x-auto rounded border border-edge bg-base p-2 mono text-[10px] text-dim">
            {JSON.stringify(entity.attributes, null, 2)}
          </pre>
        )}
      </div>

      <div className="grid grid-cols-1 gap-3 xl:grid-cols-2">
        <Panel title={`Relationships (${entity.relationships.length})`}>
          {entity.relationships.length === 0 ? <Empty label="no relationships" /> : (
            <ul className="space-y-1 mono text-[11px]">
              {entity.relationships.map((r, i) => (
                <li key={i}>{r.from} <span className="text-accent">—{r.rel_type}→</span> {r.to}</li>
              ))}
            </ul>
          )}
        </Panel>

        <Panel title={`Claims (${entity.claims.length})`}>
          {entity.claims.length === 0 ? <Empty label="no claims about this entity" /> : (
            <ul className="space-y-1.5 text-xs">
              {entity.claims.map((c) => (
                <li key={c.claim_id} className="flex items-start gap-2">
                  <EpistemicBadge kind="AGENT CLAIM" />
                  <span className="flex-1">{c.text}</span>
                  <span className="shrink-0 mono text-[10px] text-dim">{c.role} · {c.status}</span>
                </li>
              ))}
            </ul>
          )}
        </Panel>
      </div>
    </div>
  );
}
