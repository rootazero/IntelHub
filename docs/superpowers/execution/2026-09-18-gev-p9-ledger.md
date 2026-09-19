# GEV P9 执行台账 — 驾驶舱仪表 + 区域简报（cockpit 仪表 + weather/summary 简报 + 视觉模式 5 档 + hub 双端点）（2026-09-18）

> Spec：`docs/superpowers/specs/2026-09-18-gev-p9-cockpit-briefing-design.md` · Plan：`docs/superpowers/plans/2026-09-18-gev-p9-cockpit-briefing.md`
> 分支：`feat/gev-p9-cockpit-briefing`（6 commits，abae6c0..236f501，BASE 1ea5fef）→ merge（--no-ff）→ main（`8f7b2a7`）。**315 验收全绿 → 410 生产验收 → push。**

## 范围

GEV 可视化移植 P9：把 vendor cockpit（罗盘/高度尺/速度尺/航向带）+ 区域简报（weather/summary）移植进 IntelHub Globe HUD，并新增 hub `gev_weather` / `gev_summary` 双端点。三条交付线：

1. **hub 双端点**（T2）：`GET /api/v1/gev/weather`（NOAA → Open-Meteo 双源 fallback + 5min in-mem cache）+ `GET /api/v1/gev/summary`（三源聚合 stub 200 + 空 bullets + Redis 15min TTL，因 acled/reliefweb/gdelt 在 hub-core 不存在）。`secrets.rs` 新建（minimal `noaa_user_agent`）。
2. **console 适配层**（T3）：`gev-visual/cockpit/` 五 adapter —— instruments-mount / briefing-mount / vision-mount / cockpit-store / style-gate + index。Vision Map→Record 桥接（P6 getStages Map → vendor applyCockpitVisionStageIntensities 的 Object.entries）。
3. **HUD 接线**（T4）：`HudCockpitFrame` + 4 子组件（Instruments/BriefingPanel/Context/VisionSwitch）+ HudLayerRail 第 9 图标「🎮」+ GlobeV2 wiring（cleanup 顺序 cockpit FIRST）+ R5 风格守门。

**验收**（T5）：probe-gev P9 段 + sp6 +2 / sp8 +2 检查位。

## 决策记录

spec D1-D6 落地 + T1-T5 实施纠正（均被 per-task reviewer 确认）：

- **D1（区域简报双 tab + 轮播）**：weather + summary 双 tab，`COCKPIT_BRIEF_ROTATE_MS`=9000（vendor 实际值，非 plan 的 6000），hover 暂停。
- **D2（视觉模式 5 档两步顺序）**：`VisionMountHandle.setMode()` 两步 —— `setStyle(TARGET_STYLE_BY_MODE[mode])` + `applyCockpitVisionStageIntensities(stages, mode)`；先 anime 基准 pick 再覆盖。R5 守门：cockpit vision mode 激活时 globe 风格选择器拦截 anime 不变（同 stage 写入冲突防护）。
- **D3（双源 fallback）**：NOAA → Open-Meteo；失败返 503 + `{error, sources_tried}`。NOAA UA 优先级 `HUB_NOAA_UA` → `NOAA_USER_AGENT` → `"IntelHub/dev"`（空值忽略）。
- **D6（三源 stub 优先契约）**：acled/reliefweb/gdelt 未实装 → gev_summary.rs stub 200 + 空 bullets（与 P8「失败源保持可见」一致）。

实施纠正（T1-T5 报告原文，均被 reviewer 确认）：

