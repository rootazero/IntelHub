# GEV P8 标注绘制与持久化 设计

> 日期：2026-09-18 · 状态：草稿（待用户批准）
> 性质：分期 P8，隶属于 `docs/superpowers/specs/2026-09-18-gev-visual-port-design.md` 总纲的"标注绘制"分期
> 上游文档：`2026-09-18-gev-visual-port-design.md` §3 P8 + 风险 §5/§6
> 同期姊妹：P6 视觉预设已上线（main `30a709b`），P7 相机姿态+搜索已上线（main `da166a1`）

## 0. 目标与边界

### 目标
将 GEV 引擎 `console/gev-engine/src/annotations/` 的 **手绘 pin/line/area 工具 + 世界锚定标注（脉冲环/弯箭头/callout 卡）+ area drape 贴地渲染** 移植进 IntelHub Globe HUD；并实现 **标注 PG 持久化**（上游不存，IntelHub 跨会话存活）。新增 `console/src/gev-visual/annotations/` 适配层 + `console/src/globe-hud/` 左轨工具栏 + hub 新建 `annotation` 资源。

### 显式不做的（与总纲一致）
- **不进** `cockpitInstrument`、`cockpitBriefing`、`visualPresets` 内联动（属 P9/P6）
- **不进** panel 拖拽（属 P10）
- **不复制** vendor 冗余写入：每个 annotation 只走一条通路（PG primary，vendor 内存为 ephemeral buffer）
- **不实例化** `VisualSettings`/`scopeMask`/`applicationShell`（vendor 单例禁区）

### 设计原则
- 沿用方案 1 渲染核直引 + React 壳；adapter 窄契约；vendor 零修改
- 标注为世界锚定（lon/lat），跟随相机；TTL 持久态并存（vendor 既有 22s 默认 TTL）
- 标注 CRUD 与 Neo4j/Signals 联动**不在本期**（§4 决策）

## 1. 现状盘点（Gap Analysis）

### vendor 已有
| 模块 | 行数 | 角色 |
|---|---|---|
| `annotationEngine.js` | 1569 | 状态机 + 渲染调度 + TTL/fade |
| `drawTool.js` | 546 | 手绘工具（DOM 接线 + 监听 canvas 鼠标） |
| `drawMode.js` | 303 | 几何计算（greatCircleM、ringAreaM2、finishSpec 等纯函数） |
| `worldAnnotationRenderer.js` | 542 | 地面/空中锚点（脉冲环、callout、弯箭头） |
| `screenAnnotationRenderer.js` | 857 | 屏幕空间层 |
| `hybridAnnotationRenderer.js` | 158 | 合成层（world + screen） |
| `resolver.js` | 1945 | 名称 → 几何 + outline Overpass 回退 |

**vendor 模式**：drawTool 持有自己的 DOM 监听 + Cesium `ScreenSpaceEventHandler`，自己处理 vertex 选取、双击收尾、preview polyline。**这种壳-事件耦合不直接复用**——React HUD 必须重写交互层，只复用几何/finishSpec 纯函数 + 引擎渲染调度。

### IntelHub shell 现状（0 接线）
- `console/src/gev-visual/` 已有 camera-orientation / follow-controller / location-search / visual-effects 4 个 adapter（**无任何 annotation adapter**）
- `console/src/globe-hud/HudLeftRail.tsx` 当前含 7 个图标（详情面板/分析/搜索等），**无绘制入口**
- hub-core **无 annotations 表、无 REST 端点**（PG schema 当前 0010）

## 2. 总体架构

