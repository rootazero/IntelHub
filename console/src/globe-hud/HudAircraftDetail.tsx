// GEV P13 T3 — right-side metadata card for the currently tracked flight.
//
// WHY TWO DATA PATHS (deviation from the plan's §Task 3 interface):
//   The plan assumed `catalog.layers.find(l => l.id === "flights").state`
//   exposes `_trackedIcao` + `records.data`. That object does not exist: the
//   vendor composes the flights layer with `Object.assign(layer,
//   parts.queries.methods, parts.lifecycle.methods, parts.ingestion.methods)`
//   (gev-engine/src/layers/flights/index.js:55-62) and never publishes the
//   internal `flightState` — and the vendor engine is read-only (AGENTS.md
//   "no vendor fork"). The REAL public seam is `layer.getTrackedInfo()`
//   (gev-engine/src/layers/flights/queries.js:840), the same one GlobeV2
//   already feeds to the cockpit instruments.
//   So: `getTrackedInfo` is the production path, `catalog` is kept as the
//   plan's fixture/QA shape (and is what the plan's unit tests pin). The seam
//   wins when both are supplied.
//
// POLLING: the card re-renders on a 1 s interval (a tick, not a data copy) so
// the dead-reckoned altitude/speed/position track the globe. Reading props
// during render keeps the latest values regardless of when the parent last
// re-rendered. The interval is always installed and cleared on unmount.
//
// COCKPIT: hidden while the cockpit owns the screen. The vendor cockpit
// toggles `body.cockpit-mode`; the IntelHub React cockpit store does not (it
// never dispatches `gev:cockpit-mode-changed`), so the parent also passes
// `cockpitActive`. Both signals are honored.
import { useEffect, useState } from "react";
import { useT } from "../i18n";

/** Poll cadence for the tracked-flight readout (ms). */
export const DETAIL_POLL_MS = 1000;

const M_TO_FT = 3.280839895;
const MPS_TO_KT = 1.9438444924; // 1 / 0.514444
const PLACEHOLDER = "—";

/** Shape returned by the vendor `flightsLayer.getTrackedInfo()` seam.
 *  Every field is optional so it stays structurally compatible with the
 *  GlobeV2 `CockpitTrackedInfo` reference (instruments-mount.ts:30) that is
 *  already held in `flightsRef`. */
export interface TrackedFlightInfo {
  icao24?: string | null;
  callsign?: string | null;
  latitude?: number | null;
  longitude?: number | null;
  altitudeM?: number | null;
  velocityMps?: number | null;
  track?: number | null;
  typeCode?: string | null;
  typeName?: string | null;
  registration?: string | null;
  airline?: string | null;
  route?: {
    origin?: { code?: string | null } | null;
    destination?: { code?: string | null } | null;
  } | null;
}

/** Minimal structural view of the plan's catalog fixture shape. */
export interface CatalogLike {
  layers?: Array<{
    id?: string;
    state?: {
      _trackedIcao?: string | null;
      records?: {
        data?: { get?: (key: string) => unknown };
      };
    };
  }> | null;
}

export interface HudAircraftDetailProps {
  /** Production seam: `flightsLayer.getTrackedInfo()`. Preferred source. */
  getTrackedInfo?: (() => TrackedFlightInfo | null) | null;
  /** Plan/QA fixture shape — flights layer `state._trackedIcao` +
   *  `state.records.data`. Only consulted when no seam is supplied. */
  catalog?: CatalogLike | null;
  /** IntelHub cockpit store state. The vendor `body.cockpit-mode` class is
   *  honored as well, so this is additive. */
  cockpitActive?: boolean;
}

/** Normalized view model — the single shape the render reads. */
interface FlightView {
  hex: string;
  callsign: string | null;
  typeLabel: string | null;
  registration: string | null;
  airline: string | null;
  routeOrigin: string | null;
  routeDestination: string | null;
  latitude: number | null;
  longitude: number | null;
  altitudeM: number | null;
  velocityMps: number | null;
  track: number | null;
}

type Resolution =
  | { kind: "none" }
  | { kind: "evicted" }
  | { kind: "tracked"; flight: FlightView };

function clean(value: unknown): string | null {
  const s = String(value ?? "").trim();
  return s || null;
}

