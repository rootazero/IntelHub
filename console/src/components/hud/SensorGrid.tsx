export interface SourceHealth {
  name: string;
  state: string;
  last_new?: number;
}

/** Crucix-style sensor grid: one row per monitor source (dot + name + last_new). */
export default function SensorGrid({
  sources,
  selected,
  onSelect,
}: {
  sources: SourceHealth[];
  selected: string | null;
  onSelect: (name: string | null) => void;
}) {
  return (
    <div>
      {sources.map((s) => (
        <div
          key={s.name}
          className={`hud-sensor-row ${selected === s.name ? "active" : ""}`}
          onClick={() => onSelect(selected === s.name ? null : s.name)}
        >
          <span className={`hud-dot ${s.state === "ok" ? "hud-dot-ok" : s.state === "error" ? "hud-dot-bad" : "hud-dot-dim"}`} />
          <span className="hud-mono flex-1 truncate">{s.name}</span>
          <span className="hud-mono text-[10px]" style={{ color: s.last_new ? "var(--hud-accent)" : "var(--hud-dim)" }}>
            +{s.last_new ?? 0}
          </span>
        </div>
      ))}
      {sources.length === 0 && (
        <div className="px-1 py-2 text-[11px]" style={{ color: "var(--hud-dim)" }}>—</div>
      )}
    </div>
  );
}
