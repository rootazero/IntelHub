# GEV P9 驾驶舱与区域简报 设计

> 日期：2026-09-18 · 状态：草稿（待用户批准）
> 性质：分期 P9，隶属于 `docs/superpowers/specs/2026-09-18-gev-visual-port-design.md` 总纲的"驾驶舱"分期
> 上游文档：`2026-09-18-gev-visual-port-design.md` §3 P9 + 风险 §5/§6
> 同期姊妹：P6 视觉预设（main `30a709b`）、P7 相机姿态+搜索（main `da166a1`）、P8 标注绘制+持久化（main `b29255a`）已上线

## 0. 目标与边界

### 目标
将 GEV 引擎 `console/gev-engine/src/ui/cockpit*`（15 JS + 1 CSS，共 5531 LOC）的**第一人称飞行追踪**移植进 IntelHub Globe：
1. **仪表**：罗盘 / 高度 / 速度尺 + 航向带
2. **视觉模式**：复用 P6 滤镜管线，5 档（optical/crt/nvg/thermal/noir）切换
3. **区域简报轮播**：weather + summary 实时面板，按距离 + 时间刷新
4. **确定性绘制回调**（P8 已知边界 item g）：addClickWorld 流经 Cesium test seam

### 显式不做的（与总纲一致）
- **不进** cockpitSignal 联动 Signal/图谱（与 P8 一致；annotation_links 同样留空）
- **不进** TR-3B 切换（vendor 独有隐藏彩蛋，无场景需求）
- **不进** recordCockpitSession / flight history（属 P10 长尾）
- **不进** cloud/precipitation vendor 渲染（要数据源+独立阶段，超出 P9）
- **不实例化** `CockpitViewController` / `CockpitCoordinator` / `cockpitTrackingController` —— 内部 `cockpitEntryAllowed`/`dispatchCockpitModeChanged` 强耦合到 vendor `contextModePolicy` 状态机
- **不重做** P7 follow-controller / camera-orientation / P6 visual-effects —— P9 是消费者

### 设计原则
- 渲染核直引：纯函数模块（cockpitMath、cockpitPresentation、cockpitVisionPolicy）→ 直接 import；DOM 渲染模块（cockpitInstruments/cockpitLayout）→ adapter 包壳；状态机模块（cockpitCoordinator/cockpitController/cockpitTrackingController）→ 不复用，重写 React 状态
- 仪表数据来自 `dataManager.layers`（已有 flights layer + velocity/altitude/heading 字段），无需新 backend 数据契约
- 简报数据：weather/summary 新建 hub 代理（总纲 §6 风险 2 已预排）
- 视觉模式：复用 P6 `mountVisualEffects` handle，新增 `onVisionChange` listener 桥接 `cockpitVisionPolicy.applyCockpitVisionStageIntensities`
- 跟踪所有者仍是 P7 `follow-controller`（cockpit 退出后 follow 保持）

## 1. 现状盘点

### vendor cockpit 模块（5531 LOC = 3343 JS + 2188 CSS）
- **纯函数可直引**：`cockpitMath`（heading/altitude/bearing）、`cockpitPresentation`（常量+格式化）、`cockpitVisionPolicy`（5 档 + intensity gating）、`cockpitCamera`（Cesium-only update 函数）
- **DOM 渲染需包壳**：`cockpitInstruments`（method-based，updateHud/updateRoute/setVisionMode/cycleVisionMode）、`cockpitLayout`（DOM scaffolding）、`cockpitBriefing`（pages + auto-rotate）、`cockpitSignals`（列表渲染）、`cockpitContext`（HUD readout）
- **状态机禁用**：`cockpitCoordinator`（imports `cockpitEntryAllowed` from `contextModePolicy` + `shellFacade` events）、`cockpitTrackingController`（_adoptTrackedEntity 直接持有 viewer + shell）、`cockpitController`（top-level orchestrator with shellFacade binding）
- **CSS**：`styles/cockpit.css`（2188 行）—— 直接复制到 `console/src/hud.css`，适配器只调整 className 前缀（`cockpit-` → `intelhub-cockpit-`）

