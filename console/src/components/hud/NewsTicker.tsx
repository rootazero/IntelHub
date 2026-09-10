// Crucix OSINT-stream style vertical ticker: news-kind geo events scroll
// slowly UPWARD in a seamless loop (duplicated track, translateY -50%).
// Speed adapts to item count (~5s per item) so users can track entries;
// hover pauses for mouse capture; animation removed in VISUALS LITE.
import { useEffect, useState } from "react";
import { api } from "../../api";
import { TimeAgo } from "../../ui";

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
  const duration = Math.max(40, items.length * 5); // ~5s per item, floor 40s
  return (
    <div className="hud-vticker">
      <div className="hud-vticker-track" style={{ animationDuration: `${duration}s` }}>
        {track.map((n, i) => (
          <div key={`${n.event_id}-${i}`} className="hud-vticker-item" title={n.title}>
            <div className="flex items-center gap-1.5">
              <span className="hud-ticker-src">{n.source.replace("monitor:", "")}</span>
              <TimeAgo ts={n.occurred_at} />
            </div>
            <div className="hud-vticker-title">{n.title}</div>
          </div>
        ))}
      </div>
    </div>
  );
}