```
React HUD
  HudLeftRail 增「绘制」入口 → 浮出 HudDrawToolbar
  HudDetailPanel 可绑定当前标注（点选跳转 + 详情卡）
    │
适配层 ★ console/src/gev-visual/annotations/（新目录）
  draw-tool.ts        pin/line/area 三模态 + 预览态
  annotation-store.ts React state ↔ annotationEngine 同步
  annotation-persist.ts  PG CRUD 包装（apiFetch）
  annotation-list.ts 列表渲染（按时间/距离）
    │ import（只读）
vendor 渲染核
  annotationEngine     状态机（保留 vendor 22s TTL 行为）
  drawMode 纯函数       几何/finishSpec（pin/line/area 共用）
  hybridAnnotationRenderer
  worldAnnotationRenderer
  screenAnnotationRenderer
    │
hub-core（本期新建）
  /api/v1/annotations GET/POST/PATCH/DELETE
  PG 表 annotations_v1（jsonb geometry）+ annotation_links（信号关联预留）
```

## 3. 数据契约

### 3.1 vendor annotation 形态（输入到渲染核）
```ts
interface AnnotationSpec {
  id: string;                    // uuid, 由 hub 生成
  shape: 'pin' | 'line' | 'area';
  geometry: {                    // 世界锚定
    vertices: Array<{ lon: number; lat: number; height?: number }>;
  };
  label?: string;
  color: 'primary' | 'amber' | 'cyan' | 'green' | 'red';
  ttl_ms?: number;               // 默认 null = 持久
  meta?: Record<string, unknown>;
}
```
**adapter 负责**：`AnnotationSpec ↔ vendor annotation engine API`，不破坏 vendor 既有 `annotate()` 签名的前提下加 `id`/`persist` 通道。

### 3.2 PG schema（迁移 `0011_annotations_v1.sql`）

```sql
CREATE TABLE annotations_v1 (
  id              uuid PRIMARY KEY,
  agent_id        text,                       -- nullable（手绘可无 agent）
  shape           text NOT NULL CHECK (shape IN ('pin','line','area')),
  label           text,
  color           text NOT NULL DEFAULT 'primary',
  geometry        jsonb NOT NULL,             -- {vertices:[{lon,lat,height?},...]}
  ttl_ms          integer,                    -- null = 持久
  meta            jsonb NOT NULL DEFAULT '{}'::jsonb,
  created_at      timestamptz NOT NULL DEFAULT now(),
  expires_at      timestamptz,                -- ttl_ms 派生（null = 永不过期）
  UNIQUE (id)
);
CREATE INDEX annotations_v1_created_idx ON annotations_v1 (created_at DESC);
CREATE INDEX annotations_v1_expires_idx ON annotations_v1 (expires_at) WHERE expires_at IS NOT NULL;
CREATE INDEX annotations_v1_geom_idx     ON annotations_v1 USING GIN (geometry jsonb_path_ops);
```

**annotation_links 表**（预留）：`annotation_id uuid FK + entity_id uuid FK + rel text`，本期建表不接线（保留给后续 Signal 联动，零成本）。

### 3.3 hub REST surface

| 端点 | 行为 | 备注 |
|---|---|---|
| `GET /api/v1/annotations?since=&until=&bbox=` | 列 | 默认 last 24h；bbox 用于空间查询 |
| `POST /api/v1/annotations` | 创建 | body = AnnotationSpec；返回 id + expires_at |
| `PATCH /api/v1/annotations/{id}` | 更新 label/color/ttl | 部分更新；不允许改 geometry（删除重建） |
| `DELETE /api/v1/annotations/{id}` | 删除 | 物理删除 |
| `GET /api/v1/annotations/{id}` | 单条详情 | 用于点击列表行 → 飞向 |

**鉴权**：沿用 Bearer `ihk_*`（与 GEV 代理同一 agent 名册）；不必 agent_id 必填。

**去重**：etag 客户端可 `If-Match`（迁移后可加 v2 audit columns）。本期不做。

### 3.4 Redis 缓存
- **写穿透**：POST/PATCH/DELETE 同步 invalidate `hub:annotations:since:{ts}`（range query 不强依赖，TTL 60s 即可）
- **读取**：`GET` 短 TTL 30s（用户视图，频繁轮询压 1×QPS）

## 4. 显式决策（与总纲 §6 风险联动）

