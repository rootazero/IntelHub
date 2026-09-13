// SP9 console /graph page — entity picker + neighbor expansion.
// Replaces the SP9 skeleton (manual UUID input + empty canvas) with:
//   - landing view: most-recently-created entities (auto-loaded)
//   - search box: alias-aware entity search via REST
//   - click entity → fetch neighbors → render with @antv/g6 v5
// Filters, side panels, edge interaction land in SP10.

import { useEffect, useMemo, useRef, useState } from "react";
import {
  getNeighbors,
  listEntities,
  type EntitySearchHit,
  type EntitySummary,
} from "../api/graph";

const UUID_RE = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

export default function GraphPage() {
  const containerRef = useRef<HTMLDivElement>(null);
  const [search, setSearch] = useState<string>("");
  const [entities, setEntities] = useState<EntitySearchHit[]>([]);
  const [entitiesStatus, setEntitiesStatus] = useState<string>("loading…");
  const [selected, setSelected] = useState<string | null>(null);
  const [neighbors, setNeighbors] = useState<EntitySummary[] | null>(null);
  const [neighborsStatus, setNeighborsStatus] = useState<string>("idle");
  const [error, setError] = useState<string | null>(null);

  // Load entity list (default landing view, or search results).
  useEffect(() => {
    let cancelled = false;
    setEntitiesStatus("loading…");
    (async () => {
      try {
        const rows = await listEntities({
          q: search.trim() || undefined,
          limit: 100,
        });
        if (cancelled) return;
        setEntities(rows);
        setEntitiesStatus(`${rows.length} entit${rows.length === 1 ? "y" : "ies"}`);
      } catch (e) {
        if (!cancelled) {
          setEntitiesStatus("error");
          setError(e instanceof Error ? e.message : String(e));
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [search]);

  // Group entities by kind for the picker sidebar.
  const grouped = useMemo(() => {
    const m = new Map<string, EntitySearchHit[]>();
    for (const e of entities) {
      const k = e.kind || "other";
      const arr = m.get(k) ?? [];
      arr.push(e);
      m.set(k, arr);
    }
    return Array.from(m.entries()).sort((a, b) => a[0].localeCompare(b[0]));
  }, [entities]);

  // Render the neighbor canvas when an entity is selected.
  useEffect(() => {
    if (!selected || !UUID_RE.test(selected)) {
      setNeighbors(null);
      setNeighborsStatus(selected ? "enter a valid uuid" : "idle");
      return;
    }
    let cancelled = false;
    setNeighborsStatus("loading…");
    setError(null);
    (async () => {
      try {
        const data = await getNeighbors(selected, 2);
        if (cancelled || !containerRef.current) return;
        const G6 = await import("@antv/g6");
        const graph = new G6.Graph({
          container: containerRef.current,
          width: containerRef.current.clientWidth,
          height: containerRef.current.clientHeight,
          data: g6Data(selected, data),
          node: {
            style: { labelText: (d: unknown) => (d as { name?: string }).name ?? "" },
          },
          edge: {
            style: { labelText: (d: unknown) => (d as { rel_type?: string }).rel_type ?? "" },
          },
          behaviors: ["drag-canvas", "zoom-canvas"],
        });
        await graph.render();
        if (cancelled) {
          graph.destroy();
          return;
        }
        setNeighbors(data);
        setNeighborsStatus(`${data.length} neighbor${data.length === 1 ? "" : "s"}`);
      } catch (e) {
        if (!cancelled) {
          setError(e instanceof Error ? e.message : String(e));
          setNeighborsStatus("error");
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [selected]);

  return (
    <div className="p-4">
      <h2 className="text-lg font-bold mb-2">Graph</h2>

      <div className="grid grid-cols-[320px_1fr] gap-4">
        {/* LEFT: entity picker */}
        <aside className="border border-edge bg-base p-2 overflow-y-auto" style={{ maxHeight: 600 }}>
          <input
            type="text"
            placeholder="search entities (name / alias)…"
            className="border border-edge bg-base px-2 py-1 mb-2 mono text-xs w-full"
            value={search}
            onChange={(e) => setSearch(e.target.value)}
          />
          <p className="text-xs text-dim mb-2">{entitiesStatus}</p>
          {grouped.map(([kind, rows]) => (
            <div key={kind} className="mb-3">
              <p className="text-xs uppercase text-dim mb-1">{kind} ({rows.length})</p>
              <ul className="text-xs">
                {rows.map((e) => (
                  <li key={e.entity_id}>
                    <button
                      type="button"
                      onClick={() => setSelected(e.entity_id)}
                      className={
                        "block w-full text-left px-1 py-0.5 hover:bg-edge " +
                        (selected === e.entity_id ? "bg-edge font-bold" : "")
                      }
                    >
                      {e.name}
                    </button>
                  </li>
                ))}
              </ul>
            </div>
          ))}
        </aside>

        {/* RIGHT: neighbor canvas + status */}
        <section>
          <p className="text-xs text-dim mb-1">
            {selected
              ? `entity ${selected.slice(0, 8)}… · ${neighborsStatus}`
              : "pick an entity on the left to expand its neighbors"}
          </p>
          {error && <p className="mt-1 text-red-600 text-xs">{error}</p>}
          <div
            ref={containerRef}
            className="mt-1 border border-edge bg-base"
            style={{ height: 480, width: "100%" }}
          />
          <p className="mt-2 text-xs text-dim">
            minimal canvas — filters / side panels / edge interactivity land in SP10
          </p>
        </section>
      </div>
    </div>
  );
}

interface G6Node {
  id: string;
  name: string;
  kind: string;
  [k: string]: unknown;
}
interface G6Edge {
  source: string;
  target: string;
  rel_type: string;
  [k: string]: unknown;
}

function g6Data(root: string, data: EntitySummary[]) {
  // Backend returns one row per neighbor. Each row's `entity_id` is the
  // neighbor node and `edges[]` describes relationships incident to that
  // neighbor. SP10 enriches edges with explicit source/target IDs; for
  // now we connect every incident edge to (root → neighbor) so the
  // canvas is not floating-point orphan-less.
  const nodes = new Map<string, G6Node>();
  nodes.set(root, { id: root, name: "root", kind: "selected" });
  const edges: G6Edge[] = [];

  for (const e of data) {
    nodes.set(e.entity_id, { id: e.entity_id, name: e.name, kind: e.kind });
    for (const ed of e.edges || []) {
      if (!ed.rel_type) continue;
      edges.push({ source: root, target: e.entity_id, rel_type: ed.rel_type });
    }
  }

  return { nodes: Array.from(nodes.values()), edges };
}