### 切割验证（总纲 §6 风险 1 触发）
cockpitCoordinator.js 显式依赖 `contextModePolicy.js` 提供的 `cockpitEntryAllowed`：
- 进入条件 = trackedEntity 存在 + 不是 cockpit mode + 不是 context mode
- contextModePolicy 是**纯函数模块**（437 LOC，零 Cesium 依赖）→ 安全切割
- 但 `_adoptTrackedEntity` 与 `_setTrackedEntity`（cockpitTrackingController:107-184）直接持有 `viewer.trackedEntity` + 写 `camera.lookAtTransform` → **必须重写为 P7 follow-controller + camera-orientation 的消费者**

### IntelHub shell 现状
- `console/src/gev-visual/` 已有 5 个 adapter：camera-orientation / follow-controller / location-search / visual-effects / annotations（**无任何 cockpit adapter**）
- `console/src/gev-boot/__tests__/source-contracts.test.ts` 守卫 22 模块/函数（P8 末态）→ P9 增 4 模块
- `console/src/globe-hud/` 已实装：HudFrame/HudLayerRail/HudTopBar/HudDetailPanel/HudFollowButton/HudSearchLocation/HudDrawToolbar/HudAnnotationList
- `console/src/hud.css` 已含 hud-* 类（前缀统一）

### hub request-services 现状（重要）
**当前 0 个** weather / summary 端点：
- `gev_earthquakes / gev_celestrak / gev_vessels / gev_traffic / gev_installations / gev_cctv` 全是空间/物理/天气无关源
- weather: 无代理、无 key 配置、无数据契约
- summary: 完全空白
- **P9 必须新建 `gev_weather.rs` + `gev_summary.rs`**（或合并单文件）

### P8 已知边界须 P9 处理
- **(g) 确定性 addClickWorld 流** —— P8 验收时无法 headless 验证手绘点击（pickPosition 在 headless Cesium 不稳定）；P9 仪表坐标采用相同 Cesium API（viewer.scene.pickPosition），复用 P8 probe 的 `probeCoordinateFromScreen` test seam（待 T1 验证 vendor `pickWorldFromScreen` 是否在 HUD 也可走相同路径）

## 2. 总体架构

```
React HUD（console/src/globe-hud/ 新增）
  HudCockpitToggleButton    HudLayerRail 第 9 个图标「🎮」
  HudCockpitFrame           全屏 overlay（HudFrame 风格）
    HudCockpitInstruments     左下：罗盘 + 高度尺 + 速度尺 + 航向带（纯 CSS+SVG）
    HudCockpitBriefingPanel   右上：简报轮播（weather + summary 双 tab）
    HudCockpitContext         左上：callsign/ICAO/altitude/heading/speed readout
    HudCockpitVisionSwitch    中下：5 档视觉模式切换器
  ↓
适配层 ★ console/src/gev-visual/cockpit/（新目录）
  instruments-mount.ts      包壳 cockpitInstruments（纯 Cesium 数据 → React props）
  briefing-mount.ts         包壳 cockpitBriefing（pages rotation + auto-refresh）
  vision-mount.ts           桥接 cockpitVisionPolicy + mountVisualEffects
  cockpit-store.ts          React state ↔ adapter 同步（mode/active/refreshInterval）
  index.ts                  re-export 3 handles
  ↓ import（只读，契约守卫锁定）
vendor 渲染核
  cockpitMath, cockpitPresentation, cockpitVisionPolicy, cockpitCamera
  cockpitInstruments, cockpitBriefing (DOM helpers only)
  × 不 import: cockpitController, cockpitCoordinator, cockpitTrackingController,
    cockpitContext, cockpitLayout, cockpitSignals, cockpitDisplayPortal
  ↓
hub-core（本期新建 2 模块）
  /api/v1/gev/weather?lat&lon&units    NOAA + Open-Meteo 双源（fallback）
  /api/v1/gev/summary?entity_id        简报源聚合（acled + reliefweb + gdelt 三源 + cache）
  crates/hub-core/src/gev_weather.rs
  crates/hub-core/src/gev_summary.rs
```

## 3. 数据契约

### 3.1 仪表数据流（**直接走 dataManager.layers，无新 backend**）
- **飞行实体读取**：用 P7 `follow-controller.trackedId()` 拿到 flight kind+id，向 flights layer 查询当前帧
- **属性提取**：vendor cockpitMath 已经定义：
  ```js
  heading(velocity) → degrees(0-360)  // 已是纯函数
  altitudeMeters(position) → number
  speedMps(velocity) → number
  ```
- **订阅**：flights layer 已有 `onTick(callback)`；HUD 用 RAF 轮询（250ms 4Hz 节流，避免过载）

