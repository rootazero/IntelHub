# GEV P9 驾驶舱与区域简报 实施计划

> 日期：2026-09-18 · 状态：草稿（待 P9-T1 验证协议校正）
> 上游 spec：`docs/superpowers/specs/2026-09-18-gev-p9-cockpit-briefing-design.md`
> 上游总纲：`docs/superpowers/specs/2026-09-18-gev-visual-port-design.md`
> 同期姊妹：P6 `30a709b` · P7 `da166a1` · P8 `b29255a`

## 0. 范围与边界

**In scope**：
- cockpit 仪表（罗盘/高度/速度尺/航向带）+ readout
- 视觉模式 5 档（optical/crt/nvg/thermal/noir）桥接 P6
- 区域简报双 tab（weather + summary）+ 轮播
- hub 新增 gev_weather / gev_summary 端点
- sp6/sp8 各 +2 检查 + probe P9 段
- 315 → 410 → push

**Out of scope**：cockpitSignal 联动、TR-3B、recordCockpitSession、cloud/precipitation vendor 渲染、确定 addClickWorld seam（D6 deferred to P10）

## 1. 总览

| Task | 内容 | 出口 | 文件 |
|---|---|---|---|
| **T1** | 契约守卫扩展 + P6 getStages 暴露 + cockpit 模块/函数钉扎 + baseline sp6/sp8 测量 | source-contracts 守卫绿（新增 22 模块/函数数 +1 method）；baseline 数报告 | `console/src/gev-boot/__tests__/source-contracts.test.ts`, `console/src/gev-visual/visual-effects.ts` |
| **T2** | hub gev_weather.rs + gev_summary.rs + axum route 注册 + 集成测试 | cargo test --package hub-core ≥ 12 新测试 | `hub-core/crates/hub-core/src/gev_weather.rs`, `gev_summary.rs`, `api.rs` |
| **T3** | console adapter 4 个（instruments/briefing/vision/cockpit-store）+ index + 15+ 单测 | vitest 全绿；mock 复刻真实构造器契约 | `console/src/gev-visual/cockpit/{instruments-mount,briefing-mount,vision-mount,cockpit-store,index}.ts` + `__tests__/` |
| **T4** | HUD HudCockpitFrame + 4 子组件（HudCockpitInstruments/HudCockpitBriefingPanel/HudCockpitContext/HudCockpitVisionSwitch）+ HudLayerRail 第 9 图标 + GlobeV2 wiring + 熵减 | console vitest 全绿（含 baseline 4 失败不变）；新 +9 测试；tsc rc=0；vendor 0 行 | `console/src/globe-hud/HudCockpit{Frame,Instruments,BriefingPanel,Context,VisionSwitch}.tsx` + `pages/GlobeV2.tsx` 改 + `hud.css` |
| **T5** | sp6 +2 + sp8 +2 + probe P9 段 + 315 验收 | sp6 39+5sh/0f · sp8 48+2sh/0f · probe exit 0 | `scripts/accept-sp6.py` · `scripts/accept-sp8.py` · `console/probe-gev.mjs` |
| **T6** | ledger + whole-branch final review + merge main + 410 build-hub+build-console+deploy+验收+push | main @ 新 commit；410 验收全绿；pushed to origin/main | `docs/superpowers/execution/2026-09-18-gev-p9-ledger.md` + `AGENTS.md` sp6/sp8 数更新 |

## 2. Worktree 隔离

```bash
cd /Volumes/TBU/Workspace/IntelHub
git worktree add ../IntelHub-gev-p9 -b feat/gev-p9-cockpit-briefing
sleep 4   # 网络盘同步
cd /Volumes/TBU/Workspace/IntelHub-gev-p9
git log -1 main   # 确认 BASE = b29255a (P8 ledger commit)
```

## 3. 任务详情

### Task 1 — 契约守卫扩展 + P6 getStages 暴露 + baseline measure

**Files**：
1. `console/src/gev-boot/__tests__/source-contracts.test.ts` — 增 4 模块
2. `console/src/gev-visual/visual-effects.ts` — 暴露 `getStages()`

