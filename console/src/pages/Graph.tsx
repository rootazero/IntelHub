// SP9 console /graph skeleton (Task 8). Minimal canvas: input → fetch
// neighbors → render with @antv/g6 v5. Filters, panels, and edge
// interactivity land in SP10.

import { useEffect, useRef, useState } from "react";
import { getNeighbors, type EntitySummary } from "../api/graph";

const UUID_RE = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

export default function GraphPage() {
  const containerRef = useRef<HTMLDivElement>(null);
  const [root_, setRoot] = useState<string>("");
  const [error, setError] = useState<string | null>(null);
  const [status, setStatus] = useState<string>("idle");

  useEffect(() => {
    if (!root_ || !UUID_RE.test(root_)) {
      setStatus(root_ ? "enter a valid uuid" : "idle");
      return;
    }
    let cancelled = false;
    setStatus("loading…");
    setError(null);

    (async () => {
      try {
        const data = await getNeighbors(root_, 2);
        if (cancelled || !containerRef.current) return;

        const G6 = await import("@antv/g6");
        const graph = new G6.Graph({
          container: containerRef.current,
          width: containerRef.current.clientWidth,
          height: containerRef.current.clientHeight,
          data: g6Data(data),
          node: { style: { labelText: (d: unknown) => (d as { name?: string }).name ?? "" } },
          edge: { style: { labelText: (d: unknown) => (d as { rel_type?: string }).rel_type ?? "" } },
          behaviors: ["drag-canvas", "zoom-canvas"],
        });
        await graph.render();
        if (cancelled) {
          graph.destroy();
          return;
        }
        setStatus(`${data.length} neighbor(s)`);
      } catch (e) {
        if (!cancelled) setError(e instanceof Error ? e.message : String(e));
        setStatus("error");
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [root_]);

  return (
    <div className="p-4">
      <h2 className="text-lg font-bold mb-2">Graph (skeleton)</h2>
      <div className="flex items-center text-sm">
        <input
          type="text"
          placeholder="entity_id (uuid)"
          className="border border-edge bg-base px-2 py-1 mr-2 mono text-xs"
          value={root_}
          onChange={(e) => setRoot(e.target.value.trim())}
        />
        <span className="text-xs text-dim">{status}</span>
      </div>
      <p className="mt-2 text-xs text-dim">minimal canvas — filters/panels in SP10</p>
      {error && <p className="mt-2 text-red-600 text-xs">{error}</p>}
      <div
        ref={containerRef}
        className="mt-4 border border-edge bg-base"
        style={{ height: 480, width: "100%" }}
      />
    </div>
  );
}

interface G6Node { id: string; name: string; kind: string; [k: string]: unknown }
interface G6Edge { source: string; target: string; rel_type: string; [k: string]: unknown }

function g6Data(data: EntitySummary[]) {
  // Backend returns one row per neighbor; the row's own `entity_id` is
  // the neighbor node and each `edges[]` entry describes a relationship
  // incident to that neighbor. Without source/target IDs on each edge
  // (future SP9 enrichment), connect each edge's endpoints to the
  // neighbor itself so the canvas isn't floating-point orphan-less.
  // The goal is "loads, shows something" per Task 8 brief; full
  // source/target resolution lands in SP10.
  const nodes = new Map<string, G6Node>();
  const edges: G6Edge[] = [];

  for (const e of data) {
    nodes.set(e.entity_id, { id: e.entity_id, name: e.name, kind: e.kind });
    for (const ed of e.edges || []) {
      if (!ed.rel_type) continue;
      edges.push({ source: e.entity_id, target: e.entity_id, rel_type: ed.rel_type });
    }
  }

  return { nodes: Array.from(nodes.values()), edges };
}