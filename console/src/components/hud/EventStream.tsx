import type { BusEventPayload } from "../../api";
import { TimeAgo } from "../../ui";

/** Live SSE event cards (monitor sweeps with new items + alerts). */
export default function EventStream({ events }: { events: BusEventPayload[] }) {
  return (
    <div>
      {events.map((ev) => {
        const isAlert = ev.event_type === "alert_raised";
        const p = ev.payload as Record<string, unknown>;
        const text = isAlert
          ? String(p.title ?? ev.event_type)
          : `${p.source ?? "?"}  +${String(p.new ?? 0)}/${String(p.fetched ?? 0)}`;
        return (
          <div key={ev.event_id} className={`hud-stream-card ${isAlert ? "alert" : ""}`}>
            <div className="flex items-center justify-between gap-2">
              <span className="hud-mono text-[9px] uppercase tracking-wider" style={{ color: "var(--hud-dim)" }}>
                {ev.event_type}
              </span>
              <TimeAgo ts={ev.ts} />
            </div>
            <div className="mt-0.5 truncate" title={text}>{text}</div>
          </div>
        );
      })}
      {events.length === 0 && (
        <div className="py-2 text-[11px]" style={{ color: "var(--hud-dim)" }}>—</div>
      )}
    </div>
  );
}
