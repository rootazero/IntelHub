# GEV 页面可视化移植总纲：引擎 UI/标注渲染核 → IntelHub Globe HUD

> 日期：2026-09-18 · 状态：已批准（用户确认方案 1 + 六节设计）
> 性质：**多阶段工程总纲**。P6-P10 每期各自走完整 spec→plan→SDD→315→410 循环；本文档是架构与决策的单一事实来源。
> 上游文档：`docs/superpowers/specs/2026-09-17-gev-engine-fusion-design.md`（数据层 P1-P5，本文档是其可视化维度的姊妹篇）
> 调研证据：两个 Explore agent 对 `/Volumes/TBU/Github/gods-eye-view`（上游 src/ui ~90 文件 + src/annotations 13 文件）与 `console/`（globe-hud / gev-boot / gev-adapters）的全量盘点，2026-09-18。

## 0. 目标与已决决策

将 GEV 引擎 `src/ui/` + `src/annotations/` 中**未接线的页面可视化能力**（视觉预设、相机姿态、标注绘制、驾驶舱、长尾工具）全量移植进 IntelHub Globe 页面。

| 决策 | 结果 |
|---|---|
| 技术路线 | **方案 1：渲染核直引 + React 壳**。vendor 保持纯净只读；GLSL shader / PostProcessStage 管线 / 相机数学 / 标注渲染器从 vendor 直接 import；DOM 事件接线与 shell 组合层全部丢弃，交互壳在 `console/src/globe-hud/` 用 React 重写 |
| 弃选方案 | 方案 2（引擎 UI 整体直挂，违背路线 A）、方案 3（全量 TS/React 重写，工程量翻倍且失去月度同步红利） |
| 相机约束 | **严禁引入上游圆形视野遮罩/周边压黑**。相机姿态与驾驶舱只移植姿态数学与跟随逻辑，保留当前开放全景渲染；scopeMask 维持 `setScopeMaskEnabled(false)` |
| 工程组织 | 总纲 + 分期（P6-P10），每期独立 spec→plan→SDD→315→410→push |
| 标注存储 | **超越上游**：上游标注仅存会话内存，IntelHub 持久化入 PG，标注跨会话存活 |
| 适配层位置 | 新增 `console/src/gev-visual/`——本工程唯一新代码密集区 |

## 1. 现状盘点（Gap Analysis 结论）

**已移植（P1-P3）**：20+ 图层中的 10 层数据 + 引擎基座 + 自研四边 HUD 壳（图层轨/详情面板/健康条/顶条/底条）。

**完全未接线**：引擎 `src/ui/`（~90 文件）与 `src/annotations/`（13 文件）在 `console/src/` **整体零引用**——console 仅 import 9 个引擎模块（application/constructCatalog/scene/contextStore/layerState/lifecycle/installations·traffic source/scopeMask），全部是数据层。GlobeV2 无任何 postProcess stage、visual preset、相机跟随代码。

### 移植候选池（规模均含渲染核，不含上游测试）

| 候选 | 内容 | 规模 | 内部依赖 | 难度 |
|---|---|---|---|---|
| A. 视觉预设 | 6 套 GLSL 后处理（retro CRT / surveillance NVG / thermal FLIR / anime / noir / snow）+ bloom/sharpen + detection 聚光遮罩，500ms 强度 crossfade | ~3.3k 行 | bloom.js、detectionPolicy、renderGovernor | 低-中 |
| B. 标注绘制 | pin/line/area 手绘（白板）、世界锚定标注（脉冲环/弯箭头/callout 卡）、area drape 贴地渲染 | ~6k 行 | renderGovernor、inputOwnership、resolver（Overpass/NaturalEarth） | 中 |
| C. 驾驶舱 | 航班第一人称追踪：罗盘/高度/速度尺仪表 + 区域简报轮播 + 视觉模式切换（optical/crt/nvg/flir） | ~2.9k 行 | trackedCamera、cockpitMath、cockpitVisionPolicy、contextModePolicy | 高 |
| D. 相机姿态 | 追踪目标斜视 35°/垂直俯视切换 | 379 行 | viewer.trackedEntity | 低 |
| E. 长尾 | 可拖拽面板系统（位置记忆）、场景分享链接、录制安全框、快捷键、帧率监视、地点搜索 | 各 0.4-3.6k 行 | panel→shellFacade（需重写组合层）、scene→director | 中-高 |

## 2. 总体架构：渲染核直引 + React 壳

```
React HUD（console/src/globe-hud/ 新增模块）
  滤镜切换器 / 绘制工具栏 / 座舱进出按钮 / 场景分享对话框 / 搜索框实装…
    │ 调用（React state ↔ adapter 窄契约）
适配层（console/src/gev-visual/ 新增）★ 唯一新代码密集区
  每功能一个 adapter：封装渲染核创建/参数更新/销毁；
  抹平 vanilla JS 事件 → React state；统一在 GlobeV2 生命周期内挂载/卸载
    │ import（只读，契约守卫锁定）
渲染核（vendor console/gev-engine/src/ui/ + src/annotations/）
  visualPresets + styles/ GLSL、visualEffects PostProcessStage 管线、
  cockpitMath、cameraOrientation 姿态数学、annotation world/screen 渲染器
    ✗ 不 import：DOM 事件接线、applicationShell、shellFacade、standalone 模板
```

**关键机制**

