// GEV P9 — cockpit adapters public surface (plan Task 3).
//
// Four adapters + the store, mirroring the P7/P8 re-export convention
// (gev-visual/annotations/index.ts): every handle factory + its types.

export {
  mountCockpitInstruments,
  type CockpitTrackedInfo,
  type CockpitInstrumentFrame,
  type CompassDivision,
  type RulerTick,
  type InstrumentsHandle,
} from "./instruments-mount";

export {
  mountCockpitBriefing,
  type WeatherMetricKey,
  type WeatherMetric,
  type SummaryBullet,
  type Briefing,
  type BriefingHandle,
} from "./briefing-mount";

export {
  mountCockpitVision,
  VISION_MODES,
  VISION_MODE_STORAGE_KEY,
  isVisionMode,
  type VisionMode,
  type VisionMountHandle,
} from "./vision-mount";

export {
  createCockpitStore,
  cockpitReducer,
  INITIAL_COCKPIT_STATE,
  type CockpitStoreState,
  type CockpitAction,
  type CockpitStore,
} from "./cockpit-store";

export {
  gateStyleWhileCockpitActive,
  type StyleEffects,
  type GatedStyleControl,
} from "./style-gate";

export {
  useCockpitShortcuts,
  type CockpitShortcutsOptions,
} from "./shortcuts";

export {
  mountCockpitCameraTransition,
  type CockpitCameraTransition,
  type CockpitCameraTransitionDeps,
  type CockpitCameraTarget,
  type CockpitCameraFlyOptions,
} from "./camera-transition";

export {
  mountModelVisibility,
  type ModelVisibilityDeps,
  type ModelVisibilityHandle,
} from "./model-visibility";

export {
  mountCockpitChaseCam,
  type ChaseCamDeps,
  type ChaseCamHandle,
} from "./chase-cam";

export {
  mountCockpitViewportLock,
  type CockpitViewportLock,
  type CockpitViewportLockViewer,
} from "./viewport-lock";

export {
  mountCockpitMouseLook,
  MOUSE_LOOK_ZERO_OFFSET,
  type MouseLookHandle,
  type MouseLookDeps,
  type MouseLookFrameOffset,
  type MouseLookStoreShape,
} from "./mouse-look";

// GEV P16 — cockpit HUD avionics (10Hz update timer + chase-cam attitude)
export {
  mountCockpitHudTick,
  type HudTickHandle,
  type HudTickOptions,
} from "./cockpit-hud-tick";

// GEV P16 — chase-cam attitude state (bank from Quaternion→roll; VSI from altitude window)
export type { ChaseCamResolvedState } from "./chase-cam";

// GEV P16 — instruments-mount deps bag (already re-exported above)
export type { CockpitInstrumentDeps } from "./instruments-mount";

// GEV P17 — cockpit HUD element visibility (per-element show/hide)
export {
  COCKPIT_ELEMENT_KEYS,
  ELEMENT_VISIBILITY_STORAGE_KEY,
  DEFAULT_ELEMENT_VISIBILITY,
  isCockpitElementKey,
  readPersistedElementVisibility,
  persistElementVisibility,
  type CockpitElementKey,
  type ElementVisibility,
} from "./element-visibility";