**Steps**：
1. **baseline 测量（先做）**：在 315 上 `KEY=$(...)` 跑 `accept-sp6.py` + `accept-sp8.py` 各 2 次，记下 stable 数（P8 ledger 报 37+5sh / 46+2sh；如差异 > ±1 报告阻塞）
2. **读 vendor cockpit 模块/函数 export 列表**：
   - cockpitMath: heading/altitudeMeters/speedMps/cockpitCloudCover/cockpitCloudLightning（**T1 先验证 export 名**）
   - cockpitVisionPolicy: COCKPIT_VISION_MODES / TARGET_STYLE_BY_MODE / normalizeCockpitVisionMode / applyCockpitVisionStageIntensities / captureCockpitVisionBaseline
   - cockpitPresentation: COCKPIT_* 常量 / formatCockpitBriefAge / setCockpitRollingValue / COCKPIT_BRIEF_PAGES
   - cockpitCamera: update(viewer, entity, params)（**纯函数，不是 class**）
3. **source-contracts.test.ts** 增 4 模块钉扎 + `applyCockpitVisionStageIntensities` arity + `COCKPIT_BRIEF_PAGES` 长度（6 页）
4. **`mountVisualEffects` handle 增 `getStages()`**：
   - 读现有 `console/src/gev-visual/visual-effects.ts`（P6 产物），看是否已有 `getStages()` 或类似；如无则在 handle interface 增 `getStages(): Map<string, PostProcessStage> | null`，实现调 vendor `viewer.scene.postProcessStages`
   - 验证：`mountVisualEffects(viewer).getStages()` 返 Map 实例；mock test 验
5. **跑 source-contracts**：必须全绿（22+4=26 个 module + cockpitVisionPolicy 增 5 函数 arity 钉扎）

**Verify**：
- `cd console && npx vitest run src/gev-boot/__tests__/source-contracts.test.ts` — 全绿
- vendor 0 行改动
- 报告含 baseline 数（sp6/sp8）

### Task 2 — hub gev_weather.rs + gev_summary.rs

**Files**：
1. `hub-core/crates/hub-core/src/gev_weather.rs`（新）
2. `hub-core/crates/hub-core/src/gev_summary.rs`（新）
3. `hub-core/crates/hub-core/src/lib.rs` — 字母序 `pub mod gev_weather; pub mod gev_summary;`
4. `hub-core/crates/hub-core/src/api.rs` — route 注册 `GET /api/v1/gev/weather` + `GET /api/v1/gev/summary`
5. `hub-core/crates/hub-core/src/secrets.rs`（如不存在需新建）— env 读 NOAA_USER_AGENT（NOAA 公开 API 需 UA header，否则 403）
6. `hub-core/crates/hub-core/tests/gev_weather.rs` + `tests/gev_summary.rs`（集成测试）

**Step details**：
1. **gev_weather**：双源 fallback
   - NOAA: `https://api.weather.gov/points/{lat},{lon}` → `forecastGridData` URL → 取 5 properties（temperature/windSpeed/windDirection/precipitationProbability/visibility）。**NOAA 需 UA header**，配置 `HUB_NOAA_UA` env（fallback to "IntelHub/dev"）
   - Open-Meteo: `https://api.open-meteo.com/v1/forecast?latitude={lat}&longitude={lon}&current=temperature_2m,wind_speed_10m,wind_direction_10m,precipitation,cloud_cover,visibility,surface_pressure` — 无需 key
   - 5 分钟 in-memory cache（`tokio::sync::RwLock<HashMap<(lat,lon), Instant>>`）避免 hot-spot
   - 失败返 503 + `{error, sources_tried}` 字段
2. **gev_summary**：三源聚合
   - acled/reliefweb/gdelt 现有函数（hub 已有吗？查 `hub-core/crates/hub-core/src/gev_*.rs`）—— **T1 验证报告里明确"已有"vs"需新建"**
   - 如已有：gev_summary.rs 是聚合层，调 3 函数 + 去重 + Redis 缓存 15 min
   - 如无：gev_summary.rs 至少 stub 返 200 + 空 bullets（确保契约成立）
   - Redis TTL key: `cockpit:summary:<entity_id>`，用 sp4 redis 健康格协议
