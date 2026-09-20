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
import { useEffect, useRef, useState, type ReactNode } from "react";
import { useGlobeSelection } from "../gev-boot/context-bridge";
import type { FollowHandle } from "../gev-visual/follow-controller";
import type { CameraOrientationHandle } from "../gev-visual/camera-orientation";
import { CctvPopoutPanel, type PopoutCamera } from "./CctvPopoutPanel";

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
  vessel: "船舶 VESSEL",
  cctv: "摄像头 CCTV",
  installation: "军事设施 INSTALLATION",
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

/** class → bilingual label. Keys mirror the vendor humanize table
 * (militaryInstallationData.js:24-29,42-52) plus the google-search
 * 'places_candidate' bucket (policy.js GOOGLE_MILITARY_PLACE_TYPES). */
const INSTALLATION_CLASS_LABEL: Record<string, string> = {
  airfield: "军用机场 Airfield",
  naval_base: "海军基地 Naval base",
  range: "靶场 Range",
  barracks: "营房 Barracks",
  military_land: "军事用地 Military land",
  base: "军事基地 Base",
  places_candidate: "搜索候选 Candidate",
};

function humanizeInstallationClass(value: string): string {
  if (!value) return "—";
  return (
    INSTALLATION_CLASS_LABEL[value] ??
    value.replaceAll("_", " ").replace(/\b\w/g, (c) => c.toUpperCase())
  );
}

function VesselBody({ data }: { data: Record<string, unknown> }) {
  // speedKn is already knots: the bridge converts speedMps ÷0.514444
  // (vessels.js ingestion.js:157 convention) or passes vendor speedKt through.
  const speedKn = data.speedKn as number | null;
  const observedAtMs = data.observedAtMs as number | null;
  return (
    <>
      <div className="hud-detail-title">
        {String(data.name || data.label || "—")}
      </div>
      <dl className="hud-detail-fields">
        <dt>MMSI</dt>
        <dd>{String(data.mmsi || "—")}</dd>
        <dt>IMO</dt>
        <dd>{String(data.imo || "—")}</dd>
        <dt>类型</dt>
        <dd>{String(data.type || "—")}</dd>
        <dt>目的地</dt>
        <dd>{String(data.destination || "—")}</dd>
        <dt>航速</dt>
        <dd>{speedKn == null ? "—" : `${speedKn.toFixed(1)} kn`}</dd>
        <dt>航向</dt>
        <dd>
          {data.courseDeg == null ? "—" : `${Math.round(data.courseDeg as number)}°`}
        </dd>
        <dt>船头向</dt>
        <dd>
          {data.headingDeg == null ? "—" : `${Math.round(data.headingDeg as number)}°`}
        </dd>
        <dt>观测</dt>
        <dd>{observedAtMs == null ? "—" : formatRelativeTime(observedAtMs)}</dd>
      </dl>
    </>
  );
}

function CctvBody({
  data,
  onOpenPopout,
}: {
  data: Record<string, unknown>;
  onOpenPopout: (cam: PopoutCamera) => void;
}) {
  const frameUrl = String(data.frameUrl || "");
  const live = data.live as boolean | null;
  return (
    <>
      <div className="hud-detail-title">
        {String(data.name || data.label || "—")}
      </div>
      <dl className="hud-detail-fields">
        <dt>状态</dt>
        <dd>{live == null ? "—" : live ? "在线" : "离线"}</dd>
        <dt>城市</dt>
        <dd>{String(data.city || "—")}</dd>
        <dt>提供商</dt>
        <dd>{String(data.provider || "—")}</dd>
        <dt>源类型</dt>
        <dd>{String(data.feedType || "—")}</dd>
        <dt>方位</dt>
        <dd>
          {data.headingDeg == null ? "—" : `${Math.round(data.headingDeg as number)}°`}
          {" / "}
          {data.fovDeg == null ? "—" : `${Math.round(data.fovDeg as number)}°`}
          {" / "}
          {data.pitchDeg == null ? "—" : `${Math.round(data.pitchDeg as number)}°`}
        </dd>
      </dl>
      <div className="hud-detail-actions">
        {frameUrl ? (
          <button
            data-testid="cctv-open-popout"
            type="button"
            className="hud-detail-link"
            onClick={() =>
              onOpenPopout({
                id: String(data.id ?? data.label ?? ""),
                name: String(data.name ?? data.label ?? ""),
                city: String(data.city ?? ""),
                lat: typeof data.lat === "number" ? data.lat : undefined,
                lon: typeof data.lon === "number" ? data.lon : undefined,
                headingDeg:
                  typeof data.headingDeg === "number"
                    ? data.headingDeg
                    : undefined,
                fovDeg:
                  typeof data.fovDeg === "number" ? data.fovDeg : undefined,
                pitchDeg:
                  typeof data.pitchDeg === "number"
                    ? data.pitchDeg
                    : undefined,
                feedType: (
                  data.feedType as string | undefined
                ) as PopoutCamera["feedType"],
                provider: String(data.provider ?? ""),
                license: String(data.license ?? ""),
                frameUrl: String(data.frameUrl ?? ""),
                // P12 follow-up: pass upstream mediaUrl through so mp4/
                // hls/webm cameras stream real H.264 video in the popout
                // (skipping the hub proxy). Image cameras omit this.
                mediaUrl:
                  typeof data.mediaUrl === "string" && data.mediaUrl
                    ? data.mediaUrl
                    : undefined,
                live: typeof data.live === "boolean" ? data.live : undefined,
              })
            }
          >
            打开实时画面 →
          </button>
        ) : (
          <span className="hud-detail-link disabled" aria-disabled="true">
            实时画面 不可用
          </span>
        )}
      </div>
    </>
  );
}