- **适配层窄契约**：每个 adapter 单文件单职责，暴露 `mount(viewer, opts) → { update(state), destroy() }`。引擎 DOM 事件一律不接线，交互触发全部 React 侧重写。
- **生命周期纪律**：adapter 在 GlobeV2 的引擎 start 之后挂载、destroy 之前卸载（沿用 cesium-credits 清理教训）；React StrictMode 双挂载安全。
- **引擎纯净度**：vendor 目录零修改——新增的只是 import 关系。月度同步 = 整体替换 + 契约守卫验证。
- **i18n**：HUD 新控件全量 zh/en；引擎内部 toast/readout 英文可接受。

## 3. 分期路线

| 期 | 内容 | 渲染核来源 | 出口标准 |
|---|---|---|---|
| **P6 视觉预设** | 6 套 GLSL 滤镜 + bloom/sharpen + detection 聚光遮罩 + 500ms crossfade；HUD 滤镜切换器（建议放右下或顶条）；滤镜选择持久化 localStorage | visualPresets.js、visualEffects.js、styles/*.glsl、visualSettings.js（detection 部分） | 6 滤镜切换无闪屏无报错；detection 遮罩跟随图层开关；probe 截图断言滤镜生效 |
| **P7 相机姿态+导航** | 追踪目标斜视 35°/俯视切换（**全景渲染，无圆圈遮罩**）；地点搜索飞行实装（接通 HudTopBar 占位搜索框，geocode 走 hub 代理或免 key 服务） | cameraOrientationControls.js（姿态数学）、location/locationSearch 核 | 选中航班一键跟随/退出，姿态两档切换；搜索框实装可飞行定位；占位 disabled 搜索框代码删除（熵减） |
| **P8 标注绘制** | pin/line/area 手绘工具（HUD 左轨工具栏）+ 世界锚定标注（脉冲环/弯箭头/callout 卡）+ area drape 贴地；**标注持久化入 PG**（新表 + REST `/api/v1/annotations/*`），跨会话存活 | annotationEngine.js（状态机）、drawTool/drawMode、world/screen/hybridAnnotationRenderer；resolver 按需裁剪（Overpass 代理解析可后置） | 标注刷新后仍在；绘制工具栏进 HUD；sp8 新增标注 CRUD 检查位 |
| **P9 驾驶舱** | 航班第一人称追踪：罗盘/高度/速度尺仪表 + 视觉模式切换（复用 P6 滤镜管线）+ 区域简报轮播 | cockpitCamera/cockpitMath/cockpitInstruments/cockpitBriefing/cockpitVisionPolicy；简报数据源需先在 P8 尾补实 request-services（weather/summary 代理） | 选中航班进座舱，仪表数据实时刷新；退出完整恢复 HUD；全程全景渲染无遮罩 |
| **P10 长尾** | 可拖拽面板系统（位置/折叠态 localStorage）、场景分享链接（URL 恢复视觉状态）、录制安全框（16:9/9:16）、快捷键、帧率监视 | panel 系统核、scene/sceneSharing 核（director 依赖重写）、recordingControls、applicationShortcuts、frameRateMonitor | 场景链接可分享恢复；面板拖拽位置刷新后保持；快捷键表文档化 |

P6 是命门：首次验证"vendor 渲染核直引 + React 壳"模式与契约守卫扩展，为后续四期立模板。

## 4. 契约守卫扩展

`console/src/gev-boot/__tests__/source-contracts.test.ts` 增加 **import-surface 断言**：

- 适配层 import 的每个引擎模块路径存在性检查
- 被引模块的导出签名快照（导出名列表 + 关键函数 arity）
- `styles/` GLSL 目录文件清单哈希（shader 与 Cesium 版本耦合，上游改动需显式确认）

上游月度同步时守卫红 → 先补适配层再放行，与数据层契约守卫同一保险丝机制。

## 5. 测试与验收

- **适配层单测**：每 adapter 配 vitest（fake Cesium viewer）。沿用 P3 教训——mock 必须复刻真实构造器契约（构造器内断言参数形状，如 `if (!el?.addEventListener) throw`），禁止 lenient mock。
- **probe 扩展**：`console/probe-gev.mjs` 每期加视觉断言——P6 滤镜切换前后截图对比、P9 座舱仪表 DOM 存在性、P8 标注实体计数。
- **sp8 验收**：每期追加检查位，遵循三态（passed/shelved/failed），退码只看 failed。
- **熵减**：每期落地后同步清理被取代的旧代码（P7 实装搜索后删除占位 disabled 搜索框；被 adapter 取代的临时接线删除）。
- **验收序列**：每期走完整 315 验收全绿 → merge main → 410 部署 → 生产验收 → push。

## 6. 风险登记

1. **引擎 UI 模块隐性依赖 standalone shell**（shellFacade 单例组合、application.js 全局状态）→ 每期开工先由 subagent 做"依赖切割验证"：确认目标渲染核可剥离、列出其隐性依赖清单，再动手写 adapter。切割失败则该模块降级为"参考实现重写"（该期 spec 中显式记录）。
2. **P9 座舱简报依赖 request-services 占位**（weather/summary 现返 null）→ 在 P8 尾排期补实对应 hub 代理，P9 开工前验收。
3. **GLSL shader 与 Cesium 版本耦合** → 契约守卫覆盖 styles/ 目录哈希；Cesium 升级走独立窗口。
4. **上游持续 churn**（396 commits/2 周）→ 维持月度同步窗口；可视化开发 pin 当前版本，不同步期中新功能。
5. **标注持久化是上游没有的表结构** → P8 spec 需设计迁移 + REST 面 + 与 Signals/图谱的潜在联动（标注可否转 Signal 留待 P8 spec 决策）。
6. **面板系统/场景分享的 shellFacade、director 依赖** → P10 适配层重写组合层，预估为 P10 主要工作量；若切割验证发现耦合过深，允许 P10 拆分（面板/场景分享各自独立成期）。