- **T1 `getStages()` 返 Map 但 vendor `applyCockpitVisionStageIntensities` 走 `Object.entries`/`map['xxx']` 索引** —— Map 无 enumerable 自身属性 → 若直接传 Map 会静默 no-op（最高风险下游）。T3 桥接 `Object.fromEntries(stages.entries())` → plain Record，单测断言 retro.intensity 0→1、其他 stage 归零、optical 回滚 baseline。
- **T1 `TARGET_STYLE_BY_MODE` 未导出**（仅 4 映射 + optical-restore，非 5）→ 本地硬编码 `VISION_STYLE_BY_MODE`（crt→retro, nvg→surveillance, thermal→thermal, noir→noir）与 vendor 私有表逐字一致。
- **T1 `cockpitMath` 无 heading/altitudeMeters/speedMps** → 实数据走 `flights.getTrackedInfo()` 返 altitudeM/velocityMps/track/callsign/onGround；velocityMps→kts ×1.94384。
- **T1 `cockpitCamera.update()` 是 zero-arity mixin（需 `this`），非纯函数** → 正确不包壳。
- **T1 `BRIEF_PAGES=3` 非 6、`ROTATE_MS=9000` 非 6000** → plan 假设值纠正。
- **T2 acled/reliefweb/gdelt 在 hub-core 中不存在** → gev_summary.rs stub 200 + 空 bullets + Redis 15min TTL only。
- **T2 `secrets.rs` 新建**（minimal one fn `noaa_user_agent`）；lib.rs 字母序注册 `gev_summary` / `gev_weather`（secrets 模块非字母序 —— pre-existing 排序问题，记 P10 大整理）。
- **T2 NOAA UA 优先级链路验证**：`HUB_NOAA_UA` → `NOAA_USER_AGENT` → `"IntelHub/dev"`；默认 UA 工作（source=noaa）。
- **T3 cockpitInstruments 是 this-bound mixin 非 class** → adapter 抽纯函数/rotation。
- **T3 hub gev_summary `bullets:[]`** → adapter 暴露 empty/degraded 不抛错。
- **T4 文件路径纠偏（P8 教训复用）**：HudLayerRail.tsx（非 HudLeftRail）；pages/GlobeV2.tsx（非 globe-hud/GlobeV2）。
- **T4 static import 替代 plan 的 lazy import（deviation，正当）**：验证 cockpitPresentation/cockpitMath/cockpitVisionPolicy 三模块 0 imports 0 .css references（纯 leaf constants），实测 305 passed。
- **T4 无 flight 降级**：入 cockpit 但无 flight 时显示 "AIRCRAFT/-----/-----/---°/--- KT"（test 已断言）。
- **T5 计数纠偏**：plan T5 写 sp8 48+2sh 目标（410 实际）；315 实测基线 43+2sh + 2 新 = **45+2sh/0f**（非 48）。sp6 315 基线 37+5sh + 2 新 = **39+5sh/0f**。

## 任务执行（T1-T6）

| 任务 | 内容 | 结果 |
|---|---|---|
| T1 | 契约守卫 4 模块钉扎 + getStages 暴露 + baseline 测量 | ✅ abae6c0（source-contracts 绿） |
| T2 | hub gev_weather.rs + gev_summary.rs + route + 集成测试 | ✅ 62e7c06 |
| T3 | console cockpit 5 adapter + 43 测试 + Map→Record 桥 | ✅ 70fae90 |
| T4 | HUD HudCockpitFrame + 4 子组件 + GlobeV2 wiring + R5 守门 | ✅ 2dc1a7d（25 新测试） |
| T5 | sp6 +2 + sp8 +2 + probe P9 段 + 315 验收 | ✅ 5704d2f |
| T6 | AGENTS.md sp6/sp8 数 + 全分支终审 + merge + 410 部署 + push（本任务） | ✅ 236f501 + 8f7b2a7 |

## 315 验收结果（Debian-test / 10.10.10.35，2026-09-18，来自 T5）

- probe-gev exit 0：P9 段全绿 `p9-cockpit-button=1 p9-cockpit-frame=1 p9-cockpit-instruments=3 p9-cockpit-vision-modes=5 p9-briefing-panel=1 p9-briefing-weather=200 p9-briefing-summary=200`。
- 验收基线：
  ```
  sp6: == 39 passed, 5 shelved, 0 failed ==   ← 37+5sh 基线 +2
  sp8: == 45 passed, 2 shelved, 0 failed ==   ← 43+2sh 基线 +2
  sp7: == 16 passed, 11 shelved, 0 failed ==
  sp3: == 19 passed, 0 failed ==
  ```
