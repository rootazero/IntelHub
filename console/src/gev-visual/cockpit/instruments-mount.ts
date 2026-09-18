// GEV P9 cockpit instruments adapter — a pure-computation seam over the
// vendored cockpitMath helpers (compass / altitude / speed rulers).
//
// The vendored cockpitInstruments.js exports METHOD-based functions
// (updateHud / updateRoute / setVisionMode / cycleVisionMode /
// syncWeatherToggle / clearPredictiveRoute) bound to a controller `this` full
// of DOM refs. The React HUD renders its own SVG (T4), so this adapter does
// NOT wrap those methods (D1: 不实例化 cockpitController). It extracts the
// SAME math the vendor's updateHud applies — the 1.94384 m/s→knots conversion,
// cockpitAltitudeDisplayFt, compassDivisions, altitude/speed ruler ticks — into
// a pure frame the HUD reads every 4 Hz poll.
//
// Real data source: `flights.getTrackedInfo()` (layers/flights/queries.js)
// returns `{ icao24, callsign, latitude, longitude, altitudeM, velocityMps,
// track, onGround, registration, layerId, stale }`. Track is degrees (heading
// = track, same field); velocity is m/s (no conversion needed on input).
import {
  normalizeHeading,
  cockpitAltitudeDisplayFt,
  formatAltitudeRulerTick,
  formatSpeedRulerTick,
  compassDivisions,
  formatCompassDivision,
  altitudeRulerTicks,
  altitudeRulerCurveInset,
  speedRulerTicks,
} from "gev-engine/src/cockpitMath.js";

/** Tracked-aircraft info as reported by flights.getTrackedInfo(). */
export interface CockpitTrackedInfo {
  icao24?: string;
  callsign?: string | null;
  registration?: string | null;
  latitude?: number;
  longitude?: number;
  altitudeM?: number;
  velocityMps?: number | null;
  track?: number | null;
  onGround?: boolean;
  layerId?: string;
  stale?: boolean;
}

export interface CompassDivision {
  division: number;
  label: string;
  /** -3..+3 slot relative to the center pointer. */
  slot: number;
  depth: number;
  active: boolean;
}

export interface RulerTick {
  /** valueFt (altitude) or valueKt (speed). */
  value: number;
  slot: number;
  depth: number;
  major: boolean;
  label: string;
  /** altitudeRulerCurveInset(slot) — horizontal inset for the keyhole rim. */
  curve: number;
}

export interface CockpitInstrumentFrame {
  heading: number;
  headingLabel: string;
  altitudeFt: number | null;
  altitudeLabel: string;
  speedKt: number | null;
  speedLabel: string;
  callsign: string;
  compass: CompassDivision[];
  altitudeTicks: RulerTick[];
  speedTicks: RulerTick[];
}

export interface InstrumentsHandle {
  /** Compute the instrument frame. `info` defaults to `flights.getTrackedInfo()`
   *  when omitted (null → a neutral "AIRCRAFT" / dashed frame). */
  update(info?: CockpitTrackedInfo | null): CockpitInstrumentFrame;
  getFrame(): CockpitInstrumentFrame | null;
  destroy(): void;
}

/** The vendor updateHud knot conversion (cockpitInstruments.js). */
const MPS_TO_KTS = 1.94384;
/** Matches cockpitMath.altitudeRulerTicks/speedRulerTicks default. */
const RULER_TICK_COUNT = 9;

export function mountCockpitInstruments(
  viewer: { scene: { canvas?: unknown } },
  flights?: { getTrackedInfo(): CockpitTrackedInfo | null },
): InstrumentsHandle {
  // Constructor contract (P3 lesson): the HUD hands the adapter a Cesium
  // viewer, whose scene.canvas is the render seam the cockpit consumes. Assert
  // the shape so a lenient mock can't hide a wrong object handed in later.
  if (!viewer?.scene || !("canvas" in viewer.scene)) {
    throw new TypeError(
      "mountCockpitInstruments: viewer.scene.canvas missing",
    );
  }

  let lastFrame: CockpitInstrumentFrame | null = null;
  let destroyed = false;

  function compute(info: CockpitTrackedInfo | null): CockpitInstrumentFrame {
    const heading = normalizeHeading(info?.track ?? 0);
    const altitudeFt = info
      ? cockpitAltitudeDisplayFt(info?.altitudeM, info?.onGround)
      : null;
    const speedKt =
      info && Number.isFinite(info.velocityMps)
        ? (info.velocityMps as number) * MPS_TO_KTS
        : null;
    const callsign = info
      ? info.callsign || info.registration || info.icao24 || "AIRCRAFT"
      : "AIRCRAFT";

    const compass = compassDivisions(heading).map(
      (division: number, index: number): CompassDivision => {
        const slot = index - 3;
        return {
          division,
          label: formatCompassDivision(division),
          slot,
          depth: Math.abs(slot),
          active: slot === 0,
        };
      },
    );

    const altitudeTicks = altitudeRulerTicks(
      altitudeFt,
      RULER_TICK_COUNT,
    ).map(
      (t: { valueFt: number; slot: number; depth: number; major: boolean }): RulerTick => ({
        value: t.valueFt,
        slot: t.slot,
        depth: t.depth,
        major: t.major,
        label: formatAltitudeRulerTick(t.valueFt),
        curve: altitudeRulerCurveInset(t.slot),
      }),
    );

    const speedTicks = speedRulerTicks(speedKt, RULER_TICK_COUNT).map(
      (t: { valueKt: number; slot: number; depth: number; major: boolean }): RulerTick => ({
        value: t.valueKt,
        slot: t.slot,
        depth: t.depth,
        major: t.major,
        label: formatSpeedRulerTick(t.valueKt),
        curve: altitudeRulerCurveInset(t.slot),
      }),
    );

    return {
      heading,
      headingLabel: String(Math.round(heading) % 360).padStart(3, "0"),
      altitudeFt,
      altitudeLabel:
        altitudeFt !== null
          ? Math.round(altitudeFt).toLocaleString("en-US")
          : "-----",
      speedKt,
      speedLabel: speedKt !== null ? formatSpeedRulerTick(speedKt) : "---",
      callsign,
      compass,
      altitudeTicks,
      speedTicks,
    };
  }

  return {
    update(info?: CockpitTrackedInfo | null) {
      const resolved =
        info !== undefined ? info : (flights?.getTrackedInfo() ?? null);
      lastFrame = compute(resolved);
      return lastFrame;
    },
    getFrame: () => (destroyed ? null : lastFrame),
    destroy() {
      destroyed = true;
      lastFrame = null;
    },
  };
}
