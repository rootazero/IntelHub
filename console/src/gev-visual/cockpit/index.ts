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
