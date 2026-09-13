// SP10 console /graph page — full canvas.
// Replaces the SP9 minimal canvas with:
//   - filter bar (kind / rel_type / confidence / time)
//   - G6 nodes colored by KIND_COLORS, sized by degree
//   - edges with real source_id/target_id (multi-hop, no star topology)
//   - side panel showing node details + timeline + evidence OR edge details + evidence
//
// Selection state (node/edge) is lifted here so filter chips and the canvas
// can both read it without re-fetching.

import { useCallback, useEffect, useMemo, useState } from "react";
import {
  getNeighbors,
  listEntities,
  type EntitySearchHit,
  type EntitySummary,
} from "../api/graph";
import GraphCanvas, {
  type G6EdgeData,
  type G6NodeData,
} from "../components/GraphCanvas";
import FilterBar, { emptyFilter, type FilterState } from "../components/FilterBar";
import SidePanel, { type SidePanelSelection } from "../components/SidePanel";

const UUID_RE = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

export default function GraphPage() {
  // ---------- picker state (unchanged from SP9) ----------
  const [search, setSearch] = useState<string>("");
  const [entities, setEntities] = useState<EntitySearchHit[]>([]);
  const [entitiesStatus, setEntitiesStatus] = useState<string>("loading…");

  // ---------- canvas state ----------
  // ?root=<uuid> URL param (2026-09-14): lets users share/deep-link a graph
  // view — and lets the headless probe drive a known Neo4j entity whose
  // name never appears in the PG-backed picker (e.g. SP9 Neo4j-only seeds).
  const [selectedRoot, setSelectedRoot] = useState<string | null>(() => {
    if (typeof window === "undefined") return null;
    const q = new URLSearchParams(window.location.search).get("root");
    return q && UUID_RE.test(q) ? q : null;
  });
  const [neighbors, setNeighbors] = useState<EntitySummary[] | null>(null);
  const [neighborsStatus, setNeighborsStatus] = useState<string>("idle");

  // ---------- filter state ----------
  const [filter, setFilter] = useState<FilterState>(emptyFilter());

  // ---------- side-panel state ----------
  const [selection, setSelection] = useState<SidePanelSelection>(null);

  // ---------- shared error ----------
  const [error, setError] = useState<string | null>(null);

  // ----- picker: load entity list ---------------------------------------
  useEffect(() => {
    let cancelled = false;
    setEntitiesStatus("loading…");
    (async () => {
      try {
        const rows = await listEntities({ q: search.trim() || undefined, limit: 100 });
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

  // Group picker items by kind for the sidebar.
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

  // ----- canvas: fetch neighbors on root / filter change -----------------
  useEffect(() => {
    if (!selectedRoot || !UUID_RE.test(selectedRoot)) {
      setNeighbors(null);
      setNeighborsStatus(selectedRoot ? "enter a valid uuid" : "idle");
      return;
    }
    let cancelled = false;
    setNeighborsStatus("loading…");
    setError(null);

    // Compose backend filter args from current filter state.
    const atTime = (() => {
      if (!filter.from && !filter.to) return undefined;
      // Combine into an RFC3339 instant — backend treats it as the
      // "currently true at" anchor. From-only / To-only are passed as-is.
      if (filter.from && filter.to) {
        return `${filter.to}T23:59:59Z`;
      }
      if (filter.to) return `${filter.to}T23:59:59Z`;
      if (filter.from) return `${filter.from}T00:00:00Z`;
      return undefined;
    })();

    (async () => {
      try {
        const data = await getNeighbors(selectedRoot, 2, {
          atTime,
          relTypes: filter.relTypes.size > 0 ? Array.from(filter.relTypes) : undefined,
          minConfidence: filter.minConfidence > 0 ? filter.minConfidence : undefined,
        });
        if (cancelled) return;
        setNeighbors(data);
        setNeighborsStatus(`${data.length} neighbor${data.length === 1 ? "" : "s"}`);
        // Reset side-panel selection on data change (stale IDs would point to nothing).
        setSelection(null);
      } catch (e) {
        if (!cancelled) {
          setNeighborsStatus("error");
          setError(e instanceof Error ? e.message : String(e));
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [selectedRoot, filter.relTypes, filter.minConfidence, filter.from, filter.to]);

  // ----- derived G6 data -------------------------------------------------
  // Apply client-side kind filter; rel_type/confidence already filtered server-side.
  const filteredNeighbors = useMemo(() => {
    if (!neighbors) return [];
    if (filter.kinds.size === 0) return neighbors;
    return neighbors.filter((n) => filter.kinds.has(n.kind));
  }, [neighbors, filter.kinds]);

  const { g6Nodes, g6Edges } = useMemo(() => {
    const nodes = new Map<string, G6NodeData>();
    const edgesMap = new Map<string, G6EdgeData>();
    // Root label: prefer the picker's real entity name (PG-backed list),
    // fall back to the uuid prefix (Neo4j-only entities, deep links).
    const rootName =
      entities.find((e) => e.entity_id === selectedRoot)?.name ??
      (selectedRoot ? `${selectedRoot.slice(0, 8)}…` : "root");
    nodes.set(selectedRoot ?? "_", {
      id: selectedRoot ?? "_",
      name: rootName,
      kind: "selected",
      degree: 0,
    });
    let degreeById = new Map<string, number>();
    for (const n of filteredNeighbors) {
      degreeById.set(n.entity_id, n.edges.length);
      nodes.set(n.entity_id, {
        id: n.entity_id,
        name: n.name,
        kind: n.kind,
        degree: n.edges.length,
      });
      for (const ed of n.edges) {
        if (!ed.rel_type || !ed.source_id || !ed.target_id) continue;
        // SP10: real source→target from backend. Both endpoints must be in
        // the current node set (or be the root) for the edge to render.
        const key = `${ed.source_id}->${ed.target_id}:${ed.rel_type}`;
        if (edgesMap.has(key)) continue;
        const srcKnown = nodes.has(ed.source_id);
        const tgtKnown = nodes.has(ed.target_id);
        if (!srcKnown || !tgtKnown) continue;
        edgesMap.set(key, {
          key,
          source: ed.source_id,
          target: ed.target_id,
          rel_type: ed.rel_type,
          confidence: ed.confidence ?? null,
        });
      }
    }
    // Make sure the root's degree reflects how many incident edges survive.
    const rootDegree = Array.from(edgesMap.values()).filter(
      (e) => e.source === selectedRoot || e.target === selectedRoot,
    ).length;
    if (selectedRoot && nodes.has(selectedRoot)) {
      nodes.set(selectedRoot, {
        ...nodes.get(selectedRoot)!,
        degree: rootDegree,
      });
    }
    return { g6Nodes: Array.from(nodes.values()), g6Edges: Array.from(edgesMap.values()) };
  }, [filteredNeighbors, selectedRoot, entities]);

  // All rel_types observed in the *current* (server-filtered) neighbors
  // payload — feeds the FilterBar's relationship chip row.
  const availableRelTypes = useMemo(() => {
    if (!neighbors) return [];
    const s = new Set<string>();
    for (const n of neighbors) for (const e of n.edges) s.add(e.rel_type);
    return Array.from(s).sort();
  }, [neighbors]);

  // ----- selection callbacks (stable identity for G6 effect deps) --------
  const onSelectNode = useCallback((id: string | null) => {
    if (!id) {
      setSelection(null);
      return;
    }
    setSelection({ kind: "node", entityId: id });
  }, []);
  const onSelectEdge = useCallback((key: string | null) => {
    if (!key) {
      setSelection(null);
      return;
    }
    setSelection((prev) => {
      // Find the edge payload for this key from current neighbors.
      for (const n of neighbors ?? []) {
        for (const ed of n.edges) {
          if (`${ed.source_id}->${ed.target_id}:${ed.rel_type}` === key) {
            return {
              kind: "edge",
              edge: {
                key,
                source: ed.source_id,
                target: ed.target_id,
                rel_type: ed.rel_type,
                confidence: ed.confidence ?? null,
              },
            };
          }
        }
      }
      return prev;
    });
  }, [neighbors]);

  const onClosePanel = useCallback(() => setSelection(null), []);

  const onPickNeighbor = useCallback(
    (id: string) => {
      // If the picked neighbor is the current root, just stay; otherwise jump to it.
      if (id === selectedRoot) return;
      // We don't have a fetched subgraph for the new root yet — but listEntities
      // already loaded it (it's in the picker). Switch root and let the fetch
      // effect pick it up.
      setSelectedRoot(id);
    },
    [selectedRoot],
  );

  return (
    <div className="p-4">
      <h2 className="text-lg font-bold mb-2">Graph</h2>

      <div className="grid grid-cols-[280px_1fr_320px] gap-4">
        {/* LEFT: entity picker */}
        <aside
          className="border border-edge bg-base p-2 overflow-y-auto"
          style={{ maxHeight: 720 }}
        >
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
              <p className="text-xs uppercase text-dim mb-1">
                {kind} ({rows.length})
              </p>
              <ul className="text-xs">
                {rows.map((e) => (
                  <li key={e.entity_id}>
                    <button
                      type="button"
                      onClick={() => setSelectedRoot(e.entity_id)}
                      className={
                        "block w-full text-left px-1 py-0.5 hover:bg-edge " +
                        (selectedRoot === e.entity_id ? "bg-edge font-bold" : "")
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

        {/* CENTER: filter bar + canvas + status */}
        <section>
          <p className="text-xs text-dim mb-1">
            {selectedRoot
              ? `entity ${selectedRoot.slice(0, 8)}… · ${neighborsStatus}`
              : "pick an entity on the left to expand its neighbors"}
          </p>
          <FilterBar
            value={filter}
            onChange={setFilter}
            availableRelTypes={availableRelTypes}
            totalNodes={g6Nodes.length}
            totalEdges={g6Edges.length}
          />
          {error && <p className="mt-1 text-red-600 text-xs">{error}</p>}
          {selectedRoot ? (
            <GraphCanvas
              rootId={selectedRoot}
              nodes={g6Nodes}
              edges={g6Edges}
              onSelectNode={onSelectNode}
              onSelectEdge={onSelectEdge}
            />
          ) : (
            <div
              className="border border-edge bg-base flex items-center justify-center text-dim text-xs"
              style={{ height: 520, width: "100%" }}
            >
              select an entity on the left
            </div>
          )}
        </section>

        {/* RIGHT: side panel */}
        <aside style={{ maxHeight: 720, overflowY: "auto" }}>
          <SidePanel
            selection={selection}
            context={filteredNeighbors}
            onPickNeighbor={onPickNeighbor}
            onClose={onClosePanel}
          />
        </aside>
      </div>
    </div>
  );
}