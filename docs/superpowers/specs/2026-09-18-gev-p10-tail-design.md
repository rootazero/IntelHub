# GEV P10 长尾移植 设计

> 日期：2026-09-18 · 状态：草稿（待用户批准 + 拆分决策）
> 性质：分期 P10，隶属于 `docs/superpowers/specs/2026-09-18-gev-visual-port-design.md` 总纲的"长尾"分期
> 上游文档：`2026-09-18-gev-visual-port-design.md` §3 P10 + §6 risk #6
> 同期姊妹：P6 `30a709b` · P7 `da166a1` · P8 `b29255a` · P9 `4a93022` 全部上线

## 0. 目标与边界

### 目标
将 GEV 引擎 `console/gev-engine/src/ui/` + `scenes/` 中"长尾工具"移植进 IntelHub Globe HUD：
1. **面板系统**：可拖拽位置 + 折叠态 + 刷新后保持（localStorage）
2. **场景分享链接**：URL hash 编码/恢复视觉状态（位置/风格/追踪/选择）
3. **录制安全框**：16:9 / 9:16 / 4:3 三档切换 + 录制模式 body class + safe-frame overlay
4. **快捷键**：setStyle / toggleHud / toggleOrbit / cycleDetection 等 8 项
5. **帧率监视**：postRender FPS 实时读数
6. **P9 deferred fold-in 4 项**：NOAA unitCode 守门、vision 重入修复、BriefingHandle dead API、lib.rs 字母序

### 显式不做的（与总纲一致）
- **不进** panel 系统与 Signals/Neo4j 联动（vendor 没有，规格内禁）
- **不进** 服务组合层（`services/application.js` 6 slot 模式）—— P9 已用 per-endpoint stub 模式，P10 不重写
- **不进** cycleDetection（vendor 独有功能，与 IntelHub 无关）
- **不进** GEV 语音控制（`#gev-voice-control`）—— 仅 vendor demo
- **不实例化** `PanelChrome` / `SceneControls` / `RecordingControls` 顶层 composition 类——这些是 vendor 全局组合层，与 cockpitCoordinator 同性质（React 重写状态机）
- **不改** vendor localStorage 命名空间（首次 P10 沿用 `godsEyeView.v6.*` 前缀，避免数据形 churn；P11+ 再整理）

### 设计原则
- 纯函数模块（panelMeasurement / panelRailGeometry / scenePolicy / frameRateMonitor）→ 直接 import
- DOM-only 模块（panelDisclosure / scenePresentation / sceneSharing / applicationShortcuts）→ adapter 包壳
- Composition 模块（panelChrome / sceneControls / recordingControls）→ 不复用，重写 React 状态机
- shareRestoration.js vendor 内部依赖多 → **D1 决策点**：丢 vs 写引入用用用自己 React hook
- P9 deferred 4 项作为 P10 子任务（T0 之前的修复列）

## 1. 现状盘点

### vendor P10 候选池（20 个 .js 文件 + 2 个 CSS = 22 文件）

**Tier A（直引 / 微包壳）— 12 文件 ~12k LOC**：
- `panelMeasurement.js`（1736）— 纯函数 `measurePanelNaturalHeight`
- `panelRailGeometry.js`（2175）— 纯函数（已 P9 source-contracts 钉扎）
- `panelRails.js`（452）— re-export 桶形
- `panelDisclosure.js`（9906）— DOM-only `bindPanelDisclosure` / `collapsePanelOnEscape` / `createHoverDisclosure`
- `scenePolicy.js`（7347）— 纯函数（已 P9 source-contracts 钉扎 contextLayerEnableBlockReason）
- `scenePresentation.js`（5299）— DOM 渲染 helpers
- `scenes.js`（52）— re-export 桶形
- `sceneSharing.js`（4006）— DOM 工厂 `createSceneDialog` / `mountSceneSharing`
- `applicationShortcuts.js`（1572）— DOM+8 action 回调
- `frameRateMonitor.js`（2201）— Cesium `postRender` FPS 读出
- `sceneControls.js`（7655，Tier B 偏低）— scene 控制器，scope-cut 已确认无 shellFacade
- `shareRestoration.js`（9709，Tier C 偏中）— vendor 内部 import 多（详见 §1.4 决策）