### 3.2 视觉模式契约（**扩展 P6 而非新建**）
- `mountVisualEffects` handle **增 1 方法**：`getStages(): { stages: Map<string, PostProcessStage> } | null`
  - 用途：让 cockpitVisionPolicy 拿到 stage 列表做 intensity gating
  - 验证：`mountVisualEffects(viewer).getStages()?.stages` instanceof Map
- cockpitVisionPolicy 5 档定义不动；P9 仅消费 `TARGET_STYLE_BY_MODE` 映射
- 持久化：`localStorage.intelhub.cockpit.visionMode`（与 P6 的 `intelhub.visual.style` 同源思路）

### 3.3 简报契约（**新 backend**）
```ts
interface WeatherResponse {
  source: 'noaa' | 'open-meteo';
  fetched_at: string;       // RFC3339
  temperature_c: number | null;
  wind_speed_kts: number | null;
  wind_direction_deg: number | null;
  precipitation_mm: number | null;
  cloud_cover_pct: number | null;
  visibility_m: number | null;
  pressure_hpa: number | null;
}

interface SummaryResponse {
  entity_id: string;        // 'ICAO:ZBAA' 或 'flight:UAL123'
  generated_at: string;     // RFC3339
  sources: Array<'acled' | 'reliefweb' | 'gdelt' | 'cache'>;
  bullets: Array<{ text: string; source_url?: string; age_hours: number }>;
  next_refresh_after: string;
}
```
- **weather 优先 NOAA**（无 key），fallback Open-Meteo（无 key）；失败 503 + body `{error, sources_tried}`
- **summary 三源并行**（acled/reliefweb/gdelt 已有上游），任一源 OK 即返 bullets；TTL 15 分钟 Redis（复用 sp4 redis 健康格约定）
- **节流**：飞行追踪每 10 km 移动 OR 30 分钟 → 触发 refresh（vendor `COCKPIT_REGIONAL_REFRESH_DISTANCE_M` / `COCKPIT_REGIONAL_REFRESH_MS`）
- **空载防御**：entity_id 无对应源 → 返 200 + `bullets: []` + `sources: ['cache']`（不报错，避免 cockpit 抖动）

### 3.4 HUD 简报轮播
- 顶部双 tab：[Weather] [Summary]
- Weather 自动 rotate 4 metrics（temp/wind/precip/visibility），每 4 秒切换
- Summary 自动 rotate bullets，每 6 秒切换（vendor `COCKPIT_BRIEF_ROTATE_MS` 6000）
- 手动 ← → 切换：键盘 `←` / `→`（在 cockpit mode 下接管 HudFrame 全局快捷键）
- 暂停/继续：鼠标 hover 简报面板

## 4. 关键决策

### D1 — 不实例化 cockpitController
理由：`cockpitController` 顶层 orchestrator 持有 `shellFacade.onShowNotification/onPermissionChange/onShareLinkCopied` 等事件订阅，重写成本 > 收益。React HUD 自有 state 树够用。

### D2 — vision-mode 通过 P6 handle 桥接
P6 已有 `mountVisualEffects`，`setStyle()` 走 crossfade。cockpitVisionPolicy 的 `applyCockpitVisionStageIntensities` 直接写入 stage intensity —— 两套机制并存，但 P9 用 `setVisionMode(name)` 触发两步：(1) `setStyle(TARGET_STYLE_BY_MODE[mode])` 500ms crossfade，(2) intensity gating（applyCockpitVisionStageIntensities）。这样 P6 的 6 预设（'retro'/'surveillance'/'thermal'/'noir' 等）+ cockpitVisionPolicy 的 5 档共享同一套 stage。

### D3 — 简报 fallback 严格
简报源全部 degraded-by-design 时：简报面板显示「简报暂不可用 — 距上次成功 X 分钟」灰色 banner，**不**重试风暴。retry 走 hub `gev_summary.rs` 内部 5 分钟间隔。

### D4 — HUD cockpit 全屏 overlay 而非替换 globe
不用 `viewer.scene.mode = SCENE2D` 之类；保持 globe 可见（追踪实体只改 camera），HUD cockpit 是 full-screen 浮层。**严禁圆形视野遮罩**（总纲 §0 禁令）。

### D5 — 跟踪所有权不变
cockpit 退出后，P7 follow 保持。HUD button 文案：「退出驾驶舱（继续跟随）」。
进入 cockpit：自动 call `follow-controller.follow('flight', id)` if not already tracked。

