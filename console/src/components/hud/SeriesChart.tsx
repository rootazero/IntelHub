import { useEffect, useState } from "react";
import { api } from "../../api";
import { useT } from "../../i18n";

interface Pt { t: string; v: number }

/** Hand-rolled SVG line chart for /api/v1/signals/history (zero chart deps). */
export default function SeriesChart({ series, days = 40 }: { series: string; days?: number }) {
  const { t } = useT();
  const [points, setPoints] = useState<Pt[]>([]);
  const [error, setError] = useState(false);

  useEffect(() => {
    let live = true;
    setError(false);
    api<{ points: Pt[] }>(`/api/v1/signals/history?series=${encodeURIComponent(series)}&days=${days}`)
      .then((d) => { if (live) setPoints(d.points ?? []); })
      .catch(() => { if (live) { setPoints([]); setError(true); } });
    return () => { live = false; };
  }, [series, days]);

  const W = 640, H = 150, PAD = 8;
  const vals = points.map((p) => p.v);
  const min = vals.length ? Math.min(...vals) : 0;
  const max = vals.length ? Math.max(...vals) : 0;
  const span = max - min || 1;
  const step = points.length > 1 ? (W - PAD * 2) / (points.length - 1) : 0;
  const y = (v: number) => H - PAD - ((v - min) / span) * (H - PAD * 2);
  const d = points.map((p, i) => `${i === 0 ? "M" : "L"}${(PAD + i * step).toFixed(1)},${y(p.v).toFixed(1)}`).join(" ");
  const fmt = (v: number) => (Math.abs(v) >= 1000 ? v.toLocaleString(undefined, { maximumFractionDigits: 0 }) : v.toFixed(2));
  const day = (iso: string) => iso.slice(5, 10);

  return (
    <div className="hud-mono">
      <div className="mb-1 flex items-baseline justify-between text-[10px]" style={{ color: "var(--hud-dim)" }}>
        <span className="tracking-widest">{series}</span>
        {points.length > 0 && (
          <span>
            {day(points[0].t)} → {day(points[points.length - 1].t)} · {t("hud.last")}{" "}
            <b style={{ color: "var(--hud-accent)" }}>{fmt(vals[vals.length - 1])}</b>
          </span>
        )}
      </div>
      {points.length < 2 ? (
        <div className="flex h-[150px] items-center justify-center text-[11px]" style={{ color: "var(--hud-dim)" }}>
          {error ? t("common.error") : t("hud.noData")}
        </div>
      ) : (
        <svg viewBox={`0 0 ${W} ${H}`} className="w-full">
          {[0.25, 0.5, 0.75].map((f) => (
            <line key={f} x1={PAD} x2={W - PAD} y1={H * f} y2={H * f} stroke="rgba(100,240,200,.08)" strokeWidth={1} />
          ))}
          <path d={d} fill="none" stroke="#44ccff" strokeWidth={1.6} strokeLinejoin="round" />
          <circle cx={PAD + (points.length - 1) * step} cy={y(vals[vals.length - 1])} r={3} fill="#64f0c8" />
          <text x={PAD} y={12} fontSize={9} fill="#5d7285">{fmt(max)}</text>
          <text x={PAD} y={H - 2} fontSize={9} fill="#5d7285">{fmt(min)}</text>
        </svg>
      )}
    </div>
  );
}
