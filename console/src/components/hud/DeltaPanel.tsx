import { useT } from "../../i18n";

export interface DeltaRow {
  source: string;
  state: string;
  new: number;
  fetched: number;
  prev_new: number | null;
  direction: "up" | "down" | "flat" | "new_source";
  ts: string | null;
}

const ARROW: Record<string, [string, string]> = {
  up: ["▲", "var(--hud-warn)"],
  down: ["▼", "var(--hud-good)"],
  flat: ["=", "var(--hud-dim)"],
  new_source: ["NEW", "var(--hud-accent2)"],
};

/** Sweep-delta panel: per-source direction vs previous sweep (Crucix pattern). */
export default function DeltaPanel({ rows }: { rows: DeltaRow[] }) {
  const { t } = useT();
  const accumulating = rows.length > 0 && rows.every((r) => r.direction === "new_source");
  return (
    <div className="hud-mono">
      {accumulating && (
        <div className="mb-1 text-[10px]" style={{ color: "var(--hud-accent2)" }}>{t("hud.accumulating")}</div>
      )}
      {rows.map((r) => {
        const [arrow, color] = ARROW[r.direction] ?? ARROW.flat;
        return (
          <div key={r.source} className="hud-delta-row">
            <span style={{ color, minWidth: 26, fontSize: 9 }}>{arrow}</span>
            <span className="flex-1 truncate">{r.source}</span>
            <span className="text-[10px]" style={{ color: "var(--hud-dim)" }}>
              {r.prev_new === null ? "—" : r.prev_new} → <b style={{ color }}>{r.new}</b>
            </span>
          </div>
        );
      })}
      {rows.length === 0 && <div className="py-2 text-[11px]" style={{ color: "var(--hud-dim)" }}>{t("hud.noData")}</div>}
    </div>
  );
}
