# GEV P6 视觉预设（6 套 GLSL 滤镜进 HUD）Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 GEV 引擎的 6 套 GLSL 视觉滤镜（retro CRT / surveillance NVG / thermal FLIR / anime / noir / snow）+ bloom/sharpen 联动 + 500ms crossfade 接入 IntelHub Globe 页面，HUD 顶条新增滤镜切换器，选择持久化到 localStorage。

**Architecture:** 渲染核直引 + React 壳。从 vendor `gev-engine/src/ui/visualEffects.js`（`VisualEffects` class，纯依赖注入、零 DOM 耦合）与 `visualPresets.js`（纯数据）直接 import；新增适配层 `console/src/gev-visual/visual-effects.ts` 封装 mount/setStyle/destroy 窄契约；React 壳 `console/src/globe-hud/HudStyleSwitcher.tsx` 挂在 HudTopBar。**不**实例化 `VisualSettings` class（DOM/shell 深度耦合），**不**接线 detection/worldOverlay/scopeMask。

**Tech Stack:** React 18 + TypeScript + Vite + vitest(jsdom) + Cesium PostProcessStage + playwright（probe）。

**Spec:** `docs/superpowers/specs/2026-09-18-gev-visual-port-design.md`（§3 P6 行；detection 已移除，见 §0 决策表）

## Global Constraints

- **vendor 纯净只读**：`console/gev-engine/` 内零修改；只新增 import 关系。
- **严禁 scopeMask / 圆形视野遮罩 / 周边压黑**：保持开放全景渲染。
- **严禁实例化 `VisualSettings`**（visualSettings.js）：其构造器要求 shell DOM 元素。只用 `VisualEffects` + `visualPresets`。
- **不 install renderGovernor**：P6 不给引擎装 render governor（避免全局渲染行为变更）；adapter 的 holdRender/releaseRender 传 noop。
- **mock 必须复刻真实构造器契约**（P3 lenient-mock 教训）：fake viewer/stage 工厂断言参数形状，缺失即抛 TypeError。
- **localStorage key 风格**：`intelhub.globe.style`（点分域名式，跟随 `intelhub.hud.visuals` 惯例；不要用 vendor 的 `gev:*` 命名空间）。
- **GLSL 无需 loader**：六个 shader 是 JS 模块内联模板字符串（`console/gev-engine/src/styles/*.js`），vite/vitest 现有配置直接可 import，不许动 vite.config.ts / vitest.config.ts 的 alias/define。
- **生命周期纪律**：adapter 必须在引擎 `start()` resolve 之后创建，在 `globe.destroy()` 之前销毁；StrictMode 双挂载安全。
- **每期必走部署序列**：315 验收全绿 → merge main → 410 部署 → 生产验收 → push。worktree 隔离开发。
- 验收脚本扩展落点是 `console/probe-gev.mjs`（sp8 只覆盖 hub 侧数据，不管 console 视觉面）。

## 关键事实（探索验证过，executor 不必重查）

- `VisualEffects` 构造器（visualEffects.js:19-53）全依赖注入：`{ viewer, requestRender, holdRender, releaseRender, requestFrame?, cancelFrame?, now?, wallNow?, createStage? }`。
- 方法（行号）：`initStyles()` :55（幂等，建 6 个 `godsEyeView_<name>` stage 加入 `viewer.scene.postProcessStages`，enabled=false）；`initPostProcess()` :75（快照并借用 Cesium 内建 bloom + 建 `godsEyeView_sharpen`）；`setStageIntensity(stage, value)` :111（>0.001 才 enable）；`startTransition(styleName, from, to)` :170（500ms easeInOutQuad crossfade，TRANSITION_DURATION_MS=500）；`applyBloomIntensity(0-200)` :132；`setBloomEnabled(bool)` :148；`applySharpenIntensity(0-1)` :155（amount=0.1+v*2.0）；`setSharpenEnabled(bool)` :163；`stop()` :213；`destroy()` :223（幂等，remove 全部自建 stage + 恢复 bloom 快照）。
- `visualPresets.js` 导出：`TRANSITION_DURATION_MS=500`、`STYLES`（key: retro/surveillance/thermal/anime/noir/snow）、`STYLE_PRESET_DEFAULTS`（**仅 retro/surveillance/thermal 三个 key**，shape `{ bloom:{enabled,intensity}, sharpen:{enabled,intensity}, styleParams, hudVariant, hudVisible, detection }`）、`GLOBAL_POST_DEFAULTS`（sharpen `{enabled:true, intensity:49}`，bloom `{enabled:false, intensity:0}`）、`STYLE_STATUS_LABELS`（`{normal:'NORMAL',retro:'CRT',surveillance:'NVG',thermal:'FLIR',anime:'ANIME',noir:'NOIR',snow:'SNOW'}`）、`SHARPEN_SHADER`、`MILITARY_DETECTION_PRESET`。
- 'normal' 风格 = 无 stage（所有 style stage intensity 0）。
- adapter 需要读 stage 当前强度做渐变起点：`VisualEffects` 实例的 `this.stages[name].uniforms.intensity`（内部字段，wildcard d.ts 全 any，可直接访问）。
- GlobeV2 拿 viewer：`globe.start()` resolve 后 `getComponents().scene.viewer`（GlobeV2.tsx:73-85）；destroy 序列在 cleanup（:90-99）；模块级 `booted` guard 防 StrictMode 双 boot（:31）。
- HudTopBar 控件写法范本：HudTopBar.tsx:84-93（`button.hud-bar-back` + `data-testid` + 内联双语 title）；条件 class 范本 :109-110。
- vitest Cesium fake 范本：globe-hud/__tests__/hud-live-lanes.test.tsx:97-180（fakeCesium + 构造器形状断言 + 手动 rAF 队列）；VisualEffects 单测 fixture 范本：vendor `src/ui/visualEffects.test.mjs:9-58`（纯 JS 假件，jsdom 下可跑）。
- 契约守卫现有模式：source-contracts.test.ts c1 段（:100-122 锚 identifier 正则，从不锚行号）；工具函数 `readVendor(rel)` :34-36。
- probe-gev.mjs 断言机制：`gate(selector, timeoutMs, label)` :104-119；`failures`/`warnings` 收集器 :399-425；rail-toggle 交互冒烟范本 :310-390。probe 目前无 screenshot，本期新增。