**Tier B（adapter 包装）— 4 文件 ~80k LOC**：
- `panelChrome.js`（18999）— composition root（不实例化）
- `panelPositionControls.js`（10274）— 拖拽 + localStorage
- `panelLayoutController.js`（17766）— 轨测量 + 布局调度
- `leftPanelRail.js`（11935）/ `rightPanelRail.js`（9709）— 左右轨测量放置
- `recordingControls.js`（3202）— recording mode + safe-frame

**Tier C（composition root 重写）— 1 文件**：
- `panelChrome.js`（同 Tier B，但额外需要 adapter wrapper，React 状态机接管）

### 切割验证（总纲 §6 risk #1 / #6 已 rerun）

**关键发现：原担心的 scope-cut 风险很大程度上是误判。**
- `shellFacade` —— **P10 模块零引用**（仅 `applicationShell.js` 用）
- `director.js` —— **P10 模块零运行时引用**（仅 test files）
- `application.js` —— **P10 模块零引用**（仅 applicationShell）

**真实残余风险**（5 项）：
1. **LocalStorage 命名空间**：vendor 用 `godsEyeView.v6.panelPos.*` / `godsEyeView.v6.layoutRightPanels` —— P10 沿用 vendor 前缀（P11+ 整理）
2. **DOM-id 契约**：recordingControls + panelRails 写死 `#safe-frame-overlay`、`#title-bar`、`#style-indicator`、`#scene-panel`、`#hud-toggle`、`#hud-layout-select`、`#cockpit-hud .cockpit-topline` 等 —— adapter 必须确认这些 ID 在 IntelHub DOM 存在或重命名
3. **HUD 契约**：recordingControls 要求 `this.hud.{getMode, getVariant, setMode, setVariant, visible}` —— adapter 包壳 HUD 提供
4. **shareRestoration.js vendor 内部 import**：`LayerStateCoordinator` / `stampInitialShareGesture` / `canPresentDeferredStatusNotice` —— D1 决策点
5. **CSS 引用**：`recording.css`（38 行）+ `scenes.css`（456 行）选择器多指向 vendor 自有 DOM

### IntelHub shell 现状

- `console/src/gev-visual/` 现有 9 个 adapter（camera-orientation / follow-controller / location-search / visual-effects / annotations / 4 cockpit）—— 0 P10 adapter
- `console/src/globe-hud/` 已含 HudFrame + 4 子轨（HudLayerRail/HudTopBar/HudDetailPanel/HudLayerRail）+ HudCockpitFrame + 4 子组件
- **panel 拖拽/位置记忆 0 实现**；**快捷键 0 实现**（之前 keyboard 只测过 HudFrame 接管）
- AGENTS.md 已知坑无 panel/scene/recording 特定条目（P10 是首期导入）

### P9 已知边界待 P10 处理（4 项）
- **D1 NOAA unitCode 守门**（final review important non-blocking）—— hub `gev_weather.rs` parser 固定转换（km/h→kts、Pa→hPa）无 unitCode 守门；NOAA 若改单位静默错值。P10 T0 修复。
- **D2 vision 重入缺陷**（final review new finding）—— cockpit-store seed `readPersistedVisionMode()` 但 `vision.setMode()` 仅由显式 click 触发；重入/重载后 switch 高亮 active 但 globe 实际未切。P10 T0 修复。
- **D3 BriefingHandle.start/stop/pages dead API**（T3 minor）—— 仅测试引用，HUD 自持 setInterval。P10 T0 删除。
- **D4 lib.rs secrets 模块字母序**（T2 minor）—— pre-existing 排序问题。P10 T0 整理。

### sp9 baseline（重要）
- AGENTS.md 提到 `sp9 14` checks —— 当前未跑过 baseline
- P10 不新增 sp9 检查（除非 sp9 有 panel/scene 现有检查）
- P10 sp8 +?/probe P10 段必须有

## 2. 总体架构