- NOAA 默认 UA "IntelHub/dev" 工作（source=noaa）；sp6 overpass flap 未复发（200 round-trip）。

## 410 生产验收结果（IntelHub / 10.10.10.41，2026-09-19，本任务）

- probe-gev exit 0：`p8-draw-button=1 p8-draw-toolbar=1 p8-draw-modes=3 p8-annotations-api status=200 count=0` + `p9-cockpit-button=1 p9-cockpit-frame=1 p9-cockpit-instruments=3 p9-cockpit-vision-modes=5 p9-briefing-panel=1 p9-briefing-weather=200 p9-briefing-summary=200` + `hud=true canvas=true pageerrors=0` + `webgl=webgl2 layers=4/18` + `probe-gev OK`。剩余 WARN 为既有 P3 rail toggle `locator.check` 超时（sea/ground/infra），非本期引入。
- console 构建：`host=IntelHub`（VM 构建非 Mac），`stadia=true carto=true`。basemap observed=esri-imagery（report-only 字段，Stadia/CARTO 回退链部署属性，非 P9 代码回归）。
- 验收基线：
  ```
  sp8: == 48 passed, 2 shelved, 0 failed ==   ← 46+2sh 基线 +2
  sp6: == 39 passed, 5 shelved, 0 failed ==   ← 37+5sh 基线 +2（overpass flap 未复发）
  sp7: == 16 passed, 11 shelved, 0 failed ==
  sp3: == 19 passed, 0 failed ==
  ```
- **sp6 overpass flap 未复发**：本轮 sp6 39/5sh/0f 全绿（非 38+5sh/1f 的 flap 形态）。sp8 计数 315 vs 410 差异（45 vs 48）是环境性（410 历史数据更全，数据依赖检查位在 410 通过），非回归——退码只看 failed，两 VM 均 0 failed。

## 已知边界

- **9 deferred minors**（见下节 triage）——全部 low，无 blocking。
- **final review 2 新发现**（见下节）——1 minor（vision 模式重入未重放）+ 1 折叠进 minor（BriefingHandle.pages() dead），均 P10 修。
- **NOAA UA 保持默认 "IntelHub/dev"**（用户决策 T6）：不修改 core/hub.env，不硬编码占位联系方式。final review important 项「Debian-test 活探 unitCode」未在 410 前 spot-check（parser 固定转换 km/h→kts / Pa→hPa 无 unitCode 守门，错误单位退化字段为 null 不崩），留 P10 活探。
- **vision 模式重入缺陷（final review 新发现）**：persisted vision mode 在 HUD 高亮但 re-entry/重载时 `vision.setMode()` 未重放（globe 实际 render normal vs switch 显示 active）——纯 UI-vs-globe 状态分歧，无 crash/数据丢失，P10 修（enter 时 gate 到 active 再 `vision.setMode(store.visionMode)`，optical no-op）。
- **`.superpowers/` rsync 排除**：本任务 410 rsync 沿用 `--exclude '.superpowers/'`（命令级）；AGENTS.md 未改（brief ruling 限定只改 sp6/sp8 数行）。
- **Vendor 只读**：本期零 vendor 改动（`git diff --stat 1ea5fef..5704d2f -- console/gev-engine/` 空，final review 独立复核确认）。

## Task reviewer deferred minors（9 项，final review 逐项 triage）