---

### Task 1: worktree + 契约守卫 import-surface 扩展

**Files:**
- Modify: `console/src/gev-boot/__tests__/source-contracts.test.ts`（c1 describe 内追加一个 test）

**Interfaces:**
- Consumes: 现有 `readVendor(rel)` helper（source-contracts.test.ts:34-36）。
- Produces: vendor 渲染核 export 钉扎——后续任务 import 的每个符号被守卫锁定；上游同步改动这些导出时守卫红。

- [ ] **Step 1: 建 worktree**

```bash
cd /Volumes/TBU/Workspace/IntelHub && git worktree add ../IntelHub-gev-p6 -b feat/gev-p6-visual-presets && sleep 4
```

后续所有改动在 `/Volumes/TBU/Workspace/IntelHub-gev-p6` 内进行。

- [ ] **Step 2: 写契约钉扎 test（c1 段内追加）**

```ts
test("visual presets render-core exports are pinned", () => {
  const presets = readVendor("src/ui/visualPresets.js");
  expect(presets).toMatch(/export const TRANSITION_DURATION_MS = 500/);
  expect(presets).toMatch(/export const STYLES\b/);
  expect(presets).toMatch(/export const STYLE_PRESET_DEFAULTS\b/);
  expect(presets).toMatch(/export const STYLE_STATUS_LABELS\b/);
  expect(presets).toMatch(/export const SHARPEN_SHADER\b/);
  for (const [key, shader] of [
    ["retro", "retroShader"],
    ["surveillance", "nightVisionShader"],
    ["thermal", "thermalShader"],
    ["anime", "animeShader"],
    ["noir", "noirShader"],
    ["snow", "snowShader"],
  ] as const) {
    expect(presets, `STYLES.${key}`).toMatch(new RegExp(`${key}\\s*:\\s*${shader}\\b`));
  }
  const fx = readVendor("src/ui/visualEffects.js");
  expect(fx).toMatch(/export class VisualEffects\b/);
  for (const m of [
    "initStyles", "initPostProcess", "setStageIntensity", "startTransition",
    "applyBloomIntensity", "setBloomEnabled", "applySharpenIntensity",
    "setSharpenEnabled", "stop", "destroy",
  ]) {
    expect(fx, `VisualEffects.${m}`).toMatch(new RegExp(`\\n  ${m}\\(`));
  }
  const bloom = readVendor("src/bloom.js");
  for (const e of ["BLOOM_INTENSITY_DEFAULT", "clampBloomIntensity", "bloomStrengthFromIntensity"]) {
    expect(bloom).toMatch(new RegExp(`export (const|function) ${e}\\b`));
  }
  for (const s of ["retro", "surveillance", "thermal", "anime", "noir", "snow"]) {
    expect(readVendor(`src/styles/${s}.js`)).toMatch(/export const \w+Shader\b/);
  }
});
```

- [ ] **Step 3: 跑守卫验证（应直接绿——钉扎的是现状）**

Run: `cd /Volumes/TBU/Workspace/IntelHub-gev-p6/console && npx vitest run src/gev-boot/__tests__/source-contracts.test.ts`
Expected: PASS（若红，说明 vendor 已漂移，停下排查，不要改 test 迁就）

- [ ] **Step 4: Commit**

```bash
cd /Volumes/TBU/Workspace/IntelHub-gev-p6 && git add console/src/gev-boot/__tests__/source-contracts.test.ts && git commit -m "test(gev-boot): pin visual-presets render-core exports in contract guard"
```

---

### Task 2: gev-visual 适配器 `mountVisualEffects`

**Files:**
- Create: `console/src/gev-visual/visual-effects.ts`
- Test: `console/src/gev-visual/__tests__/visual-effects.test.ts`

**Interfaces:**
- Consumes: vendor `VisualEffects`（构造器与方法签名见"关键事实"）。
- Produces（Task 3/4 依赖）：
  - `GLOBE_STYLES: readonly ["normal","retro","surveillance","thermal","anime","noir","snow"]`
  - `type GlobeStyle = (typeof GLOBE_STYLES)[number]`
  - `interface VisualEffectsHandle { setStyle(style: GlobeStyle): void; getStyle(): GlobeStyle; destroy(): void }`
  - `mountVisualEffects(viewer: ViewerLike, deps?: VisualEffectsDeps, initialStyle?: GlobeStyle): VisualEffectsHandle`（initialStyle 即时应用无渐变；destroy 幂等）
  - `interface VisualEffectsDeps { createStage?; requestFrame?; cancelFrame?; now? }`（测试注入点）

- [ ] **Step 1: 写失败测试**

`console/src/gev-visual/__tests__/visual-effects.test.ts`：

```ts
import { describe, expect, test, vi } from "vitest";
import { mountVisualEffects, GLOBE_STYLES } from "../visual-effects";

// Fake shape mirrors vendor visualEffects.test.mjs:9-58, plus the P3
// lenient-mock guard: the factory asserts the shape it is handed.
function fakeViewer() {
  const stages: any[] = [];
  const bloom = {
    enabled: true,
    uniforms: { glowOnly: false, contrast: 128, brightness: -0.6, delta: 1.0, sigma: 2.0, stepSize: 5.0 },
  };
  return {
    scene: {
      requestRender: vi.fn(),
      postProcessStages: {
        bloom,
        add(stage: any) {
          if (!stage || typeof stage !== "object" || !("fragmentShader" in stage)) {
            throw new TypeError("postProcessStages.add: stage must carry fragmentShader");
          }
          stages.push(stage);
        },
        remove(stage: any) {
          const i = stages.indexOf(stage);
          if (i >= 0) stages.splice(i, 1);
        },
        get length() { return stages.length; },
        byName(name: string) { return stages.filter((s) => s.name === name); },
      },
    },
  };
}

function manualFrames() {
  const queue: Array<{ id: number; cb: (t: number) => void }> = [];
  let nextId = 1;
  let now = 0;
  return {
    deps: {
      createStage: (options: any) => {
        if (!options || typeof options.fragmentShader !== "string") {
          throw new TypeError("createStage: options.fragmentShader must be a string");
        }
        return { ...options, enabled: true, uniforms: { ...(options.uniforms ?? {}) } };
      },
      requestFrame: (cb: (t: number) => void) => { queue.push({ id: nextId, cb }); return nextId++; },
      cancelFrame: (id: number) => {
        const i = queue.findIndex((f) => f.id === id);
        if (i >= 0) queue.splice(i, 1);
      },
      now: () => now,
    },
    tick(ms: number) {
      now += ms;
      const pending = queue.splice(0, queue.length);
      for (const f of pending) f.cb(now);
    },
  };
}

describe("mountVisualEffects", () => {
  test("rejects a viewer without postProcessStages.add (constructor contract)", () => {
    expect(() => mountVisualEffects({ scene: {} } as any)).toThrow(TypeError);
  });

  test("init creates one stage per style plus sharpen, all style stages disabled", () => {
    const viewer = fakeViewer();
    const frames = manualFrames();
    const handle = mountVisualEffects(viewer as any, frames.deps);
    for (const name of ["retro", "surveillance", "thermal", "anime", "noir", "snow"]) {
      const [stage] = viewer.scene.postProcessStages.byName(`godsEyeView_${name}`);
      expect(stage, name).toBeTruthy();
      expect(stage.enabled).toBe(false);
    }
    expect(viewer.scene.postProcessStages.byName("godsEyeView_sharpen")).toHaveLength(1);
    handle.destroy();
  });

  test("setStyle crossfades: previous fades out, next fades in over 500ms", () => {
    const viewer = fakeViewer();
    const frames = manualFrames();
    const handle = mountVisualEffects(viewer as any, frames.deps);
    handle.setStyle("thermal");
    frames.tick(600); // > TRANSITION_DURATION_MS
    let [thermal] = viewer.scene.postProcessStages.byName("godsEyeView_thermal");
    expect(thermal.uniforms.intensity).toBe(1.0);
    expect(thermal.enabled).toBe(true);
    handle.setStyle("retro");
    frames.tick(600);
    [thermal] = viewer.scene.postProcessStages.byName("godsEyeView_thermal");
    const [retro] = viewer.scene.postProcessStages.byName("godsEyeView_retro");
    expect(thermal.uniforms.intensity).toBe(0.0);
    expect(thermal.enabled).toBe(false);
    expect(retro.uniforms.intensity).toBe(1.0);
    expect(handle.getStyle()).toBe("retro");
    handle.destroy();
  });

  test("setStyle('normal') fades everything out", () => {
    const viewer = fakeViewer();
    const frames = manualFrames();
    const handle = mountVisualEffects(viewer as any, frames.deps, "noir");
    handle.setStyle("normal");
    frames.tick(600);
    const [noir] = viewer.scene.postProcessStages.byName("godsEyeView_noir");
    expect(noir.uniforms.intensity).toBe(0.0);
    expect(handle.getStyle()).toBe("normal");
    handle.destroy();
  });

  test("initialStyle applies instantly (no transition needed)", () => {
    const viewer = fakeViewer();
    const frames = manualFrames();
    const handle = mountVisualEffects(viewer as any, frames.deps, "surveillance");
    const [nvg] = viewer.scene.postProcessStages.byName("godsEyeView_surveillance");
    expect(nvg.uniforms.intensity).toBe(1.0);
    expect(nvg.enabled).toBe(true);
    expect(handle.getStyle()).toBe("surveillance");
    handle.destroy();
  });

  test("style preset defaults drive bloom/sharpen; anime/noir/snow use local fallbacks", () => {
    const viewer = fakeViewer();
    const frames = manualFrames();
    const handle = mountVisualEffects(viewer as any, frames.deps);
    handle.setStyle("retro"); // STYLE_PRESET_DEFAULTS has retro
    frames.tick(600);
    const bloom = viewer.scene.postProcessStages.bloom;
    // retro preset default: bloom enabled — assert the borrowed bloom flipped on
    expect(bloom.enabled).toBe(true);
    handle.setStyle("anime"); // no upstream preset default → local fallback: bloom off
    frames.tick(600);
    expect(bloom.enabled).toBe(false);
    handle.destroy();
  });

  test("destroy removes all self-created stages and restores bloom snapshot; idempotent", () => {
    const viewer = fakeViewer();
    const frames = manualFrames();
    const originalContrast = viewer.scene.postProcessStages.bloom.uniforms.contrast;
    const handle = mountVisualEffects(viewer as any, frames.deps, "snow");
    expect(viewer.scene.postProcessStages.length).toBe(7);
    handle.destroy();
    expect(viewer.scene.postProcessStages.length).toBe(0);
    expect(viewer.scene.postProcessStages.bloom.uniforms.contrast).toBe(originalContrast);
    expect(() => handle.destroy()).not.toThrow();
  });
});
```

注：若 `retro` 预设的 bloom 默认值实为 `enabled:false`（executor 打开 visualPresets.js:69-119 确认 STYLE_PRESET_DEFAULTS.retro.bloom），把该断言换成真实默认值的对应断言——测试意图是"预设值被应用 + fallback 生效"，不是钉死某个具体布尔。

- [ ] **Step 2: 跑测试确认失败**

Run: `cd /Volumes/TBU/Workspace/IntelHub-gev-p6/console && npx vitest run src/gev-visual/__tests__/visual-effects.test.ts`
Expected: FAIL（`../visual-effects` 不存在）

- [ ] **Step 3: 实现适配器**

`console/src/gev-visual/visual-effects.ts`：

```ts
// P6 visual presets adapter — the ONLY seam between the vendored GEV
// post-process render core (VisualEffects + visualPresets) and the IntelHub
// React HUD. Vendor modules are imported read-only; all DOM/event wiring
// lives on the React side.
//
// Deliberately NOT used: VisualSettings (DOM/shell-coupled), renderGovernor
// (not installed in our boot — hold/release are noops), detection/scopeMask
// (removed from scope, see spec §0).
import { VisualEffects } from "gev-engine/src/ui/visualEffects.js";
import { STYLES, STYLE_PRESET_DEFAULTS } from "gev-engine/src/ui/visualPresets.js";

export const GLOBE_STYLES = [
  "normal", "retro", "surveillance", "thermal", "anime", "noir", "snow",
] as const;
export type GlobeStyle = (typeof GLOBE_STYLES)[number];

export function isGlobeStyle(value: unknown): value is GlobeStyle {
  return typeof value === "string" && (GLOBE_STYLES as readonly string[]).includes(value);
}

// Upstream STYLE_PRESET_DEFAULTS only covers retro/surveillance/thermal.
// Local fallbacks for anime/noir/snow mirror GLOBAL_POST_DEFAULTS
// (sharpen on at 49%, bloom off) so switching never leaves stale bloom.
const LOCAL_PRESET_FALLBACKS: Record<
  string,
  { bloom: { enabled: boolean; intensity: number }; sharpen: { enabled: boolean; intensity: number } }
> = {
  anime: { bloom: { enabled: false, intensity: 0 }, sharpen: { enabled: true, intensity: 49 } },
  noir: { bloom: { enabled: false, intensity: 0 }, sharpen: { enabled: true, intensity: 49 } },
  snow: { bloom: { enabled: false, intensity: 0 }, sharpen: { enabled: true, intensity: 49 } },
  normal: { bloom: { enabled: false, intensity: 0 }, sharpen: { enabled: true, intensity: 49 } },
};

export interface ViewerLike {
  scene: {
    requestRender?: () => void;
    postProcessStages: {
      add(stage: unknown): void;
      remove(stage: unknown): void;
      bloom?: unknown;
    };
  };
}

export interface VisualEffectsDeps {
  createStage?: (options: unknown) => unknown;
  requestFrame?: (cb: (t: number) => void) => number;
  cancelFrame?: (id: number) => void;
  now?: () => number;
}

export interface VisualEffectsHandle {
  setStyle(style: GlobeStyle): void;
  getStyle(): GlobeStyle;
  destroy(): void;
}

export function mountVisualEffects(
  viewer: ViewerLike,
  deps: VisualEffectsDeps = {},
  initialStyle: GlobeStyle = "normal",
): VisualEffectsHandle {
  if (
    !viewer?.scene?.postProcessStages ||
    typeof viewer.scene.postProcessStages.add !== "function" ||
    typeof viewer.scene.postProcessStages.remove !== "function"
  ) {
    throw new TypeError(
      "mountVisualEffects: viewer.scene.postProcessStages.{add,remove} missing",
    );
  }

  const effects = new VisualEffects({
    viewer,
    requestRender: () => viewer.scene.requestRender?.(),
    holdRender: () => {},
    releaseRender: () => {},
    ...(deps.createStage ? { createStage: deps.createStage } : {}),
    ...(deps.requestFrame ? { requestFrame: deps.requestFrame } : {}),
    ...(deps.cancelFrame ? { cancelFrame: deps.cancelFrame } : {}),
    ...(deps.now ? { now: deps.now } : {}),
  });
  effects.initStyles();
  effects.initPostProcess();

  let current: GlobeStyle = "normal";

  const stageOf = (name: string) => (effects as any).stages?.[name];

  function applyPresetDefaults(style: GlobeStyle): void {
    const preset =
      (STYLE_PRESET_DEFAULTS as Record<string, any>)[style] ?? LOCAL_PRESET_FALLBACKS[style];
    if (!preset) return;
    effects.setBloomEnabled(Boolean(preset.bloom?.enabled));
    if (preset.bloom?.enabled) effects.applyBloomIntensity(Number(preset.bloom.intensity ?? 0));
    effects.setSharpenEnabled(Boolean(preset.sharpen?.enabled));
    if (preset.sharpen?.enabled) {
      effects.applySharpenIntensity(Number(preset.sharpen.intensity ?? 49) / 100);
    }
  }

  function setStyle(next: GlobeStyle): void {
    if (next === current) return;
    const prev = current;
    current = next;
    if (prev !== "normal") {
      effects.startTransition(prev, stageOf(prev)?.uniforms?.intensity ?? 1.0, 0.0);
    }
    if (next !== "normal") {
      effects.startTransition(next, stageOf(next)?.uniforms?.intensity ?? 0.0, 1.0);
    }
    applyPresetDefaults(next);
  }

  if (initialStyle !== "normal" && (STYLES as Record<string, unknown>)[initialStyle]) {
    const stage = stageOf(initialStyle);
    if (stage) effects.setStageIntensity(stage, 1.0);
    current = initialStyle;
    applyPresetDefaults(initialStyle);
  }

  let destroyed = false;
  return {
    setStyle,
    getStyle: () => current,
    destroy() {
      if (destroyed) return;
      destroyed = true;
      effects.destroy();
    },
  };
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cd /Volumes/TBU/Workspace/IntelHub-gev-p6/console && npx vitest run src/gev-visual/__tests__/visual-effects.test.ts`
Expected: PASS（7 个 test 全绿）

- [ ] **Step 5: Commit**

```bash
cd /Volumes/TBU/Workspace/IntelHub-gev-p6 && git add console/src/gev-visual/ && git commit -m "feat(gev-visual): mountVisualEffects adapter over vendored VisualEffects render core"
```

---

### Task 3: HudStyleSwitcher 组件 + 持久化

**Files:**
- Create: `console/src/globe-hud/HudStyleSwitcher.tsx`
- Modify: `console/src/globe-hud/hud.css`（追加下拉样式，照 .hud-bar token 体系）
- Test: `console/src/globe-hud/__tests__/hud-style-switcher.test.tsx`

**Interfaces:**
- Consumes: Task 2 的 `VisualEffectsHandle` / `GlobeStyle` / `GLOBE_STYLES` / `isGlobeStyle`。
- Produces（Task 4 依赖）：
  - `HudStyleSwitcher({ handle }: { handle: VisualEffectsHandle | null })`——handle 为 null 时不渲染（引擎未就绪）
  - `readPersistedStyle(): GlobeStyle`——读 `intelhub.globe.style`，非法值回退 "normal"
  - `STYLE_LABELS: Record<GlobeStyle, string>`——NORMAL/CRT/NVG/FLIR/ANIME/NOIR/SNOW

- [ ] **Step 1: 写失败测试**

`console/src/globe-hud/__tests__/hud-style-switcher.test.tsx`：

```tsx
import "@testing-library/jest-dom/vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { HudStyleSwitcher, readPersistedStyle } from "../HudStyleSwitcher";
import type { VisualEffectsHandle } from "../../gev-visual/visual-effects";

function fakeHandle(): VisualEffectsHandle & { setStyle: ReturnType<typeof vi.fn> } {
  let style: any = "normal";
  return {
    setStyle: vi.fn((s: any) => { style = s; }),
    getStyle: () => style,
    destroy: vi.fn(),
  };
}

beforeEach(() => localStorage.clear());
afterEach(() => cleanup());

describe("HudStyleSwitcher", () => {
  test("renders nothing while handle is null (engine not started)", () => {
    const { container } = render(<HudStyleSwitcher handle={null} />);
    expect(container).toBeEmptyDOMElement();
  });

  test("shows current style label and lists all seven options on click", () => {
    render(<HudStyleSwitcher handle={fakeHandle()} />);
    const button = screen.getByTestId("hud-style-switcher");
    expect(button).toHaveTextContent("NORMAL");
    fireEvent.click(button);
    for (const [id, label] of [
      ["normal", "NORMAL"], ["retro", "CRT"], ["surveillance", "NVG"],
      ["thermal", "FLIR"], ["anime", "ANIME"], ["noir", "NOIR"], ["snow", "SNOW"],
    ]) {
      expect(screen.getByTestId(`hud-style-option-${id}`)).toHaveTextContent(label);
    }
  });

  test("selecting a style calls handle.setStyle and persists to localStorage", () => {
    const handle = fakeHandle();
    render(<HudStyleSwitcher handle={handle} />);
    fireEvent.click(screen.getByTestId("hud-style-switcher"));
    fireEvent.click(screen.getByTestId("hud-style-option-thermal"));
    expect(handle.setStyle).toHaveBeenCalledWith("thermal");
    expect(localStorage.getItem("intelhub.globe.style")).toBe("thermal");
    expect(screen.getByTestId("hud-style-switcher")).toHaveTextContent("FLIR");
  });

  test("readPersistedStyle falls back to normal on missing/invalid values", () => {
    expect(readPersistedStyle()).toBe("normal");
    localStorage.setItem("intelhub.globe.style", "not-a-style");
    expect(readPersistedStyle()).toBe("normal");
    localStorage.setItem("intelhub.globe.style", "noir");
    expect(readPersistedStyle()).toBe("noir");
  });

  test("menu closes after selection", () => {
    render(<HudStyleSwitcher handle={fakeHandle()} />);
    fireEvent.click(screen.getByTestId("hud-style-switcher"));
    fireEvent.click(screen.getByTestId("hud-style-option-snow"));
    expect(screen.queryByTestId("hud-style-option-snow")).not.toBeInTheDocument();
  });
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cd /Volumes/TBU/Workspace/IntelHub-gev-p6/console && npx vitest run src/globe-hud/__tests__/hud-style-switcher.test.tsx`
Expected: FAIL（`../HudStyleSwitcher` 不存在）

- [ ] **Step 3: 实现组件**

`console/src/globe-hud/HudStyleSwitcher.tsx`：

```tsx
// P6 style switcher — React shell over the gev-visual adapter. Labels come
// from the vendor preset table (NORMAL/CRT/NVG/FLIR/ANIME/NOIR/SNOW);
// selection persists to localStorage so a page reload restores the filter
// (GlobeV2 applies it as initialStyle on mount).
import { useEffect, useRef, useState } from "react";
import {
  GLOBE_STYLES,
  isGlobeStyle,
  type GlobeStyle,
  type VisualEffectsHandle,
} from "../gev-visual/visual-effects";

const STORAGE_KEY = "intelhub.globe.style";

export const STYLE_LABELS: Record<GlobeStyle, string> = {
  normal: "NORMAL",
  retro: "CRT",
  surveillance: "NVG",
  thermal: "FLIR",
  anime: "ANIME",
  noir: "NOIR",
  snow: "SNOW",
};

export function readPersistedStyle(): GlobeStyle {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    return isGlobeStyle(raw) ? raw : "normal";
  } catch {
    return "normal";
  }
}

export function HudStyleSwitcher({ handle }: { handle: VisualEffectsHandle | null }) {
  const [open, setOpen] = useState(false);
  const [style, setStyle] = useState<GlobeStyle>(() => readPersistedStyle());
  const rootRef = useRef<HTMLDivElement | null>(null);

  // Close the menu on any click outside the switcher.
  useEffect(() => {
    if (!open) return;
    const onPointerDown = (event: PointerEvent) => {
      if (rootRef.current && !rootRef.current.contains(event.target as Node)) {
        setOpen(false);
      }
    };
    document.addEventListener("pointerdown", onPointerDown);
    return () => document.removeEventListener("pointerdown", onPointerDown);
  }, [open]);

  if (!handle) return null;

  const select = (next: GlobeStyle) => {
    handle.setStyle(next);
    setStyle(next);
    setOpen(false);
    try {
      localStorage.setItem(STORAGE_KEY, next);
    } catch {
      /* private mode: persistence is best-effort */
    }
  };

  return (
    <div className="hud-style" ref={rootRef}>
      <button
        type="button"
        className={`hud-bar-back hud-style-toggle${style !== "normal" ? " hot" : ""}`}
        onClick={() => setOpen((v) => !v)}
        title="视觉滤镜 / Visual style"
        aria-label="Visual style"
        aria-expanded={open}
        data-testid="hud-style-switcher"
      >
        ◐ {STYLE_LABELS[style]}
      </button>
      {open ? (
        <div className="hud-style-menu" role="menu">
          {GLOBE_STYLES.map((s) => (
            <button
              key={s}
              type="button"
              role="menuitemradio"
              aria-checked={s === style}
              className={`hud-style-option${s === style ? " active" : ""}`}
              onClick={() => select(s)}
              data-testid={`hud-style-option-${s}`}
            >
              {STYLE_LABELS[s]}
            </button>
          ))}
        </div>
      ) : null}
    </div>
  );
}
```

`hud.css` 追加（照现有 `.hud-bar-*` token 体系）：

```css
/* P6 style switcher dropdown — tokens follow .hud-bar (glass, 34px row). */
.hud-style { position: relative; display: inline-flex; }
.hud-style-toggle.hot { color: #7fd0ff; }
.hud-style-menu {
  position: absolute;
  top: calc(100% + 6px);
  right: 0;
  display: flex;
  flex-direction: column;
  min-width: 112px;
  padding: 4px;
  border-radius: 8px;
  background: rgba(3, 5, 9, 0.85);
  border: 1px solid rgba(127, 208, 255, 0.25);
  backdrop-filter: blur(8px);
  z-index: 30;
}
.hud-style-option {
  all: unset;
  padding: 6px 10px;
  font: inherit;
  font-size: 12px;
  letter-spacing: 0.08em;
  color: rgba(220, 232, 244, 0.85);
  cursor: pointer;
  border-radius: 5px;
}
.hud-style-option:hover,
.hud-style-option.active {
  background: rgba(127, 208, 255, 0.16);
  color: #7fd0ff;
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cd /Volumes/TBU/Workspace/IntelHub-gev-p6/console && npx vitest run src/globe-hud/__tests__/hud-style-switcher.test.tsx`
Expected: PASS（5 个 test 全绿）

- [ ] **Step 5: Commit**

```bash
cd /Volumes/TBU/Workspace/IntelHub-gev-p6 && git add console/src/globe-hud/HudStyleSwitcher.tsx console/src/globe-hud/hud.css console/src/globe-hud/__tests__/hud-style-switcher.test.tsx && git commit -m "feat(hud): style switcher dropdown with localStorage persistence"
```

---

### Task 4: GlobeV2 接线（生命周期安全）

**Files:**
- Modify: `console/src/pages/GlobeV2.tsx`（boot effect 内创建/销毁 adapter；state 传给 HudTopBar）
- Modify: `console/src/globe-hud/HudTopBar.tsx`（新增可选 prop `visualEffects`，渲染 `<HudStyleSwitcher>`）
- Test: `console/src/globe-hud/__tests__/hud-frame.test.tsx`（现有，确认不需要改——prop 可选即可）

**Interfaces:**
- Consumes: Task 2 `mountVisualEffects` / `VisualEffectsHandle` / `ViewerLike`；Task 3 `HudStyleSwitcher` / `readPersistedStyle`。
- Produces: HudTopBar 新可选 prop `visualEffects?: VisualEffectsHandle | null`（可选，现有调用方与测试零改动）。

- [ ] **Step 1: HudTopBar 加可选 prop**

HudTopBar.tsx 的 props 接口加 `visualEffects?: VisualEffectsHandle | null`（import type 自 `../gev-visual/visual-effects`），在告警铃铛按钮之前渲染 `<HudStyleSwitcher handle={visualEffects ?? null} />`。

- [ ] **Step 2: 跑现有 HUD 测试确认无回归**

Run: `cd /Volumes/TBU/Workspace/IntelHub-gev-p6/console && npx vitest run src/globe-hud/__tests__`
Expected: PASS 全绿（prop 可选，旧测试不受影响；hud-style-switcher 5 件也绿）

- [ ] **Step 3: GlobeV2 接线**

GlobeV2.tsx 关键改动（在 `booted` guard 的 boot effect 内，紧跟现有 `setSceneHandles(...)` 之后创建 adapter，cleanup 里在 `globe.destroy()` **之前**销毁）：

```tsx
import { mountVisualEffects, type VisualEffectsHandle } from "../gev-visual/visual-effects";
import { readPersistedStyle } from "../globe-hud/HudStyleSwitcher";

// state:
const [visualEffects, setVisualEffects] = useState<VisualEffectsHandle | null>(null);

// boot effect 内，globe.start().then(...) 里 setSceneHandles(...) 之后追加：
//   const fx = mountVisualEffects(components?.scene?.viewer as any, {}, readPersistedStyle());
//   visualEffectsRef.current = fx;
//   setVisualEffects(fx);
// （visualEffectsRef 是普通 useRef，cleanup 用；adapter 创建必须在 start() resolve 之后——
//  viewer 半建状态接 PostProcessStage 是 P2 Leaflet 教训的同类失败）

// cleanup 里，globe.destroy() 之前：
//   visualEffectsRef.current?.destroy();
//   visualEffectsRef.current = null;
//   setVisualEffects(null);
```

JSX：`<HudTopBar ... visualEffects={visualEffects} />`。

注意：adapter 销毁必须在 `globe.destroy()` 之前——`VisualEffects.destroy()` 要从活着的 `postProcessStages` 移除 stage 并恢复 bloom 快照；viewer 先毁则 remove 抛错。React 同组件多 effect 的 cleanup 按声明顺序执行，所以 adapter 的创建/销毁放进**同一个 boot effect**（用 ref 持有），不要拆成独立 effect。

- [ ] **Step 4: 类型检查 + 全量 console 测试**

Run: `cd /Volumes/TBU/Workspace/IntelHub-gev-p6/console && npx tsc -b && npx vitest run`
Expected: tsc 无 error；vitest 全绿

- [ ] **Step 5: Commit**

```bash
cd /Volumes/TBU/Workspace/IntelHub-gev-p6 && git add console/src/pages/GlobeV2.tsx console/src/globe-hud/HudTopBar.tsx && git commit -m "feat(globe): wire visual-effects adapter into GlobeV2 lifecycle + HudTopBar"
```

---

### Task 5: probe-gev.mjs 滤镜切换断言 + 315 验收

**Files:**
- Modify: `console/probe-gev.mjs`（rail-toggle 冒烟段之后追加滤镜段）

**Interfaces:**
- Consumes: Task 3 的 `data-testid="hud-style-switcher"` / `hud-style-option-<id>`；现有 `gate()` / `failures` / `warnings` 收集器。
- Produces: probe 退出码覆盖滤镜面——切换失败/页面崩溃都会 non-zero。

- [ ] **Step 1: 追加 probe 段**

在 rail flyout 冒烟段（约 :310-390）之后追加：

```js
// ── P6: style switcher — GLSL post-process actually re-renders ──
const styleBtn = await gate('[data-testid="hud-style-switcher"]', DATA_TIMEOUT_MS, "style switcher");
if (styleBtn) {
  const before = await page.screenshot();
  await page.click('[data-testid="hud-style-switcher"]');
  await page.click('[data-testid="hud-style-option-thermal"]');
  await page.waitForTimeout(700); // > TRANSITION_DURATION_MS (500) so the crossfade converges
  const label = (await page.textContent('[data-testid="hud-style-switcher"]')) ?? "";
  const after = await page.screenshot();
  if (!/FLIR/.test(label)) failures.push(`style switcher did not apply FLIR (label="${label.trim()}")`);
  if (Buffer.compare(before, after) === 0) {
    failures.push("screenshot identical after style switch — post-process stage never ticked");
  }
  // restore normal so subsequent probes see the default frame
  await page.click('[data-testid="hud-style-switcher"]');
  await page.click('[data-testid="hud-style-option-normal"]');
  await page.waitForTimeout(700);
}
```

注意：截图对比用 `Buffer.compare` 字节级不等判定（VisualEffects 动画期间每帧 `requestRender`，FLIR shader 必然改变像素）。若实测中 Cesium 截图含非确定噪声导致 normal 恢复段 flaky，把恢复段后的断言降格为 `warnings.push`——但 FLIR 切换的字节差异断言必须保持 failure 级。

- [ ] **Step 2: rsync → 315 构建 → 重启 → probe**

```bash
cd /Volumes/TBU/Workspace/IntelHub-gev-p6 && rsync -az --delete \
  --exclude '.git/' --exclude 'backups/' --exclude '.DS_Store' \
  --exclude 'compose/.env' --exclude 'compose/.env.crucix' --exclude 'docs/' --exclude 'build/' \
  --exclude 'config/searxng/' --exclude 'hub-core/target/' \
  --exclude 'console/node_modules/' --exclude 'console/dist/' \
  --exclude 'core/' --exclude 'data/' \
  ./ Debian-test:/home/zou/IntelHub/

ssh -o BatchMode=yes Debian-test 'cd /home/zou/IntelHub \
  && bash scripts/build-console.sh 2>&1 | tail -1 \
  && sudo systemctl restart hub-core && sleep 4 && systemctl is-active hub-core'
```

（本期纯 console 改动，`build-hub.sh` 无变化可跳过；若 executor 发现 hub-core 也被触碰则照 AGENTS.md 完整序列跑。）

- [ ] **Step 3: 315 上跑 probe**

```bash
KEY=$(ssh -o BatchMode=yes Debian-test 'grep "api_key:" /home/zou/IntelHub/core/agent-keys.txt | head -1 | grep -o "ihk_[a-f0-9]*"')
node console/probe-gev.mjs http://10.10.10.35:8800 "$KEY"
```

Expected: exit 0，滤镜段无 failure。

- [ ] **Step 4: 315 全量验收**

```bash
KEY=$(ssh -o BatchMode=yes Debian-test 'grep "api_key:" /home/zou/IntelHub/core/agent-keys.txt | head -1 | grep -o "ihk_[a-f0-9]*"')
for a in sp8 sp6 sp7 sp3; do
  echo "── $a: $(INTELHUB_SSH=Debian-test python3 scripts/accept-$a.py "$KEY" http://10.10.10.35:8800 2>&1 | grep -E '==.*(passed|failed)' | tail -1)"
done
```

Expected: 与基线一致全绿（sp8 42+2sh/0f、sp6 37+5sh/0f、sp7 16+11sh/0f、sp3 19/0f）。

- [ ] **Step 5: Commit**

```bash
cd /Volumes/TBU/Workspace/IntelHub-gev-p6 && git add console/probe-gev.mjs && git commit -m "test(probe): style-switch screenshot assertion (FLIR crossfade re-render)"
```

---

### Task 6: 熵减 + ledger + 合并部署

**Files:**
- Create: `docs/superpowers/execution/2026-09-18-gev-p6-ledger.md`
- Modify: `AGENTS.md`（验收基线行若 probe 检查位变化则同步）

- [ ] **Step 1: 熵减检查**

```bash
cd /Volumes/TBU/Workspace/IntelHub-gev-p6 && grep -rn "VisualSettings\|scopeMask\|detection" console/src/ --include="*.ts" --include="*.tsx" | grep -v __tests__ | grep -v "setScopeMaskEnabled(false)"
```

Expected: 空（P6 不引入任何 detection/VisualSettings 引用；application.ts 里既有的 `setScopeMaskEnabled(false)` 防御行保留）。同时确认无新增死代码：未被 import 的导出、注释掉的临时代码块全部删除。

- [ ] **Step 2: 写 P6 ledger**

`docs/superpowers/execution/2026-09-18-gev-p6-ledger.md`：范围（6 滤镜+bloom/sharpen 联动+crossfade+切换器+持久化+契约守卫+probe 断言）、决策记录（detection keyhole 移除、renderGovernor 不装、anime/noir/snow 本地 fallback）、315/410 验收结果、已知边界（screenshot 字节对比的 flaky 处置）。

- [ ] **Step 3: 合并 main + 清理 worktree**

```bash
cd /Volumes/TBU/Workspace/IntelHub-gev-p6 && git add -A && git commit -m "docs(ledger): GEV P6 视觉预设终态"
cd /Volumes/TBU/Workspace/IntelHub && git merge --no-ff feat/gev-p6-visual-presets && git worktree remove ../IntelHub-gev-p6 && git branch -d feat/gev-p6-visual-presets
```

- [ ] **Step 4: 410 生产部署 + 验收**

```bash
cd /Volumes/TBU/Workspace/IntelHub && rsync -az --delete \
  --exclude '.git/' --exclude 'backups/' --exclude '.DS_Store' \
  --exclude 'compose/.env' --exclude 'compose/.env.crucix' --exclude 'docs/' --exclude 'build/' \
  --exclude 'config/searxng/' --exclude 'hub-core/target/' \
  --exclude 'console/node_modules/' --exclude 'console/dist/' \
  --exclude 'core/' --exclude 'data/' \
  ./ IntelHub:/home/zou/IntelHub/

ssh -o BatchMode=yes IntelHub 'cd /home/zou/IntelHub \
  && bash scripts/build-console.sh 2>&1 | tail -1 \
  && sudo systemctl restart hub-core && sleep 4 && systemctl is-active hub-core'

KEY=$(ssh -o BatchMode=yes IntelHub 'grep "api_key:" /home/zou/IntelHub/core/agent-keys.txt | head -1 | grep -o "ihk_[a-f0-9]*"')
node console/probe-gev.mjs http://10.10.10.41:8800 "$KEY"
for a in sp8 sp6 sp7 sp3; do
  echo "── $a: $(python3 scripts/accept-$a.py "$KEY" 2>&1 | grep -E '==.*(passed|failed)' | tail -1)"
done
```

Expected: probe exit 0；sp8/sp6/sp7/sp3 与基线一致全绿。

- [ ] **Step 5: push**

```bash
cd /Volumes/TBU/Workspace/IntelHub && git push origin main
```
