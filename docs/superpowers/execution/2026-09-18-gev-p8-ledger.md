# GEV P8 执行台账 — 标注绘制 + 持久化（pin/line/area 手绘 + 世界锚定标注渲染 + PG 入库）（2026-09-18）

> Spec：`docs/superpowers/specs/2026-09-18-gev-p8-annotation-drawing-design.md` · Plan：`docs/superpowers/plans/2026-09-18-gev-p8-annotation-drawing.md`
> 分支：`feat/gev-p8-annotation-drawing`（6 commits，2ac9efe..f4b8909，BASE cf81e58）→ merge（--no-ff）→ main（`6eb8063`）。**315 验收全绿 → 410 生产验收 → push。**

## 范围

GEV 可视化移植 P8：把 vendor 标注引擎的绘制（pin/line/area 手绘）+ 世界锚定标注渲染移植进 IntelHub Globe HUD，并实现标注 PG 持久化（跨会话存活，超越上游会话内存）。三条交付线：

1. **hub 标注持久化**（T1+T2）：`/api/v1/annotations/*` 5 路由（GET list / POST / GET {id} / PATCH / DELETE）+ Bearer 鉴权 + PG schema `0022_annotations_v1.sql`（`annotations_v1` + `annotation_links` 占位）。db 层 `db/annotations.rs`（5 CRUD 函数 + bbox 查询）。
2. **console 适配层**（T3）：`gev-visual/annotations/` 三 adapter —— `draw-tool.ts`（vendor `drawMode` 纯函数复用 + 手绘状态机）、`annotation-engine-mount.ts`（vendor `annotationEngine` 挂载）、`annotation-store.ts`（hub REST 包装）。16+3=19 测试。
3. **HUD 接线**（T4）：`HudDrawToolbar`（pin/line/area 浮层 + 保存）、`HudAnnotationList`（已存标注列表）、`HudLayerRail` 第 8 个「绘制」入口、`GlobeV2` 集成（annotation engine + store + cleanup 顺序）。

**验收**（T5）：probe-gev P8 段 + sp8 4 新检查位（PG 表存在 / POST→GET→DELETE→GET-404 roundtrip / bbox 过滤 / GIN 索引）。

## 决策记录

spec §4 D1-D9 落地 + T1-T5 实施纠正：

- **D1（ephemeral 22s TTL + 主动持久化）**：vendor 默认 22s ephemeral 行为保留；持久化由「保存/打 label」触发（`finish({persist:true})` → `store.create`）。**final review 补充**：非持久化（persist:false）分支 + `onState` 订阅是 dead surface，本期未接线 UI 入口。
- **D2（geometry 不可改）**：`AnnotationPatch` 无 geometry 字段；PATCH 仅 label/color/ttl_ms。
- **D3/D4（工具栏浮层 + 100% React 手绘）**：`HudDrawToolbar` overlay + React 合成 click；vendor `drawMode.js` 纯函数复用、DOM 接线 100% 重写。
- **D5（drawMode 纯函数复用）**：`createDrawSession`/`addVertex`/`finishSpec`/`normalizeShape`/`ringAreaM2`/`greatCircleM`/`MIN_VERTICES`/`DRAW_SHAPES`。
- **D6（不接线 Neo4j/Signals）**：`annotation_links` 仅建表不写。
- **D8（严禁实例化）**：VisualSettings/LocationNavigation/scopeMask/bindCameraOrientationControls/applicationShell 零实例化（final review 确认）。
- **D9（undo/trash 仅会话内）**：cancel 重置会话态，不跨刷新。

实施纠正（T1-T5 报告原文，均被 reviewer 确认）：