```
React HUD（console/src/globe-hud/ 新增）
  HudPanelDragHandle           每个 panel 顶角 → 鼠标抓
  HudScenePanel                左下场景分享（vendor 改写）
  HudRecordingControls         顶条下拉录制 + safe-frame 切换
  HudShortcutCheatsheet        "?" 键弹快捷键表
  HudFrameRateReadout          帧率监视右上角
    │
适配层 ★ console/src/gev-visual/tail/（新目录）
  panel-drag.ts               包壳 panelPositionControls + lib.ts 持久化
  panel-layout.ts             包壳 left/rightPanelRail（重命名 intelhub obstacle selectors）
  panel-disclosure.ts         包壳 panelDisclosure
  scene-controls.ts           包壳 sceneControls
  scene-sharing.ts            包壳 sceneSharing
  scene-restore.ts            D1 决策：drop vendor module → 写 useShareRestoration React hook
  recording-controls.ts       包壳 recordingControls + HUD 契约
  shortcuts.ts                包壳 applicationShortcuts + 8 action 映射
  frame-rate-monitor.ts       包壳 frameRateMonitor（极简，几乎直引）
  index.ts                    re-export
  ↓ import（只读，契约守卫锁定）
vendor 渲染核
  panelMeasurement, panelRailGeometry, scenePolicy, frameRateMonitor  ← 纯函数
  panelDisclosure, scenePresentation, sceneSharing  ← DOM-only helpers
  sceneControls, panelPositionControls, panelLayoutController, leftPanelRail, rightPanelRail, recordingControls
  applicationShortcuts
  × 不 import: panelChrome, applicationShell, director, shellFacade
  ↓
hub-core（本期仅 D1 NOAA unitCode 守门改动）
  gev_weather.rs parser 加 unitCode 守门 + Debian-test 活探
```

## 3. 数据契约

### 3.1 面板拖拽持久化（**沿用 vendor 命名空间**）
- `localStorage.godsEyeView.v6.panelPos.<panelId>` = `{x, y, w, h}` JSON
- `localStorage.godsEyeView.v6.layoutRightPanels` = 折叠态 JSON
- **不替换前缀**（避免数据 churn）；P11+ 大整理
- 持久化 key 在 IntelHub 全局 React context 提供（不 import vendor 单例）

### 3.2 场景分享链接（**D1 决策后定**）
- **方案 A**（推荐）：写 `useShareRestoration` React hook，URL hash 编码 `{style, north, tilt, tracking, selected, panelLayout}`；进入页面读 hash + 还原视觉状态
- **方案 B**：保留 vendor `shareRestoration.js` + 写 `vendor-compat-shim.ts` 提供 `LayerStateCoordinator` / `stampInitialShareGesture` / `canPresentDeferredStatusNotice` 最小 stub

### 3.3 录制安全框契约
- `body.recording-mode` class toggle（vendor `recording.css` 选择器依赖）
- `#safe-frame-overlay.active` / `#safe-frame-box` DOM 元素由 HUD 创建
- 16:9 / 9:16 / 4:3 三档：HUD 提供 HUD contract adapter `{getMode, getVariant, setMode, setVariant, visible}`
- sp8 +1 检查：`body.recording-mode` class 存在性

### 3.4 快捷键契约
- **8 个 action 映射**：
  - `setStyle(name)` → P6 `mountVisualEffects.setStyle(name)`（注：与 P9 R5 守门兼容——cockpit active 时跳过；同 P9 gate）
  - `dismissSearch` → HudTopBar 清空搜索
  - `toggleHud` → HudFrame `data-visible` 切换
  - `toggleOrbit` → 暂 stub（P10 不进完整 orbit 控制器，仅 toggle UI 标志）
  - `toggleCleanView` → stub（与 P10 scope 不符，P11+）
  - `toggleLayers` → HudLayerRail 展开/折叠
  - `cycleDetection` → vendor 独有（P10 不映射，按下 no-op + console.warn）
  - `toggleCctv` → HudDetailPanel tab=cctv 切换
- **快捷键面板**：`?` 键弹 cheatsheet（cheatsheet 是新组件）