3. **api.rs route 注册**：看现有 `/api/v1/gev/earthquakes` 注册模式，照搬
4. **集成测试**：mock wiremock 或 reqwest mocking 测 NOAA + Open-Meteo 双源降级；summary 三源一源失败仍 200

**Verify**：
- `cargo test --package hub-core --test gev_weather` 全绿
- `cargo test --package hub-core --test gev_summary` 全绿
- `cargo build --release --workspace` rc=0
- 报告含真实集成测试数 + secrets 配置步骤

### Task 3 — console adapter 4 个

**Files**：
1. `console/src/gev-visual/cockpit/instruments-mount.ts`（新）— 包壳 cockpitInstruments
2. `console/src/gev-visual/cockpit/briefing-mount.ts`（新）— 包壳 cockpitBriefing
3. `console/src/gev-visual/cockpit/vision-mount.ts`（新）— 桥接 cockpitVisionPolicy + P6 handle
4. `console/src/gev-visual/cockpit/cockpit-store.ts`（新）— React state
5. `console/src/gev-visual/cockpit/index.ts`（新）— re-export
6. `console/src/gev-visual/cockpit/__tests__/instruments-mount.test.ts` 等 4 个新测试

**Adapter 签名**（T3 验证协议：实施者必须先打开 vendor 看真实构造器）：
- `InstrumentsHandle{ update(state: { heading, altitudeM, speedMps, callsign }): void; destroy(): void }`
  - 包壳 vendor `cockpitInstruments.updateHud()`/`.updateRoute()`/`.setCockpitRollingValue()`
  - 实际 T3 验证 vendor 是否真的有 `cockpitInstruments` class 或一组 functions
- `BriefingHandle{ start(weather: WeatherResponse, summary: SummaryResponse): void; stop(): void; next(): void; prev(): void; destroy(): void }`
  - 包壳 vendor `cockpitBriefing.showBriefPage()`/`.startBriefRotation()`/`.renderRegionalBrief()`
- `VisionMountHandle{ setMode(mode: 'optical'|'crt'|'nvg'|'thermal'|'noir'): void; getMode(): string; destroy(): void }`
  - 内部调 `mountVisualEffects.setStyle(TARGET_STYLE_BY_MODE[mode])` + `applyCockpitVisionStageIntensities(viewer.scene.postProcessStages, mode)`
- `CockpitStoreState{ active: boolean; trackedId: string|null; visionMode: string; briefingPaused: boolean }`

**Verify**：
- `npx vitest run src/gev-visual/cockpit/` 全绿（target ≥ 15 tests）
- mock 复刻真实构造器契约（P3 教训）
- vendor 0 行改动

### Task 4 — HUD HudCockpitFrame + 4 子组件 + GlobeV2 wiring + 熵减

**Files**：
1. `console/src/globe-hud/HudCockpitFrame.tsx`（新）— 全屏 overlay，HudFrame 子组件 slot
2. `console/src/globe-hud/HudCockpitInstruments.tsx`（新）— SVG 仪表 4 件套
3. `console/src/globe-hud/HudCockpitBriefingPanel.tsx`（新）— 双 tab + 轮播
4. `console/src/globe-hud/HudCockpitContext.tsx`（新）— 左上 readout
5. `console/src/globe-hud/HudCockpitVisionSwitch.tsx`（新）— 5 档切换器
6. `console/src/globe-hud/HudLayerRail.tsx`（改）— 第 9 个图标「🎮」cockpit toggle
7. `console/src/pages/GlobeV2.tsx`（改）— 挂载 cockpit adapters + HudCockpitFrame
8. `console/src/hud.css`（改）— `.hud-cockpit-*` 类
9. `console/src/globe-hud/__tests__/HudCockpit*.test.tsx` × 4（新）