function finite(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function routeCode(
  route: TrackedFlightInfo["route"],
  end: "origin" | "destination",
): string | null {
  return clean(route?.[end]?.code);
}

function fromSeam(info: TrackedFlightInfo): FlightView {
  return {
    hex: clean(info.icao24) ?? PLACEHOLDER,
    callsign: clean(info.callsign),
    // Prefer the human name; the ICAO type code is the fallback (vendor
    // queries.js:180 applies the TR-3B class conversion to typeName).
    typeLabel: clean(info.typeName) ?? clean(info.typeCode),
    registration: clean(info.registration),
    airline: clean(info.airline),
    routeOrigin: routeCode(info.route, "origin"),
    routeDestination: routeCode(info.route, "destination"),
    latitude: finite(info.latitude),
    longitude: finite(info.longitude),
    altitudeM: finite(info.altitudeM),
    velocityMps: finite(info.velocityMps),
    track: finite(info.track),
  };
}

/** Plan §Task 3 fixture path — the record keys are the vendor poll record's
 *  (`hex`, `baroAltitudeM`, `speedMps`, `courseDeg`). */
function fromCatalog(catalog: CatalogLike | null | undefined): Resolution {
  const layer = catalog?.layers?.find((l) => l?.id === "flights");
  const icao = clean(layer?.state?._trackedIcao);
  if (!icao) return { kind: "none" };
  const record = layer?.state?.records?.data?.get?.(icao) as
    | Record<string, unknown>
    | undefined;
  if (!record) return { kind: "evicted" };
  const route = record.route as TrackedFlightInfo["route"];
  return {
    kind: "tracked",
    flight: {
      hex: clean(record.hex) ?? icao,
      callsign: clean(record.callsign),
      typeLabel: clean(record.typeName) ?? clean(record.typeCode),
      registration: clean(record.registration),
      airline: clean(record.airline),
      routeOrigin: routeCode(route, "origin"),
      routeDestination: routeCode(route, "destination"),
      latitude: finite(record.latitude),
      longitude: finite(record.longitude),
      altitudeM: finite(record.baroAltitudeM) ?? finite(record.altitudeM),
      velocityMps: finite(record.speedMps) ?? finite(record.velocityMps),
      track: finite(record.courseDeg) ?? finite(record.track),
    },
  };
}

function resolve(props: HudAircraftDetailProps): Resolution {
  if (typeof props.getTrackedInfo === "function") {
    const info = props.getTrackedInfo();
    // The seam returns null for both "nothing tracked" and "tracked record
    // gone"; with a seam present the fixture catalog is out of scope.
    return info ? { kind: "tracked", flight: fromSeam(info) } : { kind: "none" };
  }
  return fromCatalog(props.catalog);
}

function cockpitModeActive(): boolean {
  return (
    typeof document !== "undefined" &&
    document.body.classList.contains("cockpit-mode")
  );
}

function Placeholder() {
  return (
    <span className="hud-aircraft-detail__placeholder">{PLACEHOLDER}</span>
  );
}

function Value({ children }: { children: string | null }) {
  return children == null ? <Placeholder /> : <>{children}</>;
}

function formatFeet(metres: number | null): string | null {
  return metres == null
    ? null
    : `${Math.round(metres * M_TO_FT).toLocaleString("en-US")} ft`;
}

function formatKnots(mps: number | null): string | null {
  return mps == null
    ? null
    : `${Math.round(mps * MPS_TO_KT).toLocaleString("en-US")} kt`;
}

function formatHeading(deg: number | null): string | null {
  return deg == null ? null : `${Math.round(deg)}°`;
}

function formatCoords(lat: number | null, lon: number | null): string | null {
  if (lat == null || lon == null) return null;
  return `${lat.toFixed(4)}, ${lon.toFixed(4)}`;
}

export function HudAircraftDetail({
  getTrackedInfo = null,
  catalog = null,
  cockpitActive = false,
}: HudAircraftDetailProps) {
  const { t } = useT();
  // Tick-only state: the interval forces a re-render so the render below
  // re-reads the (possibly dead-reckoned) seam values. No data is copied.
  const [, setTick] = useState(0);

  useEffect(() => {
    const id = setInterval(() => setTick((n) => n + 1), DETAIL_POLL_MS);
    return () => clearInterval(id);
  }, []);

  if (cockpitActive || cockpitModeActive()) return null;

  const resolution = resolve({ getTrackedInfo, catalog });
  if (resolution.kind !== "tracked") {
    return (
      <aside
        className="hud-aircraft-detail hud-aircraft-detail--empty"
        data-testid="hud-aircraft-detail-empty"
      >
        {resolution.kind === "evicted"
          ? t("aircraft.detail.evicted")
          : t("aircraft.detail.empty")}
      </aside>
    );
  }

  const { flight } = resolution;
  const routeLabel =
    flight.routeOrigin && flight.routeDestination
      ? `${flight.routeOrigin} → ${flight.routeDestination}`
      : null;

  return (
    <aside
      className="hud-aircraft-detail"
      data-testid="hud-aircraft-detail"
      role="complementary"
      aria-label={t("aircraft.detail.title")}
    >
      <header className="hud-aircraft-detail__header">
        <span className="hud-aircraft-detail__title">
          {t("aircraft.detail.title")}
        </span>
        <span className="hud-aircraft-detail__hex">
          {flight.hex.toUpperCase()}
        </span>
      </header>
      <div className="hud-aircraft-detail__callsign">
        <Value>{flight.callsign}</Value>
      </div>
      <dl className="hud-aircraft-detail__body">
        <dt>{t("aircraft.detail.aircraft")}</dt>
        <dd>
          <Value>{flight.typeLabel}</Value>
        </dd>
        <dt>{t("aircraft.detail.registration")}</dt>
        <dd>
          <Value>{flight.registration}</Value>
        </dd>
        <dt>{t("aircraft.detail.airline")}</dt>
        <dd>
          <Value>{flight.airline}</Value>
        </dd>
        <dt>{t("aircraft.detail.route")}</dt>
        <dd>
          <Value>{routeLabel}</Value>
        </dd>
        <dt>{t("aircraft.detail.altitude")}</dt>
        <dd>
          <Value>{formatFeet(flight.altitudeM)}</Value>
        </dd>
        <dt>{t("aircraft.detail.speed")}</dt>
        <dd>
          <Value>{formatKnots(flight.velocityMps)}</Value>
        </dd>
        <dt>{t("aircraft.detail.heading")}</dt>
        <dd>
          <Value>{formatHeading(flight.track)}</Value>
        </dd>
        <dt>{t("aircraft.detail.coords")}</dt>
        <dd>
          <Value>{formatCoords(flight.latitude, flight.longitude)}</Value>
        </dd>
      </dl>
    </aside>
  );
}
