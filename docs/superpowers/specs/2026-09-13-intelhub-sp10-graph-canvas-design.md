# SP10 — Graph Canvas (Complete Visualization Layer)

Date: 2026-09-13
Status: **DRAFT — ready for implementation**
Branch: `feat/sp10-graph-canvas`
Depends on: SP9 (KG memory layer, REST endpoints, G6 v5 already installed)

## 0. Background

SP9 shipped the data + MCP layer of the knowledge graph and a **skeleton console**
(`/graph` page with entity picker + minimal canvas). The skeleton explicitly
deferred filters, side panels, edge interaction, and richer visuals to SP10.

SP9 console today:
- G6 node style: `labelText: d.name` only — no color, no size, no image.
- Edges all hard-coded to `(root → neighbor)` (the comment in
  `Graph.tsx:188` says "SP10 enriches edges with explicit source/target IDs").
- 6 REST endpoints exist (`/api/v1/graph/{entities/search,neighbors,entity/{id}/timeline,path,investigation/{id},evidence}`).
- Backend `get_neighbors` already accepts `rel_types`/`min_confidence` but the
  REST route does not expose them (passes `None, None`).
- `kindmeta.ts` has `KIND_COLORS` (16-kind palette) and `sevRadius` but no
  consumer.

## 1. Scope (what "complete" means for the canvas)

| Feature | Why now |
|---|---|
| **Backend: enrich edges with `source_id`/`target_id`** | Lets the console draw multi-hop graphs (not just star topology). |
| **Backend: expose `rel_types` + `min_confidence` on `/neighbors`** | Front-end filters need a real round-trip. |
| **Filter bar (kind / rel-type / confidence / time)** | The picker already lists kinds; surfacing them as filters is the obvious extension. |
| **Kind-colored nodes (KIND_COLORS) + degree-sized nodes** | Visual distinction between entity types. |
| **Confidence-weighted edge opacity + arrow heads + rel_type label** | Edges carry weight; show it. |
| **Entity side panel** (click node → details + timeline + evidence) | The page would otherwise be useless as an investigation tool. |
| **Edge side panel** (click edge → source/target/evidence) | Same reason — relationships are first-class. |
| **Real `source_id`/`target_id` (no more star topology)** | Depth > 1 must render as a graph, not a star. |

## 2. Data model deltas

### 2.1 Backend Cypher (hub-core/src/graph_queries.rs:482-491)

```cypher
MATCH p = (e:Entity {entity_id: $eid}})-[*1..{d}]-(neighbor:Entity)
WHERE e <> neighbor
  AND ($at_time IS NULL OR ALL(rel IN relationships(p) WHERE ...))
WITH neighbor, relationships(p) AS rs LIMIT 100
RETURN neighbor.entity_id AS id, neighbor.name AS name, neighbor.kind AS kind,
       [r IN rs | {
         rel_type:   type(r),
         source_id:  startNode(r).entity_id,    -- NEW
         target_id:  endNode(r).entity_id,      -- NEW
         valid_from: r.valid_from,
         valid_until: r.valid_until,
         confidence: r.confidence
       }] AS edges
```

The `r IN rs` filter loop (Rust lines 522-540) keeps the post-Cypher
rel_type / confidence filter logic unchanged — we just add two new map
keys to the output.

### 2.2 REST route (`api.rs:1015-1037`)

```rust
#[derive(Deserialize)]
struct NeighborsQuery {
    root: String,
    depth: Option<u8>,
    at_time: Option<String>,
    rel_types: Option<String>,        // NEW: comma-separated
    min_confidence: Option<f64>,      // NEW
}

async fn graph_neighbors(...) {
    let rel_types = q.rel_types.as_deref()
        .map(|s| s.split(',').map(|x| x.trim().to_string())
             .filter(|x| !x.is_empty()).collect::<Vec<_>>());
    ...
    crate::graph_queries::get_neighbors(&state, entity_id, depth, rel_types, min_confidence, at_time).await
}
```

### 2.3 Console types (`console/src/api/graph.ts`)

```ts
export interface GraphEdge {
  rel_type: string;
  source_id: string;          // NEW
  target_id: string;          // NEW
  valid_from?: string | null;
  valid_until?: string | null;
  confidence?: number | null;
}
```

### 2.4 getNeighbors wrapper

```ts
export async function getNeighbors(
  root: string,
  depth = 2,
  opts?: { atTime?: string; relTypes?: string[]; minConfidence?: number }
): Promise<EntitySummary[]> { ... }
```

## 3. UI architecture (Graph.tsx rewrite)

```
┌─────────────────────────────────────────────────────────────┐
│ Filters: [kind▾] [rel▾] [conf≥0.5] [time: ___→___] [search] │
├──────────────┬─────────────────────────────────┬─────────────┤
│ entity picker│  G6 canvas (full graph)         │ side panel  │
│ (existing)   │  - colored by kind              │ (node/edge) │
│              │  - sized by degree              │             │
│              │  - opacity by confidence        │             │
│              │  - real source→target edges     │             │
│              │  - click → side panel           │             │
└──────────────┴─────────────────────────────────┴─────────────┘
```

### Components introduced