**Step details**：
1. **HudCockpitFrame**：render 仅 when `cockpit-store.active === true`；用 `skipFirst = useRef(true)` effect 模式（**P8 教训：Leaflet/Cesium 视图未就绪禁动视图**）—— cockpit 不动 camera，但 RAF loop 启动需小心
2. **HudCockpitInstruments**：4Hz RAF 轮询 trackedEntity → 调 `InstrumentsHandle.update()`；SVG 路径直接内联（罗盘圆 + 高度尺纵向刻度 + 速度尺横向 + 航向带）
3. **HudCockpitBriefingPanel**：双 tab + auto-rotate（vendor `COCKPIT_BRIEF_ROTATE_MS` 6000）+ hover 暂停
4. **HudCockpitContext**：callsign/ICAO/altitude/heading/speed 文本，每 250ms 更新
5. **HudCockpitVisionSwitch**：5 档按钮（与 P6 HudFilterSwitch 风格统一），点击 → `VisionMountHandle.setMode()` + localStorage
6. **GlobeV2 挂载**：复用 P8 的"annotation FIRST in cleanup"模式，加 cockpit destroy；测试 seam 同 P8（lazy import for briefing-mount 因 vendor graph reaches vendor CSS）
7. **熵减 grep**：
   - `grep -rn "P5\|TODO.*cockpit\|placeholder.*cockpit" console/src/` 应为空（vendor CSS 例外）
   - `grep -rn "cockpitController\|cockpitCoordinator\|cockpitTrackingController" console/src/` 仅适配器层允许
8. **testids 逐字**：`hud-cockpit-button`、`hud-cockpit-frame`、`hud-cockpit-instruments`、`hud-cockpit-compass`、`hud-cockpit-altimeter`、`hud-cockpit-speed`、`hud-cockpit-briefing`、`hud-cockpit-tab-weather`、`hud-cockpit-tab-summary`、`hud-cockpit-context`、`hud-cockpit-vision-switch`、`hud-cockpit-vision-{optical,crt,nvg,thermal,noir}`

**Verify**：
- `npx vitest run src/globe-hud/` 全绿（含 baseline 4 失败）
- 新 ≥ 9 测试
- `npx tsc -b` rc=0
- vendor 0 行改动
- GlobeV2 cleanup 顺序：cockpit → annotation → follow → camera → visual-effects → globe.destroy()

### Task 5 — sp6 +2 + sp8 +2 + probe P9 段 + 315 验收

**Files**：
1. `scripts/accept-sp6.py`（改）— +2 检查：gev_weather 200 + body shape + gev_summary 200 + body shape
2. `scripts/accept-sp8.py`（改）— +2 检查：cockpit button + cockpit instruments DOM 三件套（不依赖 Cesium 渲染，仅 DOM 存在性）
3. `console/probe-gev.mjs`（改）— 增 P9 段
4. （如有）`scripts/_remote.py` 已有 `req()` helper；沿用

**Step details**：
1. **rsync → Debian-test**：用 AGENTS.md exclude 清单 + 新增 `--exclude '.superpowers/'`
2. **build-hub.sh + build-console.sh + restart hub-core**
3. **probe**：P8 段全绿 + P9 段：`p9-cockpit-button=1` + `p9-cockpit-frame=1` + `p9-cockpit-instruments=3` + `p9-cockpit-vision-modes=5` + `p9-briefing-panel=1` + `p9-briefing-weather=200` + `p9-briefing-summary=200`
4. **sp6 跑**：target 39+5sh/0f
5. **sp8 跑**：target 48+2sh/0f
6. **跑 sp3 + sp7 验回归**

**Verify**：
- 315 sp6/sp7/sp8/sp3 全绿
- probe exit 0
- 报告含 4 个 SP 数 + probe P9 段明细

### Task 6 — ledger + final review + merge + 410 + push

**Files**：
1. `docs/superpowers/execution/2026-09-18-gev-p9-ledger.md`（新）
2. `AGENTS.md`（改）— sp6 37+5sh/0f → 39+5sh/0f；sp8 46+2sh/0f → 48+2sh/0f