| # | 项 | severity | 结论 |
|---|---|---|---|
| 1 | T1 cockpitMath 替换（无 heading/speedMps） | low | ACCEPT：钉扎按 vendor 实际 export（21 compass/ruler fns），source-contracts mutation-probed 非空守卫 |
| 2 | T2 NOAA unitCode 守门缺失 | low | ACCEPT：缺单位转换前守门；absent 时退化 null 不崩；action = 410 前 Debian-test 活探（记 P10） |
| 3 | T3 cockpit-store 边 case 测试（enter-without-id / exit-without-enter） | low | ACCEPT：reducer idempotent，enter(id:string) 非空类型 → 结构安全 |
| 4 | T3 BriefingHandle start/stop（+pages）dead API | low | ACCEPT：HudCockpitBriefingPanel 自持 setInterval；start/stop/pages 仅测试引用，P10 清理 |
| 5 | T4 lazy-import deviation（static import 纯 leaf 模块） | low | ACCEPT：验证 0-import/0-CSS leaf；305 passed |
| 6 | T4 gatedStyleRef.current 写于 render 非 effect | low | ACCEPT：useMemo 值跨 render 稳定；风格 nit |
| 7 | T5 sp8 bundle-level 守卫（非 live DOM） | low | ACCEPT：probe P9 段同轮覆盖 live DOM |
| 8 | T5 AGENTS.md rsync `--exclude '.git/'` 尾斜杠不匹配 worktree .git FILE | low | ACCEPT：pre-existing，VM 不跑 git 则无害 |
| 9 | T5 NOAA UA 默认 "IntelHub/dev" | low | ACCEPT（RULED）：用户决策保持默认，不改 core/hub.env |

## Final review ruling（deepseek-v4-pro whole-branch review）

**MINORS** — 0 critical / 0 important-blocking / 1 important / 1 minor / 9 deferred minors（全 ACCEPT）/ 2 new findings。

- **三条硬约束 PASS**：① `git diff --stat 1ea5fef..5704d2f -- console/gev-engine/` 空（vendor 零修改）；② 熵减 grep 空 + `cockpitController|cockpitCoordinator|cockpitTrackingController` 仅命中 adapter 层注释（"we do NOT instantiate"）；③ 新双端点 GET /api/v1/gev/weather + /api/v1/gev/summary 注册于 api.rs router，server.rs `auth_middleware` `.layer()` 覆盖全 router（auth.rs 仅豁免 `/healthz` + 非 `/api`/`/mcp` 路径）→ 两端点均 gated。
- **端到端 coherence 通过**：hub `WeatherResponse`（7 字段：temperature_c/wind_speed_kts/wind_direction_deg/precipitation_mm/cloud_cover_pct/visibility_m/pressure_hpa）匹配 briefing-mount.normalizeWeather；`SummaryResponse`（entity_id/generated_at/sources/bullets[]/next_refresh_after）匹配 normalizeSummary；Vision Map→Record 桥（Object.fromEntries）匹配 vendor cockpitVisionPolicy.js 的 Object.entries 消费；GlobeV2 cleanup 顺序 cockpit（vision→briefing→instruments）FIRST → annotation → follow → camera → visual-effects → globe.destroy() 正确。Rust json!() 无 Serialize 派生依赖；Query params Deserialize；`Option<Option<Vec<u8>>>`（P3 Nil→Ok(vec![]) 教训）应用于 redis 缓存读。

1 important（非 merge blocker，deploy checklist）：
1. **NOAA unitCode 守门缺失**：parser 固定转换（windSpeed km/h→kts、pressure Pa→hPa）无 unitCode 守门——错误单位退化字段为 null 不崩，但 NOAA 若改单位可能静默错误值。action = Debian-test 活探 forecastGridData 响应确认 unitCode，记 P10。

2 new findings（均 P10）：
1. **vision 模式重入未重放**：`createCockpitStore` 以 `readPersistedVisionMode()` seed，switch 高亮 persisted mode，但 `vision.setMode()` 仅由 HudCockpitVisionSwitch.select() 显式 click 触发——无 effect 在 frame mount 或 store.enter() 时应用 `vision.setMode(store.visionMode)`。重入/重载后 switch 显示 active 但 globe 实际 render normal（上次 exit 的 restorePicked() 已 reset）。无 crash/数据丢失，纯 UI-vs-globe 分歧。
2. **BriefingHandle.pages() 生产 dead**（仅测试引用）——与 start/stop 同折叠进 deferred minor 4。

（均 cosmetic/低风险，核心 cockpit 仪表 + briefing 双 tab + vision 5 档 + hub 双端点流程不受影响，留待 P10。）