| # | 决策 | 理由 |
|---|---|---|
| D1 | **保留 vendor 默认 22s TTL 行为**作为 ephemeral 标注（手绘完成后无标签的临时高亮），同时支持持久化（用户主动「保存」/打 label 即入 PG） | 与 vendor 既有 UX 对齐；避免破坏"瞬时白板"语义 |
| D2 | **geometry 不可改，只可重建**（PATCH 不允许改 vertices） | 与 vendor annotation spec 锁定；减少并发冲突；改几何 = 删除重建 |
| D3 | **绘制工具栏进 HudLeftRail 的浮层**，不占主轨 | HudLeftRail 7 个图标已满；浮层（HudDrawToolbar overlay）在用户点绘制入口时显示在画面右侧中段 |
| D4 | **手绘交互 100% React 重写**：click 监听在 canvas 上 React 合成，**不复用** drawTool.js 的 ScreenSpaceEventHandler / 自身 DOM 接线 | React 壳路径需要 React state 与交互同步；vendor 那段耦合到 `applicationShell` 隐式状态 |
| D5 | **复用 drawMode.js 纯函数**：`finishSpec`、`normalizeShape`、`ringAreaM2`、`greatCircleM`（不依赖 DOM/Cesium 单例） | 数学层零冲突；测试可与 vendor 一致 |
| D6 | **不接线 Neo4j/Signals**（annotation_links 仅建表不写） | §6 风险 5 留待 P11+ 决策；避免 P8 范围蔓延 |
| D7 | **geocode 复用 P7 photon 代理**（不变） | 已上线、可用 |
| D8 | **不引入 LocationNavigation / VisualSettings / scopeMask 实例化**（总纲铁律） | 全局约束 |
| D9 | **trash can / undo 仅会话内**（不跨刷新） | vendor drawTool 既有 clear 行为；P10 才做跨会话抽屉 |

## 5. 模块详细

### 5.1 `console/src/gev-visual/annotations/draw-tool.ts` (新)
**契约**：
```ts
export type DrawMode = 'pin' | 'line' | 'area' | null;

export interface DrawToolHandle {
  start(mode: 'pin' | 'line' | 'area'): void;
  cancel(): void;
  finish(opts?: { label?: string; persist?: boolean }): Promise<AnnotationSpec | null>;
  onVertex(cb: (v: { lon: number; lat: number }) => void): () => void;  // unsubscribe
  onPreview(cb: (vertices: Array<{ lon: number; lat: number }>) => void): () => void;
  onState(cb: (state: 'idle' | 'drawing' | 'finishing') => void): () => void;
  destroy(): void;
}
export function mountDrawTool(viewer: { scene: { canvas: HTMLCanvasElement; pickPosition: (windowPos, x, y) => Cartesian3 | undefined } }): DrawToolHandle;
```
- `viewer.scene.pickPosition` 是 vendor drawTool 同款入口；React 端在 canvas mousedown 调一次拿经纬度
- `start(mode)` 创建 vendor `drawMode.createDrawSession(mode)`；每 mousedown 调 `addVertex`；finish 调 `finishSpec(session, {label, color:'primary'})`
- `finish()` 写入 `annotationEngine.annotate({...finishSpec, persist})`；persist=true 触发 annotation-persist POST
- **mock 校验**：`viewer.scene.pickPosition` 必须存在且返回 Cartesian3-like；构造器 throw TypeError（防 lenient-mock 坑）

### 5.2 `console/src/gev-visual/annotations/annotation-store.ts` (新)
**契约**：
```ts
export interface AnnotationStore {
  list(): Promise<AnnotationSpec[]>;          // GET /api/v1/annotations?since=...
  create(spec: AnnotationSpec): Promise<AnnotationSpec>;
  patch(id: string, patch: { label?: string; color?: string; ttl_ms?: number }): Promise<AnnotationSpec>;
  remove(id: string): Promise<void>;
  get(id: string): Promise<AnnotationSpec>;
}
export function createAnnotationStore(apiFetch: typeof fetch): AnnotationStore;
```