**Step details**：
1. **whole-branch final review**：dispatch deepseek-v4-pro subagent（kimi 限额同 P7/P8 ruling），提供 spec + plan + 全 diff + T1-T5 reports + reviews。Ask: coherence / type consistency / entropy check / deferred minors triage
2. **AGENTS.md update**：仅改 sp6/sp8 两行
3. **git merge --no-ff**：feat/gev-p9-cockpit-briefing → main
4. **rsync → IntelHub (410)**：exclude 清单
5. **410 build-hub.sh + build-console.sh + restart hub-core**
6. **410 acceptance**：probe + sp6/sp7/sp8/sp3
7. **git push origin main**

**Verify**：
- 410 sp6 39+5sh/0f · sp7 16+11sh/0f · sp8 48+2sh/0f · sp3 19/0
- 410 probe exit 0
- ledger commit + push 双 commit 全部已发
- Final review verdict（APPROVE/MINORS/REJECT）

## 4. 风险与缓解

| # | 风险 | 缓解 |
|---|---|---|
| R1 | cockpitCoordinator shell 耦合 | contextModePolicy 纯函数切割 + React 重写状态机（spec §0 决策） |
| R2 | request-services weather/summary 新建 | 双源 fallback（D3）+ 失败 banner 不报错（D3） |
| R3 | P6 visual-effects handle 增 getStages | T1 实施者读 P6 现状确认最小变更；可能已存在 |
| R4 | HUD 4Hz RAF 性能 | React.memo + setInterval 节流；性能 baseline < 16ms/frame |
| R5 | 视觉模式切换 stage 写冲突 | D2 两步顺序 + 单测断言不并发写 |
| R6 | vendor CSS 2188 行移植 | className 前缀替换 + T2 评估裁剪 hover 装饰 |
| R7 | kimi 限额（403 weekly） | 同 P7/P8 ruling，用 deepseek-v4-pro 做 final review |
| R8 | acled/reliefweb/gdelt 三源未实装 | T2 验证报告里明确"已有"vs"需新建"；如需新建 stub 200 + 空 bullets 优先契约 |

## 5. 验证协议

每 task 实施者须在 dispatch 后**先打开 vendor + 已上线代码**，纠正 plan 错误：
- T1: vendor cockpit* export 名可能与 plan 不一致（plan 列了"假设"导出名）
- T2: acled/reliefweb/gdelt 实际是否已有 hub 函数
- T3: vendor cockpitInstruments 是否真有 class 还是 functions
- T4: HudLayerRail 与 GlobeV2 文件路径（**P8 教训**：plan 写 `HudLeftRail.tsx` 但实际是 `HudLayerRail.tsx`）

每 task 报告 → `/Volumes/TBU/Workspace/IntelHub/.superpowers/sdd/2026-09-18-gev-p9-cockpit-briefing/task-{N}-report.md`

## 6. 熵减清单（task 4 / 6 必做）

- `grep -rn "TODO.*cockpit\|TODO.*briefing\|placeholder.*cockpit" console/src/` 应为空（vendor 例外）
- `grep -rn "cockpitController\|cockpitCoordinator\|cockpitTrackingController" console/src/` 仅 adapter 层允许
- `grep -rn "weatherEffectsMath\|hudSummaryResponse" console/src/` 仅 adapter 层允许
- hud.css 增 `.hud-cockpit-*`；删除 P8 留的 stub `line/area onSelect no-op`（如未实装确认）

## 7. 任务派发模板

每 task dispatch：
```bash
/Users/zouguojun/.pi/agent/git/github.com/obra/superpowers/skills/subagent-driven-development/scripts/dispatch-task \
  --task task-{N} --base {prev-commit-or-base} \
  --model deepseek-v4-flash  # 或 deepseek-v4-pro
```

Brief 含：plan 链接 + 验证协议 + cross-task context + 工作目录 + 验证命令 + 报告路径。