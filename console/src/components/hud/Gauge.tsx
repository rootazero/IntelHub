import Sparkline from "./Sparkline";

/** Risk gauge: big mono value + 40d sparkline + threshold warn coloring. */
export default function Gauge({
  label,
  value,
  points,
  warn,
  active,
  onClick,
}: {
  label: string;
  value: number | null;
  points: number[];
  warn?: boolean;
  active?: boolean;
  onClick?: () => void;
}) {
  return (
    <div className={`hud-gauge ${active ? "active" : ""} ${warn ? "hud-gauge-warn" : ""}`} onClick={onClick}>
      <div className="flex items-center justify-between gap-2">
        <div>
          <div className="text-[9px] uppercase tracking-[0.16em]" style={{ color: "var(--hud-dim)" }}>{label}</div>
          <div className="hud-gauge-value hud-mono">{value === null ? "—" : value.toFixed(2)}</div>
        </div>
        <Sparkline points={points} width={86} height={26} color={warn ? "#ffb84c" : "#64f0c8"} />
      </div>
    </div>
  );
}
