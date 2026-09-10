// Crucix-style live news ticker: horizontally auto-scrolling marquee of recent
// news-kind geo events (RSS/GDELT monitor sweeps). Duplicated track for a
// seamless loop; pauses on hover; animation removed in VISUALS LITE.
import { useEffect, useState } from "react";
import { api } from "../../api";

interface NewsItem {
  event_id: string;
  title: string;
  source: string;
  occurred_at: string;
}

export default function NewsTicker({ refreshKey }: { refreshKey: number }) {
  const [items, setItems] = useState<NewsItem[]>([]);

  useEffect(() => {
    let live = true;
    const load = () => {
      const from = new Date(Date.now() - 24 * 3600_000).toISOString();
      api<{ items: NewsItem[] }>(
        `/api/v1/radar/events?from=${encodeURIComponent(from)}&kind=news&limit=60`,
      )
        .then((d) => { if (live) setItems(d.items ?? []); })
        .catch(() => {});
    };
    load();
    const t = setInterval(load, 60_000);
    return () => { live = false; clearInterval(t); };
  }, [refreshKey]);

  if (items.length === 0) {
    return <div className="py-2 text-[11px]" style={{ color: "var(--hud-dim)" }}>—</div>;
  }
  const track = [...items, ...items]; // duplicated for seamless wrap
  return (
    <div className="hud-ticker">
      <div className="hud-ticker-track">
        {track.map((n, i) => (
          <span key={`${n.event_id}-${i}`} className="hud-ticker-item" title={n.title}>
            <span className="hud-ticker-src">{n.source.replace("monitor:", "")}</span>
            {n.title}
          </span>
        ))}
      </div>
    </div>
  );
}