### D6 — 确定性 addClickWorld test seam（P8 已知边界 g）
P8 验收的 Cesium pickPosition 不可靠问题——P9 cockpit **不依赖** click-to-coordinate 流（仪表全部从 layer 数据读）。**P9 仅消费 P8 已建的 `probeCoordinateFromScreen` test seam**（如有），如不存在则继续 deferred。
Plan 阶段会读 vendor `resolver.js:1774 pickWorldFromScreen` 复盘 P8 测试方案；如 seam 不可复用，本期不创建新的，留 P10。

## 5. 测试与验收

### 5.1 适配层单测
- `instruments-mount.test.ts`：mock dataManager（layer map），断言 updateHud 4Hz 节流
- `briefing-mount.test.ts`：mock fetch（weather/summary 双 endpoint），断言 rotation + manual pause
- `vision-mount.test.ts`：mock visual-effects handle，断言 5 档 setter 触发 setStyle + applyCockpitVisionStageIntensities
- `cockpit-store.test.ts`：纯 state reducer 测试，10 个 cases
- target: ≥15 新测试；mock 复刻真实构造器契约（P3 教训）

### 5.2 probe 扩展（P9 段）
`console/probe-gev.mjs` 追加：
- `p9-cockpit-button=1`
- `p9-cockpit-frame=1`（进 cockpit 后）
- `p9-cockpit-instruments=3`（罗盘/高度/速度尺 SVG path 数）
- `p9-cockpit-vision-modes=5`
- `p9-briefing-panel=1` + `p9-briefing-weather=200` + `p9-briefing-summary=200`（fetch probe 端点）

### 5.3 sp6/sp8 检查位扩展
- sp6: +2 检查（weather endpoint + summary endpoint，存在 + 200 + body shape）
- sp8: +2 检查（cockpit 进入 probe + instruments DOM 三件套存在）

baseline：sp6 37+5sh/0f → 39+5sh/0f；sp8 46+2sh/0f → 48+2sh/0f（按 P8 ledger 实际基线调整）

### 5.4 验收序列
按 AGENTS.md：315 全绿 → merge main → 410 build-hub.sh（hub 新增 2 端点）→ build-console.sh → restart hub-core → 410 probe+sp8+sp6 → push

### 5.5 熵减目标
- grep `P5|P9|TODO.*cockpit|TODO.*briefing|placeholder.*cockpit` 在 src/ 应为空
- cockpitController / cockpitCoordinator / cockpitTrackingController **不在 console import 链**（验证：`grep -r cockpitController console/src/` 仅适配器层允许）

## 6. 风险登记

1. **cockpitCoordinator shell 耦合（已确认可控）**：通过 contextModePolicy 纯函数切割 + 自写 React 状态机替代。验证：P9-T1 任务从 cockpitCoordinator 拉出 cockpitEntryAllowed 所需的最少参数 + trackedEntity（仅 2 个），证明可剥离。
2. **request-services 新建 weather/summary**：选 NOAA + Open-Meteo（均免 key），summary 复用 acled/reliefweb/gdelt 已有上游。失败源保持可见（D3 fallback banner）。
3. **P6 visual-effects handle 扩展 getStages**：契约变更需守卫（source-contracts 增 1 方法）。Phase P9-T1 与 P6 实施者对照现状确认最小变更（可能 P6 已暴露 stages）。
4. **视觉模式切换 + 滤镜预设并存** 状态混淆：D2 两步顺序（setStyle 500ms crossfade → intensity gating），单测断言不并发写同一 stage。
5. **HUD 全屏 overlay 性能**：4Hz 仪表更新走 RAF 节流 + React.memo；简报轮播 setInterval 在 hover 时 clearInterval。性能 baseline < 16ms/frame。
6. **vendor CSS 2188 行移植**：className 前缀替换 + 移除 vendor 字体声明（保留我们现有字体）；如有不依赖样式可裁剪的减负机会（例如 vendor hover effects），P9-T2 评估后裁。

## 7. P9 已知边界（不阻塞）

- 简报源聚合质量（bullets 去重 / 排序 / 翻译）仅做最简实现
- cockpit recording / history / TR-3B（vendor 独有）
- cockpitSignal ↔ Signals 联动
- 跨设备同步视觉模式（仅 localStorage，无 sync）
- 简报主动推送（依赖 SSE/WebSocket；本期仅 polling）
- 确定性 addClickWorld test seam（P8 已知边界 g；D6 已说明）