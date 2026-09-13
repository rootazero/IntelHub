// SP10 G6 mount. Takes pre-built data; emits clicks for the side panel.
// No data fetch logic — that's in GraphPage so filters and selection can
// be lifted and stay in sync with the side panel.
//
// Layout note (2026-09-14 fix): G6 v5 does NOT auto-layout — without a
// `layout` option every node lands at (0,0) and the canvas shows a single
// half-clipped dot in the top-left corner ("root" label rendered as "ot").
// We use d3-force + autoFit:'view' so the graph is spread and framed.
//
// Selection note: clicking used to re-create the whole graph (re-running
// the force simulation → visible jitter). Highlight now goes through G6's
// built-in 'click-select' behavior + state.selected style overrides, so
// the graph instance is only re-created when the data itself changes.

import { useEffect, useRef } from "react";
import { kindColor } from "../kindmeta";

export interface G6NodeData {
  id: string;
  name: string;
  kind: string;
  degree: number;
  [k: string]: any;
}
export interface G6EdgeData {
  key: string;
  source: string;
  target: string;
  rel_type: string;
  confidence: number | null;
  [k: string]: any;
}

interface Props {
  rootId: string;
  nodes: G6NodeData[];
  edges: G6EdgeData[];
  onSelectNode: (id: string | null) => void;
  onSelectEdge: (key: string | null) => void;
}

/// Confidence → stroke opacity in [0.2, 1.0]. Null = treat as 1.0.
function confidenceOpacity(c: number | null | undefined): number {
  if (c == null) return 1;
  const clamped = Math.max(0, Math.min(1, c));
  return 0.2 + 0.8 * clamped;
}

/// Node size in px: 16 base + 4·degree capped at 36. Bigger hubs are visually obvious.
function nodeSize(degree: number): number {
  return Math.min(36, 16 + Math.max(0, degree) * 4);
}

export default function GraphCanvas({
  rootId,
  nodes,
  edges,
  onSelectNode,
  onSelectEdge,
}: Props) {
  const containerRef = useRef<HTMLDivElement>(null);
  const graphRef = useRef<unknown>(null);

  // Render / re-render ONLY when the data changes (not on click).
  useEffect(() => {
    if (!containerRef.current) return;
    let cancelled = false;
    let graph: { render(): Promise<void>; destroy(): void; setData(d: unknown): Promise<void> } | null = null;

    (async () => {
      const G6 = await import("@antv/g6");
      if (cancelled || !containerRef.current) return;

      graph = new G6.Graph({
        container: containerRef.current,
        width: containerRef.current.clientWidth,
        height: containerRef.current.clientHeight,
        // d3-force spreads nodes; autoFit frames the result in the viewport.
        // (Both required — autoFit alone doesn't layout, layout alone may
        // leave the graph off-screen.)
        layout: {
          type: "d3-force",
          link: { distance: 140, strength: 0.6 },
          manyBody: { strength: -260 },
          collide: { radius: 30 },
        },
        autoFit: "view",
        data: { nodes, edges } as any,
        node: {
          style: {
            labelText: (d: unknown) => (d as G6NodeData).name,
            fill: (d: unknown) =>
              (d as G6NodeData).id === rootId ? "#ffffff" : kindColor((d as G6NodeData).kind),
            stroke: "rgba(255,255,255,0.5)",
            lineWidth: 1,
            size: (d: unknown) => nodeSize((d as G6NodeData).degree),
            labelBackground: true,
            labelPadding: [2, 4],
            labelFill: "#0a0e14",
            labelFontSize: 11,
          },
          // G6 'click-select' toggles this state automatically — no
          // re-render needed to show the highlight.
          state: {
            selected: {
              stroke: "#ffffff",
              lineWidth: 3,
            },
          },
        },
        edge: {
          type: "line",
          style: {
            stroke: (d: unknown) => {
              const e = d as G6EdgeData;
              return `rgba(180,200,220,${confidenceOpacity(e.confidence)})`;
            },
            lineWidth: 1,
            endArrow: true,
            endArrowSize: 8,
            endArrowFill: (d: unknown) => {
              const e = d as G6EdgeData;
              return `rgba(180,200,220,${confidenceOpacity(e.confidence)})`;
            },
            labelText: (d: unknown) => (d as G6EdgeData).rel_type,
            labelBackground: true,
            labelPadding: [1, 3],
            labelFontSize: 9,
            labelFill: "#9aa4b2",
          },
          state: {
            selected: {
              stroke: "#ffffff",
              lineWidth: 2.5,
              endArrowFill: "#ffffff",
            },
          },
        },
        behaviors: ["drag-canvas", "zoom-canvas", "click-select"],
      }) as unknown as typeof graph;

      await graph!.render();

      // Wire up click → side panel. G6 v5 fires 'node:click' / 'edge:click'
      // (and 'canvas:click' for deselect). Visual highlight is handled by
      // the click-select behavior + state styles above.
      const ev = graph as unknown as {
        on: (evt: string, fn: (e: { target?: { id?: string; data?: { key?: string } } }) => void) => void;
      };
      ev.on("node:click", (e) => {
        const id = e?.target?.id ?? null;
        onSelectNode(id);
      });
      ev.on("edge:click", (e) => {
        const k = e?.target?.data?.key ?? null;
        onSelectEdge(k);
      });
      ev.on("canvas:click", () => {
        onSelectNode(null);
        onSelectEdge(null);
      });

      graphRef.current = graph;
    })().catch((err) => {
      console.error("GraphCanvas mount failed:", err);
    });

    return () => {
      cancelled = true;
      if (graph) graph.destroy();
    };
  }, [nodes, edges, rootId, onSelectNode, onSelectEdge]);

  // Resize observer — keep canvas matched to its container.
  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;
    const ro = new ResizeObserver(() => {
      const g = graphRef.current as { resize?: () => void } | null;
      try {
        g?.resize?.();
      } catch {
        /* G6 sometimes throws on rapid resize; safe to ignore */
      }
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  return (
    <div
      ref={containerRef}
      className="border border-edge bg-base"
      style={{ height: 520, width: "100%" }}
    />
  );
}