### 3.5 帧率监视契约
- 实时读 `viewer.scene.postRender` delta
- DOM 写入 `#title-bar` 或新 `#frame-rate-readout`
- P10 决定：放右上角 + 4Hz 节流

## 4. 关键决策

### D1 — shareRestoration.js 取舍
**两个方案选一个**：
- **方案 A**（推荐）：**丢弃 vendor module**，写 `useShareRestoration` React hook。URL hash 编码纯 intelhub state（视觉风格/相机/追踪/选择/panel 位置）。轻量、可测、与现有 GlobeV2 URL restoration 路径统一。
- **方案 B**：保留 vendor module + 写 vendor-compat-shim.ts 提供 `LayerStateCoordinator` / `stampInitialShareGesture` / `canPresentDeferredStatusNotice` 三 stub。语义保留但增加 adapter 复杂度（3 个 vendor 内部依赖要复刻）。

**spec 倾向方案 A**。理由：(1) vendor module 内部依赖复杂；(2) URL restoration 在 intelhub 已部分实装（GlobeV2 read hash）；(3) 与 cockpit vision 状态统一管理（同一 React store）。

### D2 — 拆分决策（待用户）
P10 spec 范围较大。建议拆为：
- **P10a 快速胜利**（Tier A 全部）：~12k LOC 12 文件，预计 4-5 task（T1 守卫 + T2 frameRate+shortcuts+recording + T3 scene+panel disclosure + T4 HUD + T5 probe+sp8 + T6 合并）
- **P10b 面板+场景控制**（Tier B + C）：~80k LOC 6 文件，预计 6-7 task
- **P10c 分享恢复**（D1 决策落点）：~1-2 task

**建议：P10 一次性做完整版**（不分期），因为：
- 任务间依赖不强（panel-position + scene-controls 可并行）
- 用户决策窗口短（一次问 D1）
- 月度同步风险（vendor 一次性同步完整覆盖）

### D3 — AGENTS.md 更新策略
P10 一次性补：
- sp6/sp8 baseline 更新（P10 加检查后）
- 已知坑补 P10 记录（如录制 ID 冲突、localStorage 命名空间策略）

### D4 — vision 模式重入修复 + P10 deferred 4 项
作为 P10 spec §5 子任务，**T0 修复列**（T1 之前完成）：
- D1 NOAA unitCode 守门（hub gev_weather.rs 改）
- D2 vision 重入修复（cokpit-store.ts + HudCockpitVisionSwitch.tsx）
- D3 BriefingHandle.start/stop/pages dead API 删除
- D4 lib.rs secrets 字母序整理

## 5. 测试与验收

### 5.1 适配层单测（target ≥ 30 新测试）
- `panel-drag.test.ts`：mock panel DOM，断言拖拽 → localStorage 写入 + reload 恢复
- `panel-layout.test.ts`：mock panel stack，断言 obstacle selector 命中
- `scene-controls.test.ts`：mock actions/subscribe，断言 capture/update/next 8 actions
- `scene-sharing.test.ts`：mock createSceneDialog，断言分享链接生成
- `use-share-restoration.test.tsx`：D1 方案 A 路径
- `recording-controls.test.ts`：mock HUD contract，断言 body class + safe-frame toggle
- `shortcuts.test.ts`：mock actions，断言 8 action 映射（cycleDetection no-op + warn）
- `frame-rate-monitor.test.ts`：mock viewer.postRender，断言 FPS 写入 DOM

### 5.2 probe P10 段
- `p10-frame-rate-readout=1`
- `p10-shortcut-cheatsheet=1`（按 `?`）
- `p10-scene-panel=1`
- `p10-recording-mode=1`
- `p10-panel-drag=<panel_id>=1`（拖拽→localStorage 写入+reload 恢复）

### 5.3 sp8 检查位
- +3 检查：recording body class / scene panel exists / panel localStorage round-trip
- baseline: 315 sp8 45+2sh → **48+2sh/0f**（315 clean baseline）

### 5.4 验收序列
按 AGENTS.md：315 全绿 → merge main → 410 build-hub+build-console → 410 验收 → push