### 5.3 `console/src/gev-visual/annotations/annotation-engine-mount.ts` (新)
**契约**：
```ts
export interface AnnotationEngineHandle {
  mount(spec: AnnotationSpec): string;        // returns vendor annotation id
  unmount(id: string): void;
  list(): Array<{ id: string; spec: AnnotationSpec; createdAt: number }>;
  subscribe(cb: (events: AnnotationEvent[]) => void): () => void;
  destroy(): void;
}
export function mountAnnotationEngine(viewer: Viewer): AnnotationEngineHandle;
```
- 内部 = `createAnnotationEngine({viewer, renderer: createHybridAnnotationRenderer(viewer), placeSearch: undefined, resolveTarget: undefined})`（P8 不接 geocode、不接 resolver；D6/D7 决策）
- adapter 维护 `vendor annotation id ↔ our AnnotationSpec.id` 双向映射

### 5.4 `console/src/globe-hud/HudDrawToolbar.tsx` (新)
- **入口**：HudLeftRail 第 8 个图标「绘制」(icon: 编辑笔)
- **浮层**：右侧中段（避开顶条/底条），360×自适应高度
- **内容**：
  - 模态切换 chip：📍 pin / 〰️ line / ⬡ area
  - 当前顶点计数 + 测量（面积 km² / 长度 km）
  - 「保存」（persist=true） / 「取消」按钮
  - 标签输入框（可空）
  - 颜色 chip（5 色）
- **键盘**：Esc 取消；Enter 在 ≥ MIN_VERTICES 时 finish

### 5.5 `console/src/globe-hud/HudAnnotationList.tsx` (新)
- 右下浮层（默认折叠，点击展开）
- 列：最近 10 条标注（按 created_at desc）
- 行：`[图标] label | color | shape`
- 点击 → flyTo（经纬度 + bbox；line/area 用 centroid + 半径 5km 默认；pin 用 1km）
- 行尾 ⓧ 删除（带 confirm）

### 5.6 `GlobeV2.tsx` 集成
- 增 `annotationStore = createAnnotationStore(apiFetch)`（useMemo）+ `annotationEngine = mountAnnotationEngine(viewer)`（在 .then() 同 P6/P7）
- 启动时 `annotationStore.list()` → 全部 mount 到 engine（**不** batch 渲染以避免 frame storm；分批 10/tick）
- cleanup 顺序：**annotation → camera → visualEffects → globe.destroy()**
- `useGlobeSelection` 复用：手绘 finish 后 set selected = new annotation

### 5.7 hub-core 模块
- `hub-core/src/api/annotations.rs`：4 端点 + 鉴权
- `hub-core/src/db/annotations.rs`：CRUD + bbox query（jsonb `@>` 简单实现）
- `lib.rs`：字母序 mod 注册
- `api.rs`：`/api/v1/annotations/*` 路由
- `core/migrations/0011_annotations_v1.sql`

## 6. 测试与验收

### 适配层单测（vitest）
- `draw-tool.test.ts`（6 测试）：构造器契约 / 三模态 start / vertex add / finish persist vs ephemeral / cancel clear / destroy cleanup
- `annotation-engine-mount.test.ts`（5 测试）：mount/unmount/list/subscribe/destroy
- `annotation-store.test.ts`（5 测试）：API 形状 + 错误传播 + 鉴权头
- 全部沿用 lenient-mock 防线（构造器断言 viewer.scene.pickPosition 存在）
- fake vendor `annotationEngine.annotate()` 收到正确 spec 形状

### 契约守卫扩展
`source-contracts.test.ts` 增：
- `drawMode`、`annotationEngine`、`hybridAnnotationRenderer`、`worldAnnotationRenderer`、`screenAnnotationRenderer` 模块导出签名快照
- `createDrawSession` / `addVertex` / `finishSpec` / `createAnnotationEngine` / `createHybridAnnotationRenderer` arity 锁定

