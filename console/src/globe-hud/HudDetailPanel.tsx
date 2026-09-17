// HUD detail panel (T10): right-edge resident panel showing the currently
// selected globe target, templated per object kind. Consumes the T10
// context-bridge hook (engine contextStore → React); no engine imports here.
//
// Templates (task-10 brief):
//   flight    — callsign, altitude (m), speed (km/h), heading, [进图谱] → /graph
//   satellite — name, NORAD id, group
//   quake     — magnitude, place, relative time
// Empty state — 「点击地球上的目标查看详情」. Collapse handle in the panel's
// top-right corner mirrors the T9 rail handle.
import { useState } from "react";
import { useGlobeSelection } from "../gev-boot/context-bridge";

/** Relative clock for quake times: <60 s 刚刚, <60 min N 分钟前,
 *  <24 h N 小时前, otherwise an absolute "YYYY-MM-DD HH:mm". */
export function formatRelativeTime(timeMs: number, now = Date.now()): string {
  const age = now - timeMs;
  if (age < 60_000) return "刚刚";
  if (age < 3_600_000) return `${Math.floor(age / 60_000)} 分钟前`;
  if (age < 86_400_000) return `${Math.floor(age / 3_600_000)} 小时前`;
  const d = new Date(timeMs);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

const KIND_TITLE: Record<string, string> = {
  flight: "航班 FLIGHT",
  satellite: "卫星 SATELLITE",
  quake: "地震 QUAKE",
};

function formatInt(value: number | null | undefined): string {
  return value == null ? "—" : Math.round(value).toLocaleString("en-US");
}

function FlightBody({ data }: { data: Record<string, unknown> }) {
  const callsign = String(data.callsign || data.label || "");
  const query = encodeURIComponent(callsign);
  const route = String(data.route || "");
  return (
    <>
      <div className="hud-detail-title">{callsign || "—"}</div>
      <dl className="hud-detail-fields">
        <dt>高度</dt>
        <dd>{formatInt(data.altitudeM as number | null)} m</dd>
        <dt>速度</dt>
        <dd>{formatInt(data.speedKmh as number | null)} km/h</dd>
        <dt>航向</dt>
        <dd>
          {data.headingDeg == null ? "—" : `${Math.round(data.headingDeg as number)}°`}
        </dd>
        {route && (
          <>
            <dt>航线</dt>
            <dd>{route}</dd>
          </>
        )}
      </dl>
      <div className="hud-detail-actions">
        <a href={`/graph?q=${query}`} className="hud-detail-link">
          进图谱
        </a>
      </div>
    </>
  );
}

function SatelliteBody({ data }: { data: Record<string, unknown> }) {
  return (
    <>
      <div className="hud-detail-title">{String(data.name || data.label || "—")}</div>
      <dl className="hud-detail-fields">
        <dt>NORAD</dt>
        <dd>{String(data.noradId || "—")}</dd>
        <dt>组</dt>
        <dd>{String(data.group || "—")}</dd>
      </dl>
    </>
  );
}

function QuakeBody({ data }: { data: Record<string, unknown> }) {
  const mag = data.mag == null ? "—" : `M${Number(data.mag).toFixed(1)}`;
  const timeMs = data.timeMs as number | null;
  return (
    <>
      <div className="hud-detail-title">{mag}</div>
      <dl className="hud-detail-fields">
        <dt>位置</dt>
        <dd>{String(data.place || "—")}</dd>
        <dt>时间</dt>
        <dd>{timeMs == null ? "—" : formatRelativeTime(timeMs)}</dd>
      </dl>
    </>
  );
}

export function HudDetailPanel() {
  const selection = useGlobeSelection();
  const [collapsed, setCollapsed] = useState(false);

  return (
    <div
      className={`hud-detail${collapsed ? " collapsed" : ""}`}
      data-testid="hud-detail-panel"
      data-kind={selection.kind ?? "none"}
    >
      <button
        type="button"
        className="hud-detail-handle"
        title={collapsed ? "展开详情面板" : "折叠详情面板"}
        aria-label={collapsed ? "展开详情面板" : "折叠详情面板"}
        aria-expanded={!collapsed}
        onClick={() => setCollapsed((value) => !value)}
      >
        {collapsed ? "◂" : "▸"}
      </button>
      {!collapsed && (
        <div className="hud-detail-body">
          {selection.kind === null || !selection.data ? (
            <div className="hud-detail-empty">点击地球上的目标查看详情</div>
          ) : (
            <>
              <div className="hud-detail-kind">
                {KIND_TITLE[selection.kind] ?? selection.kind}
              </div>
              {selection.kind === "flight" && (
                <FlightBody data={selection.data} />
              )}
              {selection.kind === "satellite" && (
                <SatelliteBody data={selection.data} />
              )}
              {selection.kind === "quake" && <QuakeBody data={selection.data} />}
            </>
          )}
        </div>
      )}
    </div>
  );
}
