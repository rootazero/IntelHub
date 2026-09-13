// SP10 FilterBar — kind chips, rel-type chips, confidence slider, time inputs.
// Sibling control set; parent owns the filter state and re-fetches on change.

import { KINDS, kindColor } from "../kindmeta";

export interface FilterState {
  kinds: Set<string>;
  relTypes: Set<string>;
  minConfidence: number; // 0..1
  from: string;          // ISO yyyy-mm-dd; empty = unset
  to: string;
}

interface Props {
  value: FilterState;
  onChange: (next: FilterState) => void;
  /** All rel_types observed in the current neighbors payload (so the chips
   *  reflect what's actually on screen, not a hard-coded allowlist). */
  availableRelTypes: string[];
  totalNodes: number;
  totalEdges: number;
}

export const emptyFilter = (): FilterState => ({
  kinds: new Set<string>(),
  relTypes: new Set<string>(),
  minConfidence: 0,
  from: "",
  to: "",
});

export default function FilterBar({
  value,
  onChange,
  availableRelTypes,
  totalNodes,
  totalEdges,
}: Props) {
  const toggleKind = (k: string) => {
    const next = new Set(value.kinds);
    if (next.has(k)) next.delete(k);
    else next.add(k);
    onChange({ ...value, kinds: next });
  };
  const toggleRel = (r: string) => {
    const next = new Set(value.relTypes);
    if (next.has(r)) next.delete(r);
    else next.add(r);
    onChange({ ...value, relTypes: next });
  };
  const resetAll = () => {
    onChange(emptyFilter());
  };

  const activeKinds = value.kinds.size;
  const activeRels = value.relTypes.size;
  const confidenceActive = value.minConfidence > 0;
  const timeActive = value.from !== "" || value.to !== "";
  const anyFilterActive = activeKinds + activeRels + (confidenceActive ? 1 : 0) + (timeActive ? 1 : 0) > 0;

  return (
    <div className="border border-edge bg-base p-2 mb-2 text-xs">
      <div className="flex items-center justify-between mb-2">
        <div className="text-dim">
          <span className="font-bold">{totalNodes}</span> nodes · <span className="font-bold">{totalEdges}</span> edges
          {anyFilterActive && <span className="ml-2 text-yellow-400">(filtered)</span>}
        </div>
        <button
          type="button"
          onClick={resetAll}
          disabled={!anyFilterActive}
          className="px-2 py-0.5 border border-edge bg-base hover:bg-edge disabled:opacity-30"
        >
          reset
        </button>
      </div>

      {/* Kind chips */}
      <div className="mb-2">
        <p className="text-dim uppercase mb-1">kind ({activeKinds || "any"})</p>
        <div className="flex flex-wrap gap-1">
          {KINDS.map((k) => {
            const active = value.kinds.has(k);
            return (
              <button
                key={k}
                type="button"
                onClick={() => toggleKind(k)}
                className={
                  "px-1.5 py-0.5 border text-[10px] " +
                  (active ? "border-white text-black font-bold" : "border-edge text-dim hover:text-white")
                }
                style={active ? { background: kindColor(k) } : undefined}
                title={`filter by kind=${k}`}
              >
                {k}
              </button>
            );
          })}
        </div>
      </div>

      {/* Rel-type chips — only show kinds that actually appear */}
      {availableRelTypes.length > 0 && (
        <div className="mb-2">
          <p className="text-dim uppercase mb-1">relationship ({activeRels || "any"})</p>
          <div className="flex flex-wrap gap-1">
            {availableRelTypes.map((r) => {
              const active = value.relTypes.has(r);
              return (
                <button
                  key={r}
                  type="button"
                  onClick={() => toggleRel(r)}
                  className={
                    "px-1.5 py-0.5 border text-[10px] " +
                    (active ? "border-white text-black font-bold" : "border-edge text-dim hover:text-white")
                  }
                  style={active ? { background: "rgba(255,255,255,0.2)" } : undefined}
                  title={`filter by rel_type=${r}`}
                >
                  {r}
                </button>
              );
            })}
          </div>
        </div>
      )}

      {/* Confidence slider */}
      <div className="mb-2 flex items-center gap-2">
        <label className="text-dim uppercase">conf ≥</label>
        <input
          type="range"
          min={0}
          max={1}
          step={0.05}
          value={value.minConfidence}
          onChange={(e) => onChange({ ...value, minConfidence: Number(e.target.value) })}
          className="flex-1"
        />
        <span className="mono w-8 text-right">{value.minConfidence.toFixed(2)}</span>
      </div>

      {/* Time range (valid_from / valid_until envelope) */}
      <div className="flex items-center gap-2">
        <label className="text-dim uppercase">time</label>
        <input
          type="date"
          value={value.from}
          onChange={(e) => onChange({ ...value, from: e.target.value })}
          className="border border-edge bg-base px-1 py-0.5 mono text-[10px]"
        />
        <span className="text-dim">→</span>
        <input
          type="date"
          value={value.to}
          onChange={(e) => onChange({ ...value, to: e.target.value })}
          className="border border-edge bg-base px-1 py-0.5 mono text-[10px]"
        />
        <span className="text-dim text-[10px]">(valid_from/valid_until envelope)</span>
      </div>
    </div>
  );
}