### 5.5 熵减目标
- `grep -rn "TODO.*panel\|TODO.*scene\|placeholder.*panel\|P5\|P10" console/src/` 应为空（vendor + adapter comments 例外）
- `grep -rn "panelChrome\|shellFacade\|director" console/src/` 仅适配层 comment 允许

## 6. 风险登记

1. **scope-cut 风险降级**：原担心的 shellFacade/director 切割实际为误判（panel/scene 模块零引用）—— 风险降为 medium；shareRestoration 仍为唯一 Tier C（vendor 内部 import）。
2. **DOM-id 契约**：recording.css + scenes.css 选择器多指向 vendor DOM（`#gev-voice-control` 等）—— adapter 必须保留 vendor ID 或 wholesale import vendor CSS。已决定**wholesale import vendor CSS** + adapter 创建缺失 ID。
3. **localStorage 命名空间 churn**：vendor 前缀 `godsEyeView.v6.*` 沿用——已部署的 IntelHub 用户无历史数据所以无迁移问题；首次 P10 仅 intelhub 内部使用，P11+ 整理。
4. **快捷键冲突**：HudFrame 已用 `←`/`→` 接管简报轮播（P9）；P10 shortcuts 必须避开。spec §3.4 锁定 8 个 action + cheatsheet 弹出键 `?`。
5. **panel 拖拽性能**：4Hz RAF 节流（与 P9 仪表同模式）；持久化用 debounce 500ms。
6. **录制 safe-frame 与 hud css 兼容**：vendor recording.css 用 `body.recording-mode` selector；IntelHub hud.css 必须允许 body class override——已验 vendor 选择器具体性更高，无冲突。
7. **P9 deferred D1 NOAA unitCode 活探**：T0 必须从 Debian-test 实际调用 `forecastGridData` 取 response，确认默认 `unitCode: 'wmo Unit:km_h-1'` 等；不活探则守门退化为 unitCode 仅记录（不全量应用）。

## 7. 任务拆分建议（spec 草稿，预 T0 决策后定稿）

**T0** P9 deferred 4 项修复列（commit 单独，不进 P10 PR）
- D1 NOAA unitCode
- D2 vision 重入
- D3 dead API 清理
- D4 lib.rs 字母序

**T1** source-contracts + localStorage prefix 守卫（新增 8 模块）
- panelDisclosure / panelMeasurement / panelRails / panelRailGeometry / sceneControls / sceneSharing / scenePresentation / shareRestoration / scenes / recordingControls / applicationShortcuts / frameRateMonitor / scenePolicy / services/application

**T2** console adapter 9 个 + ≥30 测试
- panel-drag / panel-layout / panel-disclosure / scene-controls / scene-sharing / use-share-restoration (D1 决策) / recording-controls / shortcuts / frame-rate-monitor

**T3** HUD 5 子组件 + GlobeV2 接线 + 熵减
- HudPanelDragHandle / HudScenePanel / HudRecordingControls / HudShortcutCheatsheet / HudFrameRateReadout
- GlobeV2 wiring：clean-up order 插 HUD contract + frame rate monitor

**T4** sp8 +3 + probe P10 段 + 315 验收

**T5** ledger + whole-branch final review + merge + 410 + push

## 8. P10 已知边界（不阻塞）

- `services/application.js` 6 slot 服务组合模式（P9 已 per-endpoint stub）—— P11+ 决定是否重构
- `cycleDetection` action（vendor 独有）—— no-op
- GEV 语音控制（`#gev-voice-control`）—— 不移植
- `toggleCleanView` / `toggleOrbit` —— stub（与 P10 scope 不符）
- panel drag resize viewport-clamp 边缘行为（与 mobile viewport 适配）—— P11+ mobile 时再处理
- 录制 safe-frame 与 system-audio capture（仅 overlay + body class，无 audio）—— 后续如需录 audio 走 P11+
- 简报主动推送（SSE/WebSocket）—— 持续 deferred
- 确定性 addClickWorld test seam（P8 deferred item g）—— 持续 deferred
- scene sharing 跨用户分享（vendor 仅有 share-link 机制，无 server-side share DB）—— 后续如需服务端持久化走 P11+