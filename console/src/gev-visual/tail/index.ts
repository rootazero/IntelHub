// GEV P10 T2 — adapter index. Re-exports 9 adapter handles + types so
// GlobeV2's T3 wiring can do `import { mountPanelDrag, ... } from "./tail"`.
//
// Mirrors the gev-visual/cockpit/index.ts public-surface convention: every
// handle factory + its options + its types in one place.

export {
  mountPanelDrag,
  type PanelDragHandle,
  type PanelDragOptions,
  type PanelDragDeps,
} from "./panel-drag";

export {
  mountPanelLayout,
  layoutLeftPanelRail,
  layoutRightPanelRail,
  type PanelLayoutHandle,
  type PanelLayoutOptions,
  type PanelLayoutLeftOptions,
  type PanelLayoutRightOptions,
  type PanelLayoutCallbacks,
  type PanelHudPresentation,
} from "./panel-layout";

export {
  bindPanelDisclosure,
  collapsePanelOnEscape,
  createHoverDisclosure,
  type PanelDisclosureHandle,
  type HoverDisclosureHandle,
  type BindDisclosureOptions,
  type CollapseOnEscapeOptions,
  type HoverDisclosureOptions,
} from "./panel-disclosure";

export {
  mountSceneControls,
  type SceneControlsHandle,
  type SceneControlsOptions,
  type SceneProjectState,
  type SceneActions,
  type SceneElements,
  type SceneChangeNotification,
  type SceneChangeType,
} from "./scene-controls";

export {
  createSceneDialog,
  mountSceneSharing,
  type SceneDialogHandle,
  type SceneSharingMountHandle,
  type SceneSharingOptions,
} from "./scene-sharing";

export {
  useShareRestoration,
  decodeShareHash,
  encodeShareState,
  type ShareState,
  type UseShareRestorationResult,
  type CameraState,
  type TrackingState,
  type TrackingKind,
  type LayoutState,
  type LayoutPanel,
} from "./use-share-restoration";

export {
  mountRecordingControls,
  type RecordingControlsHandle,
  type RecordingControlsOptions,
  type RecordingModeOptions,
  type RecordingHud,
  type RecordingOverlayElements,
} from "./recording-controls";

export {
  bindShortcuts,
  type ShortcutsHandle,
  type ShortcutsOptions,
  type ShortcutsActions,
  type ShortcutsStyleSetter,
} from "./shortcuts";

export {
  mountFrameRateMonitor,
  type FrameRateMonitorHandle,
  type FrameRateMonitorOptions,
  type FrameRateViewer,
} from "./frame-rate-monitor";