function InstallationBody({ data }: { data: Record<string, unknown> }) {
  const retrievedAtMs = data.retrievedAtMs as number | null;
  const sources = (data.sources ?? []) as Array<{ name?: string; id?: string }>;
  const sourceLabel =
    sources.length === 0
      ? "—"
      : sources
          .map((source) => source.name ?? source.id ?? "")
          .filter(Boolean)
          .join(" · ");
  const osmType = String(data.osmType || "");
  const osmId = String(data.osmId || "");
  return (
    <>
      <div className="hud-detail-title">
        {String(data.name || data.label || "—")}
      </div>
      <dl className="hud-detail-fields">
        <dt>类别</dt>
        <dd>{humanizeInstallationClass(String(data.class || ""))}</dd>
        <dt>OSM</dt>
        <dd>
          {osmType && osmId ? `${osmType} / ${osmId}` : String(data.id || "—")}
        </dd>
        <dt>来源</dt>
        <dd>{sourceLabel}</dd>
        <dt>校验</dt>
        <dd>{String(data.validation || "—")}</dd>
        <dt>检索</dt>
        <dd>
          {retrievedAtMs == null ? "—" : formatRelativeTime(retrievedAtMs)}
        </dd>
      </dl>
    </>
  );
}

export interface HudDetailPanelProps {
  /** P7: follow controller (flight/satellite tracking) — null hides the
   *  follow/tilt actions. */
  follow?: FollowHandle | null;
  /** P7: camera orientation adapter — powers the tilt toggle. */
  camera?: CameraOrientationHandle | null;
  /** P10-T3: trailing widgets (drag handle, recording controls, etc.). */
  children?: ReactNode;
}

export function HudDetailPanel({
  follow = null,
  camera = null,
  children,
}: HudDetailPanelProps) {
  const selection = useGlobeSelection();
  const [collapsed, setCollapsed] = useState(false);
  // T14: popout state lives here so the panel survives re-renders
  const [popoutCamera, setPopoutCamera] = useState<PopoutCamera | null>(null);
  // P7 follow/姿态 mirrors. The FollowHandle is the engine-side truth; these
  // states exist only to re-render after user gestures and to hold transient
  // button feedback ("图层未启用" 2 s). Selection changes resync trackedId
  // from the handle below so an already-tracked object that is no longer the
  // selection stays tracked (engine keeps applying the camera frame).
  const [trackedId, setTrackedId] = useState<string | null>(null);
  const [followError, setFollowError] = useState(false);
  const [tiltDown, setTiltDown] = useState(false);
  const followErrorTimer = useRef<number | null>(null);

  useEffect(() => {
    setTrackedId(follow?.trackedId() ?? null);
  }, [selection.kind, selection.data, follow]);

  // Clear the transient feedback timer on unmount (React 18 no longer warns
  // about setState-after-unmount, but a leaked timer is still wasteful).
  useEffect(
    () => () => {
      if (followErrorTimer.current != null) {
        window.clearTimeout(followErrorTimer.current);
      }
    },
    [],
  );

  const isFollowable =
    selection.kind === "flight" || selection.kind === "satellite";
  const rawId =
    isFollowable && selection.data
      ? selection.kind === "flight"
        ? selection.data.id
        : selection.data.noradId
      : null;
  const currentId = rawId == null ? null : String(rawId);
  const isTracking =
    follow != null && currentId !== null && trackedId === currentId;

  const onFollow = () => {
    if (!follow || currentId == null) return;
    const kind = selection.kind as "flight" | "satellite";
    if (follow.follow(kind, currentId)) {
      setTrackedId(currentId);
      setFollowError(false);
    } else {
      setFollowError(true);
      if (followErrorTimer.current != null) {
        window.clearTimeout(followErrorTimer.current);
      }
      followErrorTimer.current = window.setTimeout(
        () => setFollowError(false),
        2000,
      );
    }
  };

  const onUnfollow = () => {
    follow?.unfollow();
    setTrackedId(null);
    setFollowError(false);
  };

  const onTilt = () => {
    if (!camera) return;
    const result = camera.toggleTilt();
    if (result === "down") setTiltDown(true);
    else if (result === "oblique") setTiltDown(false);
  };

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
              {selection.kind === "vessel" && <VesselBody data={selection.data} />}
              {selection.kind === "cctv" && (
                <CctvBody
                  data={selection.data}
                  onOpenPopout={setPopoutCamera}
                />
              )}
              {selection.kind === "installation" && (
                <InstallationBody data={selection.data} />
              )}
              {follow && currentId !== null && (
                <div className="hud-detail-actions">
                  {isTracking ? (
                    <>
                      <button
                        type="button"
                        className="hud-detail-link"
                        data-testid="hud-unfollow-button"
                        onClick={onUnfollow}
                      >
                        解除跟随
                      </button>
                      <button
                        type="button"
                        className="hud-detail-link"
                        data-testid="hud-tilt-button"
                        onClick={onTilt}
                        title={tiltDown ? "切换至斜视视角" : "切换至俯视视角"}
                      >
                        {tiltDown ? "俯视" : "斜视"}
                      </button>
                    </>
                  ) : (
                    <button
                      type="button"
                      className="hud-detail-link"
                      data-testid="hud-follow-button"
                      onClick={onFollow}
                    >
                      {followError ? "图层未启用" : "跟随"}
                    </button>
                  )}
                </div>
              )}
            </>
          )}
        </div>
      )}
      {children}
      {/* T14: face-on 2D CCTV popout */}
      {popoutCamera && (
        <CctvPopoutPanel
          camera={popoutCamera}
          onClose={() => setPopoutCamera(null)}
        />
      )}
    </div>
  );
}