| Component | Responsibility |
|---|---|
| `FilterBar.tsx` | Kind chips, rel-type chips, confidence slider, time range inputs. Debounced; pushes filters up. |
| `SidePanel.tsx` | Renders either `EntityPanel` or `EdgePanel` based on selection. Loading + error states. |
| `EntityPanel.tsx` | Name / kind / aliases / metadata table; embeds `TimelineView` + `EvidenceList`. Calls `/api/v1/graph/entity/{id}/timeline` and `/api/v1/graph/evidence?entity=…`. |
| `EdgePanel.tsx` | `source → target` with rel_type, confidence bar, evidence list. |
| `TimelineView.tsx` | Vertical scrollable list of timestamped events for the entity. |
| `EvidenceList.tsx` | List of `{document_id, base_url, retrieved_at, relation}` rows. |
| `GraphCanvas.tsx` | The G6 mount; takes `data` + selection state + `onSelectNode/onSelectEdge`. |

### State (lifted to GraphPage)

```ts
{
  root: string | null,                  // selected entity_id (existing)
  neighbors: EntitySummary[] | null,    // (existing)
  filter: { kinds: Set<string>; relTypes: Set<string>; minConfidence: number; atTime?: [from, to] },
  sidePanelSelection: { kind: 'node'|'edge'; data: ... } | null,
  sidePanelData: { loading: boolean; timeline?: TimelineEntry[]; evidence?: unknown[] },
}
```

Filters apply **before** sending `rel_types`/`min_confidence`/`at_time` to the
backend. Kind filter applies client-side (the backend `entities/search`
already supports `?kind=` — re-using it would invalidate the current neighbors
fetch; client-side filter is simpler and cheaper for ≤100 neighbors).

### G6 styling

```ts
{
  node: {
    style: {
      labelText: d => d.name,
      fill: d => kindColor(d.kind),
      stroke: '#fff',
      lineWidth: 1,
      size: d => 16 + Math.min(20, d.degree * 4),  // 16–36 px by degree
      labelBackground: true,
      labelPadding: [2, 4],
    }
  },
  edge: {
    type: 'line',
    style: {
      stroke: d => confidenceOpacity(d.confidence),
      endArrow: true,
      endArrowSize: 8,
      labelText: d => d.rel_type,
      labelBackground: true,
      labelPadding: [1, 3],
    }
  },
  behaviors: ['drag-canvas','zoom-canvas','click-select'],
}
```

`confidenceOpacity(c)` = `0.2 + 0.8 * (c ?? 1)` — `0.2` for c=0, `1.0` for c≥1.

### Multi-hop edges (no more star topology)

Replace the hard-coded `source: root, target: e.entity_id` loop with:

```ts
function g6Data(root, data) {
  const nodes = new Map<string, G6Node>();
  const edges: G6Edge[] = [];
  nodes.set(root, { id: root, name: 'root', kind: 'selected', degree: 0 });
  for (const e of data) {
    nodes.set(e.entity_id, { id: e.entity_id, name: e.name, kind: e.kind, degree: e.edges.length });
    for (const ed of e.edges || []) {
      if (!ed.rel_type || !ed.source_id || !ed.target_id) continue;
      // Don't double-add edges
      const key = `${ed.source_id}->${ed.target_id}:${ed.rel_type}`;
      if (!edges.find(x => x._key === key)) {
        edges.push({ _key: key, source: ed.source_id, target: ed.target_id, rel_type: ed.rel_type, confidence: ed.confidence });
      }
    }
  }
  return { nodes: [...nodes.values()], edges };
}
```

This works because the SP9 Cypher returns the FULL path traversal; a
neighbor at depth=2 may have an edge from `root → intermediate → neighbor`,
but the Cypher flattens `relationships(p)` so we get both edges in the
returned `edges[]`. The console now connects each to its real endpoints.

## 4. Acceptance (`scripts/accept-sp10.py`)

25+ assertions following the sp9 pattern. Sections:

1. **Backend enrichment** (8 checks)
   - `/api/v1/graph/neighbors?root=<eid>` returns edges with `source_id`/`target_id` (not missing/null)
   - `?rel_types=located_in` filters out other rel types
   - `?min_confidence=0.8` filters out low-confidence edges
   - Combined filters work

2. **Console bundle** (10 checks)
   - Bundle contains `kindColor`/`KIND_COLORS` usage (string literals)
   - Bundle contains `FilterBar`/`SidePanel`/`EntityPanel`/`EdgePanel`/`TimelineView`/`EvidenceList` strings (proves components built)
   - Bundle contains confidence-opacity function
   - No `@antv/g6` `labelText`-only pattern remains (search for old single-line labelText pattern)

3. **Console route** (3 checks)
   - `GET /graph` (via SPA HTML probe) returns 200
   - HTML references the bundled JS asset

4. **Side-panel API** (4 checks)
   - `/api/v1/graph/entity/{id}/timeline` returns ≥1 row for a known entity
   - `/api/v1/graph/evidence?entity={id}` returns ≥0 rows
   - `/api/v1/graph/neighbors` accepts the new params without 400

5. **UI sanity** (3 checks)
   - Source code references `kindColor` from `kindmeta.ts`
   - Side panel markup present in component file (`<SidePanel>` JSX)
   - Edge data shape used (`source_id`/`target_id`)

## 5. Out of scope (deferred)

- Image/avatar rendering for entities (no source data exists; would need
  upstream pipeline change — SP11+).
- Layout algorithm choice (force / dagre / circular). G6 default is fine
  for ≤200 nodes; revisit if usage demands.
- Edge-curving for multi-edges between same pair.
- Investigation-bound subgraph UI (the `/investigation/{id}` endpoint is
  wired but not yet exposed in console).
- Filtering by source/document.

These are non-blocking for the user's "complete canvas" directive.