### probe 扩展
`console/probe-gev.mjs` 加 P8 段（在 P7 后）：
- DOM 检查：HudDrawToolbar 浮层 DOM 存在（无 viewer 时不渲染——vs P6 滤镜是常驻）
- 计数检查：标注列表数 ≥ 0；新绘 → 列表数 +1
- DB 检查：`GET /api/v1/annotations` 返 ≥ 1 条后（如果有持久化历史），DOM 引擎实体数与 DB 数一致

### sp8 验收扩展
新增 4 检查位（passed/shelved/failed 三态）：
1. **annotations_table_present**：`SELECT to_regclass('annotations_v1')` = `annotations_v1`
2. **annotation_post_persist_roundtrip**：POST 一个 spec → GET 返同 id → DELETE → GET 404
3. **annotation_bbox_filter**：`?bbox=-10,-10,10,10` 返坐标在范围内的标注
4. **annotation_geojson_index**：GIN 索引存在 + EXPLAIN 使用（query planner hit）

### 315 验收基线预期
- sp8：46+2sh/0f（P7 42 + P8 4）
- sp6/sp7/sp3 维持不变

## 7. 风险与回滚

| 风险 | 缓解 |
|---|---|
| vendor `pickPosition` 在某些 viewport 配置下返 undefined（camera angle 太斜） | draw-tool 测点 mousedown 失败时给用户 toast "无法在该视角下选点，请调整视角"（zh/en），不 throw |
| annotation TTL 与 PG expires_at 不一致（vendor 22s 不与 PG 同步） | adapter 启动时 reconcile：加载 PG 列表 → 过滤 expires_at > now 的；其余 vendor 不挂载 |
| PG jsonb 几何查询性能（>10k 标注时） | gin 索引 + bbox 简单实现；不实现 polygon-in-polygon（P9+ 决策） |
| REST 与 vendor annotation id 冲突 | 双层映射表（adapter 内部）；vendor 用 numeric id / ours 用 uuid，**不** 复用一个命名空间 |
| 月度 vendor 同步 churn 改动 `finishSpec` 或 `createAnnotationEngine` 签名 | 契约守卫 red → 本期 spec 内 adapter 同步更新（保险丝生效） |

## 8. 工期与任务切片（建议 6 任务，参照 P7 节奏）

| Task | 内容 | 估时 |
|---|---|---|
| T1 | 契约守卫钉扎 + hub migration `0011` + PG CRUD 单测 | 30min |
| T2 | hub `/api/v1/annotations/*` 4 端点 + `db/annotations.rs` + 集成测试 | 40min |
| T3 | `gev-visual/annotations/` 3 个 adapter（draw-tool / engine-mount / store）+ 单测 | 60min |
| T4 | HudDrawToolbar + HudAnnotationList + GlobeV2 集成 + 熵减 | 50min |
| T5 | probe P8 段 + sp8 4 新检查位 | 30min |
| T6 | ledger + 全分支终审 + 合并 main + 410 部署 + push | 40min |

预计 4-5 小时交付（不含 315/410 验收等待）。

---

**审查 checklist**（自审）：

- [x] 总纲 §2 架构守则：方案 1 + adapter 单职责窄契约 + vendor 零修改
- [x] 总纲 §3 P8 出口标准：左轨工具栏 + PG 持久化 + sp8 4 检查位
- [x] 总纲 §4 契约守卫：扩 `drawMode` / `annotationEngine` / 3 renderer
- [x] 总纲 §5 测试与验收：单测 + probe + sp8 + 熵减 + 315/410
- [x] 总纲 §6 风险 5（持久化决策）：§3 schema + §4 D6 不接线 Neo4j
- [x] 总纲 §6 风险 6（panel 耦合）：P10 风险；P8 不进 panel
- [x] 无占位/TODO/模糊；所有决策有 D# 编号
- [x] 范围聚焦（单期 spec）

**Approval gate**：用户确认 §4 决策（D1-D9）、§3 数据契约、§5 模块切片后可开工。