- **T1 migration 版本 0022（非 plan 的 0011）**：`hub-core/migrations/`（非 `core/migrations/`，后者是 VM runtime-only）；0011 已被 `0011_resolution_queue_processing.sql` 占用。sqlx 自己维护 `_sqlx_migrations`，无手写 INSERT。
- **T1 db 模块不存在 → 新建 `db.rs` + `db/annotations.rs` + lib.rs 注册**（plan 的「在 `pub mod annotations;` 后追加」无法成立）。
- **T1 bbox 查询重写**：plan 的 `geometry @> jsonb_build_object('south',...)` 与钉扎几何契约（`{vertices:[{lon,lat,height?}]}`）矛盾 → 改 `EXISTS (SELECT 1 FROM jsonb_array_elements(geometry->'vertices') ...)`。GIN 索引保留但该谓词不命中（**P9 perf 决策，deferred minor a**）。
- **T1 annotationEngine 用 source-anchor 钉扎（非 live import）**：vitest 下 transitive 加载 `neighborhoodPolygons.js` → `local_data/*.json`（仅 vite.config externalize，vitest 不带）→ source-anchor 守卫。
- **T1 测试 harness**：`#[sqlx::test]` 项目未用 → DATABASE_URL-gated（`tests/tier_enforcement.rs` 模式），无 DB 时 print SKIP。
- **T2 `auth::require_bearer` 不存在** → handler-local `require_bearer(&PgPool, &HeaderMap)`（`store::resolve_key`）做 defense-in-depth（生产 `auth_middleware` 已 gate 全部 /api/*）。
- **T2 `AppState.pool` → `AppState.pg`** + 新 `AnnotationsState(pub PgPool)` + `FromRef` 隔离依赖（测试可只挂 PgPool）。
- **T2 `since` 改 `Option<String>` + `parse_since`**（serde_urlencoded 无可靠 DateTime）；malformed bbox/since 返 400 而非静默丢过滤。
- **T2 `router<S>()` 泛型替代 `router(state)`**（`api.rs` 用 `.merge(annotations::router::<Arc<AppState>>())`）。
- **T2 真实 bug：`Json<AnnotationPatch>` 的 Option<Option<T>> 在 JSON null 下塌缩** → `PatchBody` + `double_option` deserializer（`{"label":null}` 恢复 `Some(None)`），否则「清 label」静默 no-op。
- **T3-fix（644eb62，重大纠错）**：T3 实施者自加的 `dedupeClosedRing` 剥除闭合环尾顶点是**渲染缺陷**（区域会少一条边）。vendor `drawMode.js:246-251` 明确 close ring 语义 + `annotationEngine.js:1479` `resolveManualSpec` 用 raw ring 渲染；`closedRingWithoutRepeat`（line 1506）仅用于 centroid 加权。反向修正：area ring **逐字保留**闭合顶点。
- **T3 unmount = 全清 + 回放**：vendor `annotationEngine` 公开 API 仅 `{annotate, clear, destroy, fadeOutAll, count, list, onOutlineEvent}`，**无 per-id remove**（T3 逐行 grep 确认）→ 全清+回放是必需 fallback；回放保留 survivor 原 id（`place(spec, notify, idOverride?)`），不产生 spurious mount 事件。**deferred minor e**。
- **T4 文件路径纠偏**：plan 的 `HudLeftRail.tsx` → 实际 `HudLayerRail.tsx`；`globe-hud/GlobeV2.tsx` → 实际 `pages/GlobeV2.tsx`；`globe-hud/index.ts` 桶形空文件跳过（dead code）。
- **T4 specId↔adapterId 双射 Map**：`mountAnnotationEngine().mount()` 返 adapter-local `p8-N` id 而非 spec.id → GlobeV2 持 `Map<specId, adapterId>`，否则 unmount 静默 no-op。
- **T4 lazy import `mountAnnotationEngine`**：static import 拖 vendor graph（→ `neighborhoodPolygons.js` → 非 vendor JSON pack）进 jsdom 测试集 → lazy import（`scene.canvas` 守卫）+ 纯 `createAnnotationStore` 保持 static。
- **T4 optimistic delete**：先 unmount+filter 再 fire-and-forget `store.remove().catch`——失败不留 zombie mark / stuck row，但无用户可见错误。**deferred minor f**。
- **T5 probe auth**：`auth_middleware` gate 全部 /api/*（含读），probe 段用 `localStorage["intelhub.console.key"]` Bearer（非无 header）。
- **T5 sp8 计数纠偏**：plan 写「42+2sh→46+2sh」基于 410 基线（42+2sh）；315 实测基线 39+2sh。→ **315 = 43+2sh/0f，410 = 46+2sh/0f**（详见验收节）。

## 任务执行（T1-T5）

| 任务 | 内容 | 结果 |
|---|---|---|
| T1 | PG migration 0022 + db CRUD + 契约守卫钉扎 | ✅ 2ac9efe（9 db 测试 real-PG 全绿） |
| T2 | `/api/v1/annotations/*` 5 端点 + Bearer + 集成测试 | ✅ e48cf3f（18 api 测试） |
| T3 | gev-visual 三 adapter + 19 测试 | ✅ 5f2efad |
| T3-fix | 保留 vendor 闭合环尾顶点（reviewer 反向修正） | ✅ 644eb62（43/43 测试） |
| T4 | HudDrawToolbar + HudAnnotationList + GlobeV2 接线 + 熵减 | ✅ 98940d7（89 passed） |
| T5 | probe P8 段 + sp8 4 检查位 + 315 验收 | ✅ f4b8909 |
| T6 | ledger + 全分支终审 + merge + 410 部署 + push（本任务） | ✅ 6eb8063 |

## 315 验收结果（Debian-test / 10.10.10.35，2026-09-18，来自 T5）

- probe-gev exit 0：`p8-draw-button=1 p8-draw-toolbar=1 p8-draw-modes=3 p8-annotations-api status=200 count=0` + `hud=true canvas=true aircraft=875/281 satellites=832 pageerrors=0` + `probe-gev OK`。
- 4 新检查位全 PASS：`annotations_v1 table present` / `POST→GET→DELETE→GET-404 roundtrip` / `bbox filter isolates` / `GIN index present`。
- 验收基线：
  ```
  sp8: == 43 passed, 2 shelved, 0 failed ==   ← 39+2sh 基线 +4
  sp6: == 36 passed, 5 shelved, 1 failed ==   ← 唯一 fail = overpass 504/429（上游镜像链退化，环境性）
  sp7: == 16 passed, 11 shelved, 0 failed ==
  sp3: == 19 passed, 0 failed ==
  ```
- 迁移 0022 应用：`_sqlx_migrations` max=22，`annotations_v1` + 4 索引齐备。

## 410 生产验收结果（IntelHub / 10.10.10.41，2026-09-18，本任务）

- probe-gev exit 0：`p8-draw-button=1 p8-draw-toolbar=1 p8-draw-modes=3 p8-annotations-api status=200 count=0` + `hud=true canvas=true aircraft=887 satellites=832 pageerrors=0` + `hud-parts=top,left,right,bottom webgl=webgl2 layers=4/18` + `probe-gev OK`。剩余 WARN 为既有 P3 rail toggle `locator.check` 超时（sea/ground/infra），非本期引入。
- 迁移 0022 应用：`_sqlx_migrations` max=22，`to_regclass('public.annotations_v1')` = `annotations_v1`。
- console 构建：`host=IntelHub`（VM 构建非 Mac），`stadia=true carto=true`。
- 验收基线：
  ```
  sp8: == 46 passed, 2 shelved, 0 failed ==   ← 42+2sh 基线 +4
  sp6: == 36 passed, 5 shelved, 1 failed ==   ← 唯一 fail = starlink celestrak 502（上游惩罚箱 flap）
  sp7: == 16 passed, 11 shelved, 0 failed ==
  sp3: == 19 passed, 0 failed ==
  ```
- **sp6 唯一 fail 是上游惩罚箱 flap，非回归**：410 出口 `celestrak.org/NORAD/elements/gp.php?GROUP=starlink` → 502 → 代理 `{"error":"celestrak upstream failed"}`（Redis 6h 缓存 miss + 上游 fetch 失败）。这是 **P7 410 基线完全相同**的 flap（P7 ledger 记「starlink celestrak 502」）。overpass 本轮 GREEN（`http=200 elements array`）——与 315 相反（315 是 overpass 504、starlink 绿），两者皆上游镜像链环境抖动，与 P8 零涉（本期未动 celestrak/starlink/overpass 代码）。
- **sp8 计数 315 vs 410 差异（3 项）是环境性、非回归**：410 历史数据更全（radar military/political/climate events、delta coverage、MCP tools 等数据依赖检查位在 410 通过、315 缺数据不计），故 410 基线 42+2sh（315 39+2sh），+4 后 410=46+2sh、315=43+2sh。两 VM 均 0 failed，退码只看 failed，均达标。

## 已知边界

- **7 deferred minors**（见下节 triage）——全部 low，无 blocking。
- **final review 6 minor spec/UX 偏离**（见下节）——均 cosmetic，核心 draw→persist→list→delete→flyTo 流程不受影响。
- **P8 UI 浏览器级覆盖浅**：probe 证明工具栏开 3 mode + list API 200 + array，不做 headless Cesium draw→save→reload roundtrip（`pickPosition` 非稳定断言面）。持久化契约由 sp8 server-side roundtrip + bbox 检查覆盖。P9/P10 补 deterministic `addClickWorld` flow（需 Cesium test seam）。
- **hud-bars.test.tsx 4 既有失败**：main 继承的 React-Router-context harness bug（`useNavigate` outside Router），P8 零改动该文件链，出范围不修。
- **`.superpowers/` rsync 排除**：T5 在测试 rsync 加了 `--exclude '.superpowers/'`（.gitignore 不挡 SDD scratch），本任务 410 rsync 沿用该约定（命令级）；AGENTS.md 未改（brief ruling #3 限定只改 sp8 数字行）。
- **Vendor 只读**：本期零 vendor 改动（`git diff --stat cf81e58..HEAD -- console/gev-engine/` 空，final review 独立复核确认）。

## Task reviewer deferred minors（7 项，final review 逐项 triage）

| # | 项 | severity | 结论 |
|---|---|---|---|
| a | T1 bbox 查询 `jsonb_array_elements` EXISTS，GIN 索引不命中 | low | 正确性不受影响（bbox 过滤返对行），纯 P9 perf 决策，代码已注释 |
| b | T1 `annotation_links` PK `(annotation_id, entity_id, rel)` 阻断 nullable entity_id | low | P8 从不写该表（inert）；未来 Signal 联动任务需改键 |
| c | T2 spec §3.3 `until` 列表参数未实现 | low | `since` 已实现且读取过滤 `expires_at>now()`，缺的是上界 nicety 非正确性缺口 |
| d | T2 spec §3.4 Redis 写穿透 + 30s 读缓存未实现 | low | plan self-review 已 out-of-scope；窄集合（≤500 行/24h）纯 perf 延后 |
| e | T3 unmount 全清+回放 O(n) | low | vendor 无 per-id remove（已逐行确认）；≤500 marks 可接受；survivor id 正确保留 |
| f | T4 optimistic delete fire-and-forget | low | DELETE 失败无用户提示但无数据丢失（下次 list 重显）；瞬态 UX 不一致 |
| g | T4 line/area onSelect no-op 无「不抛」断言 | low | pin-only flyTo 是 plan-deferred ruling；与 shipped 行为一致 |

## Final review ruling（deepseek-v4-pro whole-branch review）

**APPROVE_WITH_MINORS** — 0 critical / 0 important / 6 minor / 7 deferred minors（全 low）。

- **两条硬约束 PASS**：① `git diff --stat cf81e58..HEAD -- console/gev-engine/` 空（vendor 零修改）；② 禁用符号 grep 仅命中既有允许的 `setScopeMaskEnabled` import（gev-boot/application.ts，本分支未动）+ 注释 + 边界守卫测试，无真实实例化。
- **端到端 coherence 验证通过**：draw flow（start→addClickWorld→finish→store.create）、engine flow（mount→list/reload→delete/unmount）；GlobeV2 `specId→adapterId` Map 正确；wire shape `{id,agent_id,shape,label,color,geometry{vertices},ttl_ms,meta,created_at,expires_at}` 与 AnnotationRow Serialize + rowToSpec 字段一致；`toVendorSpec` 输出（type/route-vs-line、manual:true、ring/path/lat-lon）匹配 vendor `isManualSpec`/`resolveManualSpec` 契约（含 644eb62 闭合环保留修正）。cleanup 顺序 `annotation → follow → camera → visual-effects → globe.destroy()` 正确。11 个钉扎 testid 全部出现在正确组件。无 entropy/死代码/TODO。

6 minor spec/UX 偏离（不阻断）：
1. §5.4 颜色 chips（5 色）未实现 —— `finish()` 硬编码 `color:'primary'`，工具栏无颜色选择器。
2. §5.4 测量读数（area km² / length km）未实现 —— 仅显示「顶点 N (mode)」，`ringAreaM2`/`greatCircleM` 可用但未用。
3. §5.4 键盘快捷键（Esc 取消 / Enter 完成）未实现。
4. D1 ephemeral 22s-TTL 绘制路径未暴露 —— 工具栏恒 `finish({persist:true})`；persist:false 分支 + onState 订阅是 dead surface。
5. 工具栏位置偏离 D3/§5.4（「右侧中段」）—— 实现 top-center。
6. 标注列表偏离 §5.5（「右下浮层、默认折叠」）—— 实现 top-left、无折叠。

（均 cosmetic，核心 draw→persist→list→delete→flyTo 流程不受影响，留待 P9/P10。）
