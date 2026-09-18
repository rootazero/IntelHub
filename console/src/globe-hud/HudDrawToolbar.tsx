// GEV P8 — manual drawing toolbar (pin/line/area) over the globe canvas.
//
// Self-mounts the draw-tool adapter + annotation store lazily (the adapters
// pull the vendored engine, which the rest of the HUD chrome never needs) via
// the same async-import + `cancelled` guard pattern HudTopBar uses for its
// location search (P7 lesson: subscriptions must be set up AFTER the async
// resolve, never before).
//
// Screen→world conversion: the HUD converts a canvas click through
// `viewer.scene.pickPosition(Cartesian2)` → Cartesian3, then converts to
// lon/lat via the Cesium singleton the vendored engine installs at module
// scope (`window.__CESIUM__ = Cesium`, annotationEngine.js:16-17). The toolbar
// needs Cesium ONLY for that one conversion; everything else flows through the
// pure draw-tool adapter (drawMode.js is importable under node — no Cesium).
import {
  useEffect,
  useRef,
  useState,
  type MouseEvent as ReactMouseEvent,
} from "react";
import type {
  AnnotationSpec,
  AnnotationStore,
  ApiFetch,
  DrawMode,
  DrawToolHandle,
  PickableViewer,
} from "../gev-visual/annotations";

interface HudDrawToolbarProps {
  viewer: unknown;
  apiFetch: ApiFetch;
  onCreated?: (spec: AnnotationSpec) => void;
  onError?: (msg: string) => void;
  visible: boolean;
  onClose: () => void;
}

/** Minimal Cesium surface the toolbar reads off `window.__CESIUM__`. */
interface CesiumGlobal {
  Cartographic: {
    fromCartesian: (
      cartesian: unknown,
    ) => { longitude: number; latitude: number } | undefined;
  };
  Math: { toDegrees: (radians: number) => number };
}

export function HudDrawToolbar({
  viewer,
  apiFetch,
  onCreated,
  onError,
  visible,
  onClose,
}: HudDrawToolbarProps) {
  const [mode, setMode] = useState<DrawMode>(null);
  const [vertices, setVertices] = useState<Array<{ lon: number; lat: number }>>(
    [],
  );
  const [label, setLabel] = useState("");
  const handleRef = useRef<DrawToolHandle | null>(null);
  const storeRef = useRef<AnnotationStore | null>(null);

  useEffect(() => {
    if (!visible || !viewer) return;
    let cancelled = false;
    let handle: DrawToolHandle | null = null;
    let unsubPreview: (() => void) | null = null;
    (async () => {
      try {
        const [{ mountDrawTool }, { createAnnotationStore }] =
          await Promise.all([
            import("../gev-visual/annotations/draw-tool"),
            import("../gev-visual/annotations/annotation-store"),
          ]);
        if (cancelled) return;
        handle = mountDrawTool(viewer as PickableViewer);
        // Subscribe AFTER the async resolve (never before): handle is null until
        // the import settles, so a pre-resolve subscription would be a no-op.
        unsubPreview = handle.onPreview(setVertices);
        storeRef.current = createAnnotationStore(apiFetch);
        handleRef.current = handle;
      } catch (e) {
        if (!cancelled) onError?.(`draw-tool init failed: ${String(e)}`);
      }
    })();
    return () => {
      cancelled = true;
      unsubPreview?.();
      handle?.destroy();
      handleRef.current = null;
      storeRef.current = null;
    };
  }, [visible, viewer, apiFetch]);

  if (!visible) return null;

  const onPickMode = (m: "pin" | "line" | "area") => {
    setMode(m);
    setVertices([]);
    handleRef.current?.start(m);
  };

  const onCanvasClick = (e: ReactMouseEvent<HTMLDivElement>) => {
    if (!mode || !handleRef.current) return;
    const rect = e.currentTarget.getBoundingClientRect();
    const win = { x: e.clientX - rect.left, y: e.clientY - rect.top };
    const scene = (
      viewer as {
        scene?: { pickPosition?: (win: { x: number; y: number }) => unknown };
      }
    ).scene;
    const cartesian = scene?.pickPosition?.(win);
    if (!cartesian) {
      onError?.("无法在该视角下选点，请调整视角");
      return;
    }
    const Cesium = (window as unknown as { __CESIUM__?: CesiumGlobal })
      .__CESIUM__;
    if (!Cesium) {
      onError?.("Cesium 未就绪，无法换算坐标");
      return;
    }
    const carto = Cesium.Cartographic.fromCartesian(cartesian);
    if (!carto) return;
    const lon = Cesium.Math.toDegrees(carto.longitude);
    const lat = Cesium.Math.toDegrees(carto.latitude);
    handleRef.current.addClickWorld(lon, lat);
  };

  const reset = () => {
    setMode(null);
    setVertices([]);
    setLabel("");
  };

  const onFinish = async () => {
    const spec = handleRef.current?.finish({ label, persist: true });
    if (!spec || !storeRef.current) return;
    try {
      const saved = await storeRef.current.create(spec);
      reset();
      onCreated?.(saved);
      onClose();
    } catch (e) {
      onError?.(`保存失败: ${String(e)}`);
    }
  };

  const onCancel = () => {
    handleRef.current?.cancel();
    reset();
    onClose();
  };

  const minVertices = mode === "pin" ? 1 : mode === "line" ? 2 : 3;

  return (
    <div
      data-testid="hud-draw-toolbar"
      className="hud-draw-toolbar"
      onClick={onCanvasClick}
    >
      <div className="hud-draw-mode-row">
        <button
          type="button"
          data-testid="hud-draw-mode-pin"
          title="图钉 / Pin"
          onClick={(e) => {
            e.stopPropagation();
            onPickMode("pin");
          }}
        >
          📍
        </button>
        <button
          type="button"
          data-testid="hud-draw-mode-line"
          title="折线 / Line"
          onClick={(e) => {
            e.stopPropagation();
            onPickMode("line");
          }}
        >
          〰️
        </button>
        <button
          type="button"
          data-testid="hud-draw-mode-area"
          title="区域 / Area"
          onClick={(e) => {
            e.stopPropagation();
            onPickMode("area");
          }}
        >
          ⬡
        </button>
      </div>
      <input
        data-testid="hud-draw-label"
        placeholder="标签"
        value={label}
        onChange={(e) => setLabel(e.target.value)}
        onClick={(e) => e.stopPropagation()}
      />
      <div className="hud-draw-preview">
        顶点 {vertices.length} ({mode ?? "未选"})
      </div>
      <button
        type="button"
        data-testid="hud-draw-finish"
        disabled={vertices.length < minVertices}
        onClick={(e) => {
          e.stopPropagation();
          void onFinish();
        }}
      >
        保存
      </button>
      <button
        type="button"
        data-testid="hud-draw-cancel"
        onClick={(e) => {
          e.stopPropagation();
          onCancel();
        }}
      >
        取消
      </button>
    </div>
  );
}
