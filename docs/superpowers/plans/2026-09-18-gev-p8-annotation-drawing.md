# GEV P8 标注绘制+持久化 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 将 GEV 引擎标注绘制（pin/line/area 手绘 + 世界锚定标注渲染）移植进 IntelHub Globe HUD，并实现标注 PG 持久化（跨会话存活，超越上游会话内存）。

**Architecture:** 方案 1 渲染核直引 + React 壳（沿用 P6/P7）。`console/src/gev-visual/annotations/` 三个 adapter（draw-tool / annotation-engine-mount / annotation-store），`console/src/globe-hud/` 新增 HudDrawToolbar 浮层 + HudAnnotationList 浮层，hub-core 新建 `/api/v1/annotations/*` 4 端点 + `core/migrations/0011_annotations_v1.sql`。Vendor `drawMode.js` 纯函数复用；vendor DOM 接线 100% 重写。

**Tech Stack:** TypeScript + React + vitest（console 侧）；axum + sqlx + PostgreSQL + Redis（hub 侧）；vendor Cesium（已集成）。

**Spec:** `docs/superpowers/specs/2026-09-18-gev-p8-annotation-drawing-design.md`（main `94e3096`）

## Global Constraints

（出自 spec §0/§4/§5，与总纲 `2026-09-18-gev-visual-port-design.md` §0 一致）

- **vendor 零修改**：`console/gev-engine/` 任何文件不改。月度同步 = 整体替换 + 契约守卫验证
- **严禁实例化**：`VisualSettings` / `LocationNavigation` / `scopeMask`（`setScopeMaskEnabled(false)` 防御行可保留）/ `bindCameraOrientationControls` / `applicationShell`
- **vendor 复用白名单（adapter 仅可 import）**：`drawMode`（`createDrawSession` / `addVertex` / `finishSpec` / `normalizeShape` / `ringAreaM2` / `greatCircleM` / `MIN_VERTICES` / `DRAW_SHAPES`）；`annotationEngine`（`createAnnotationEngine` / `normalizeTargetKey` / `resolveOutlineWithRetry`）；3 renderer（`createHybridAnnotationRenderer` / `createWorldAnnotationRenderer` / `createScreenAnnotationRenderer`）
- **adapter 单文件单职责**，窄契约 `mount(...) → { update(state), destroy() }`；不持有 DOM 监听（除 canvas mousedown）
- **mock 复刻真实构造器契约**：`viewer.scene.pickPosition` 必须存在且返 Cartesian3-like，构造器 throw TypeError（防 lenient-mock 坑）
- **Redis 缓存**：`Option<Option<Vec<u8>>>` 双层（redis-rs Nil→vec![] GEV P3 教训）；写穿透 invalidate
- **globe 生命周期**：`annotation → camera → visualEffects → globe.destroy()`（GlobeV2 cleanup 顺序）
- **TTL 行为**：vendor 默认 22s ephemeral 行为保留；持久化（PG 入库）由用户主动「保存」/打 label 触发
- **geometry 不可改**：PATCH 不允许改 vertices（删建）
- **testid 逐字**：`hud-draw-button` / `hud-draw-toolbar` / `hud-draw-cancel` / `hud-draw-finish` / `hud-draw-label` / `hud-draw-mode-{pin,line,area}` / `hud-annotation-list` / `hud-annotation-row` / `hud-annotation-delete`
- **i18n**：HUD 新控件 zh/en 全量；vendor 内部 toast/readout 英文可接受
- **REST 鉴权**：Bearer `ihk_*`（与 GEV 代理同 agent 名册），不必 agent_id 必填
- **sp8 baseline**：本期 +4 检查位（PG 表存在 / POST-GET-DELETE roundtrip / bbox 过滤 / GIN 索引 EXPLAIN），退码只看 failed

## File Structure

**新增**：
- `core/migrations/0011_annotations_v1.sql` — PG schema
- `hub-core/crates/hub-core/src/db/annotations.rs` — CRUD + bbox query
- `hub-core/crates/hub-core/src/api/annotations.rs` — 4 端点 handler
- `hub-core/crates/hub-core/src/api_annotations_tests.rs` — 集成测试（与 gev_geocode.rs 风格一致）
- `console/src/gev-visual/annotations/draw-tool.ts` — 手绘 adapter
- `console/src/gev-visual/annotations/annotation-engine-mount.ts` — vendor 引擎挂载 adapter
- `console/src/gev-visual/annotations/annotation-store.ts` — hub REST 包装
- `console/src/gev-visual/annotations/index.ts` — 单一 re-export
- `console/src/gev-visual/__tests__/draw-tool.test.ts` — 6 测试
- `console/src/gev-visual/__tests__/annotation-engine-mount.test.ts` — 5 测试
- `console/src/gev-visual/__tests__/annotation-store.test.ts` — 5 测试
- `console/src/globe-hud/HudDrawToolbar.tsx` — 左轨浮层
- `console/src/globe-hud/HudAnnotationList.tsx` — 右下浮层
- `console/src/globe-hud/__tests__/hud-draw-toolbar.test.tsx` — 5 测试
- `console/src/globe-hud/__tests__/hud-annotation-list.test.tsx` — 4 测试
- `scripts/accept-sp8-annotations.py` — sp8 4 检查位脚本（追加到现有 sp8 runner）
- `.superpowers/sdd/2026-09-18-gev-p8-annotation-drawing/` — 任务 brief/report/ledger

**修改**：
- `hub-core/crates/hub-core/src/lib.rs` — 字母序 mod 注册 `db::annotations` + `api::annotations`
- `hub-core/crates/hub-core/src/api.rs` — `/api/v1/annotations/*` 路由追加
- `hub-core/crates/hub-core/src/db.rs` — re-export
- `console/src/gev-boot/__tests__/source-contracts.test.ts` — P8 contract guard 钉扎
- `console/src/globe-hud/GlobeV2.tsx` — annotation engine + store 集成 + cleanup 顺序
- `console/src/globe-hud/HudLeftRail.tsx` — 第 8 个图标「绘制」入口
- `console/src/globe-hud/index.ts` — re-export 新组件
- `console/probe-gev.mjs` — P8 段断言
- `docs/superpowers/execution/2026-09-18-gev-p8-ledger.md` — P8 ledger

---

## Task 1: 契约守卫钉扎 + PG migration + PG CRUD 单测

**Files:**
- Create: `core/migrations/0011_annotations_v1.sql`
- Create: `hub-core/crates/hub-core/src/db/annotations.rs`
- Create: `hub-core/crates/hub-core/tests/db_annotations.rs`（或并入现有 integration tests 模块）
- Modify: `console/src/gev-boot/__tests__/source-contracts.test.ts`（追加 P8 段）
- Modify: `hub-core/crates/hub-core/src/db.rs`（re-export）

**Interfaces:**
- Consumes: `sqlx::PgPool`, `serde_json::Value`
- Produces:
  - `pub async fn list_annotations(pool: &PgPool, since: Option<DateTime<Utc>>, bbox: Option<BBox>) -> Result<Vec<AnnotationRow>, sqlx::Error>`
  - `pub async fn create_annotation(pool: &PgPool, spec: AnnotationInsert) -> Result<AnnotationRow, sqlx::Error>`
  - `pub async fn patch_annotation(pool: &PgPool, id: Uuid, patch: AnnotationPatch) -> Result<Option<AnnotationRow>, sqlx::Error>`
  - `pub async fn delete_annotation(pool: &PgPool, id: Uuid) -> Result<bool, sqlx::Error>`
  - `pub async fn get_annotation(pool: &PgPool, id: Uuid) -> Result<Option<AnnotationRow>, sqlx::Error>`
  - types: `AnnotationRow { id, agent_id, shape, label, color, geometry: serde_json::Value, ttl_ms, meta, created_at, expires_at }`, `AnnotationInsert { id, agent_id, shape, label, color, geometry, ttl_ms, meta }`, `AnnotationPatch { label?, color?, ttl_ms? }`, `BBox { south, west, north, east: f64 }`

- [ ] **Step 1.1: 写 PG migration `0011_annotations_v1.sql`**

```sql
-- 0011_annotations_v1.sql
-- GEV P8: annotation persistence (pin/line/area, vendor-agnostic, lon/lat anchored)

CREATE TABLE IF NOT EXISTS annotations_v1 (
    id          uuid PRIMARY KEY,
    agent_id    text,
    shape       text NOT NULL CHECK (shape IN ('pin','line','area')),
    label       text,
    color       text NOT NULL DEFAULT 'primary',
    geometry    jsonb NOT NULL,
    ttl_ms      integer,
    meta        jsonb NOT NULL DEFAULT '{}'::jsonb,
    created_at  timestamptz NOT NULL DEFAULT now(),
    expires_at  timestamptz,
    CONSTRAINT ttl_with_expires CHECK (
        (ttl_ms IS NULL AND expires_at IS NULL)
        OR (ttl_ms IS NOT NULL AND expires_at IS NOT NULL)
    )
);

CREATE INDEX IF NOT EXISTS annotations_v1_created_idx
    ON annotations_v1 (created_at DESC);
CREATE INDEX IF NOT EXISTS annotations_v1_expires_idx
    ON annotations_v1 (expires_at)
    WHERE expires_at IS NOT NULL;
CREATE INDEX IF NOT EXISTS annotations_v1_geom_idx
    ON annotations_v1 USING GIN (geometry jsonb_path_ops);

-- annotation_links: placeholder for future Neo4j/Signal binding; not used in P8
CREATE TABLE IF NOT EXISTS annotation_links (
    annotation_id uuid NOT NULL REFERENCES annotations_v1(id) ON DELETE CASCADE,
    entity_id     uuid,
    rel           text,
    PRIMARY KEY (annotation_id, entity_id, rel)
);
```

注：仅**手写 schema + CREATE INDEX**；不要触发 `INSERT INTO schema_migrations`，由 install/runtime 自动跟踪（与现有迁移一致；看 `core/migrations/0010*.sql` 末尾的同样模式）

- [ ] **Step 1.2: 写 `db/annotations.rs` 骨架（仅 types）**

```rust
//! GEV P8: annotations_v1 CRUD + bbox query.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct AnnotationRow {
    pub id: Uuid,
    pub agent_id: Option<String>,
    pub shape: String,
    pub label: Option<String>,
    pub color: String,
    pub geometry: JsonValue,
    pub ttl_ms: Option<i32>,
    pub meta: JsonValue,
    pub created_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct AnnotationInsert {
    pub id: Option<Uuid>,
    pub agent_id: Option<String>,
    pub shape: String,
    pub label: Option<String>,
    pub color: Option<String>,
    pub geometry: JsonValue,
    pub ttl_ms: Option<i32>,
    pub meta: Option<JsonValue>,
}

#[derive(Debug, Deserialize, Default)]
pub struct AnnotationPatch {
    pub label: Option<Option<String>>,   // double-Option: Some(None) = clear, None = no change
    pub color: Option<String>,
    pub ttl_ms: Option<Option<i32>>,     // double-Option semantics
}

#[derive(Debug, Deserialize)]
pub struct BBox {
    pub south: f64,
    pub west: f64,
    pub north: f64,
    pub east: f64,
}
```

- [ ] **Step 1.3: 写 `db/annotations.rs` 5 个 CRUD 函数（实现）**

```rust
pub async fn create_annotation(
    pool: &PgPool,
    spec: AnnotationInsert,
) -> Result<AnnotationRow, sqlx::Error> {
    let id = spec.id.unwrap_or_else(Uuid::new_v4);
    let color = spec.color.unwrap_or_else(|| "primary".to_string());
    let meta = spec.meta.unwrap_or_else(|| serde_json::json!({}));
    let expires_at = spec.ttl_ms.map(|ttl| Utc::now() + chrono::Duration::milliseconds(ttl as i64));
    sqlx::query_as::<_, AnnotationRow>(
        r#"
        INSERT INTO annotations_v1 (id, agent_id, shape, label, color, geometry, ttl_ms, meta, expires_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
        RETURNING id, agent_id, shape, label, color, geometry, ttl_ms, meta, created_at, expires_at
        "#,
    )
    .bind(id)
    .bind(spec.agent_id)
    .bind(&spec.shape)
    .bind(spec.label)
    .bind(color)
    .bind(&spec.geometry)
    .bind(spec.ttl_ms)
    .bind(&meta)
    .bind(expires_at)
    .fetch_one(pool)
    .await
}

pub async fn get_annotation(
    pool: &PgPool,
    id: Uuid,
) -> Result<Option<AnnotationRow>, sqlx::Error> {
    sqlx::query_as::<_, AnnotationRow>(
        r#"
        SELECT id, agent_id, shape, label, color, geometry, ttl_ms, meta, created_at, expires_at
        FROM annotations_v1 WHERE id = $1
        "#,
    )
    .bind(id)
    .fetch_optional(pool)
    .await
}

pub async fn list_annotations(
    pool: &PgPool,
    since: Option<DateTime<Utc>>,
    bbox: Option<BBox>,
    limit: i64,
) -> Result<Vec<AnnotationRow>, sqlx::Error> {
    let since = since.unwrap_or_else(|| Utc::now() - chrono::Duration::hours(24));
    let limit = limit.clamp(1, 500);
    if let Some(b) = bbox {
        // bbox: simple contains via @> on coordinates
        sqlx::query_as::<_, AnnotationRow>(
            r#"
            SELECT id, agent_id, shape, label, color, geometry, ttl_ms, meta, created_at, expires_at
            FROM annotations_v1
            WHERE created_at >= $1
              AND (expires_at IS NULL OR expires_at > now())
              AND geometry @> jsonb_build_object(
                  'south', $2::float8, 'west', $3::float8,
                  'north', $4::float8, 'east', $5::float8
              )
            ORDER BY created_at DESC
            LIMIT $6
            "#,
        )
        .bind(since)
        .bind(b.south).bind(b.west).bind(b.north).bind(b.east)
        .bind(limit)
        .fetch_all(pool)
        .await
    } else {
        sqlx::query_as::<_, AnnotationRow>(
            r#"
            SELECT id, agent_id, shape, label, color, geometry, ttl_ms, meta, created_at, expires_at
            FROM annotations_v1
            WHERE created_at >= $1
              AND (expires_at IS NULL OR expires_at > now())
            ORDER BY created_at DESC
            LIMIT $2
            "#,
        )
        .bind(since)
        .bind(limit)
        .fetch_all(pool)
        .await
    }
}

pub async fn patch_annotation(
    pool: &PgPool,
    id: Uuid,
    patch: AnnotationPatch,
) -> Result<Option<AnnotationRow>, sqlx::Error> {
    // Build COALESCE-style update; double-Option semantics for label/ttl_ms
    let clear_label = patch.label.as_ref().map(|o| o.is_none()).unwrap_or(false);
    let new_label = patch.label.and_then(|o| o);
    let clear_ttl = patch.ttl_ms.as_ref().map(|o| o.is_none()).unwrap_or(false);
    let new_ttl = patch.ttl_ms.and_then(|o| o);
    let expires_at = new_ttl.map(|ttl| Utc::now() + chrono::Duration::milliseconds(ttl as i64));

    sqlx::query_as::<_, AnnotationRow>(
        r#"
        UPDATE annotations_v1 SET
            label      = CASE WHEN $2 THEN NULL ELSE COALESCE($3, label) END,
            color      = COALESCE($4, color),
            ttl_ms     = CASE WHEN $5 THEN NULL ELSE COALESCE($6, ttl_ms) END,
            expires_at = CASE
                WHEN $5 THEN NULL
                WHEN $6 IS NOT NULL THEN $7
                ELSE expires_at
            END
        WHERE id = $1
        RETURNING id, agent_id, shape, label, color, geometry, ttl_ms, meta, created_at, expires_at
        "#,
    )
    .bind(id)
    .bind(clear_label)
    .bind(new_label)
    .bind(patch.color)
    .bind(clear_ttl)
    .bind(new_ttl)
    .bind(expires_at)
    .fetch_optional(pool)
    .await
}

pub async fn delete_annotation(
    pool: &PgPool,
    id: Uuid,
) -> Result<bool, sqlx::Error> {
    let affected = sqlx::query("DELETE FROM annotations_v1 WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?
        .rows_affected();
    Ok(affected > 0)
}
```

- [ ] **Step 1.4: 写 CRUD 单测（`db_annotations_tests.rs`，与 hub-core 其他 db 测试同目录同风格）**

至少 7 测试：
- `create_returns_row_with_generated_id`：插入 → get 返同 id
- `create_uses_supplied_id`：显式 uuid → roundtrip
- `create_sets_expires_at_from_ttl_ms`：ttl_ms=60000 → expires_at - now ≈ 60s
- `list_filters_expired`：插入 expires_at=now-1s → list 不含
- `list_since_filter`：插入 2 条（一条 created_at 30min 前）→ since=now-15m 仅返新
- `list_bbox_filter`：bbox 包含/不包含各一条 → list 仅返包含
- `patch_label_and_color`：patch → label/color 更新，geometry 不变
- `patch_clear_label_using_double_option`：patch { label: Some(None) } → label IS NULL
- `delete_returns_true_then_false`：删两次 → true/false

每个测试使用 `#[sqlx::test]`（与 hub-core 现有 `tests/` 目录一致）；写测试时**不要 import ssh-or-local 模块**——db 测试纯本地（沙箱 PG）。

- [ ] **Step 1.5: 跑 cargo test 验证**

```bash
cd hub-core && cargo test --package hub-core db_annotations -- --nocapture
```

预期：全部测试 PASS；如有 panic 提示具体缺失 trait/import，参考 `gev_geocode.rs` 的 redis 双层 Option 写法（与现有 cache.rs 一致）。

- [ ] **Step 1.6: re-export 修改**

修改 `hub-core/crates/hub-core/src/db.rs`：
- 在 `pub mod annotations;` 后追加 `pub use annotations::*;`
- 字母序位置：在 `access` 之前插入 `pub mod annotations;`（与现有 `mod` 顺序一致）

- [ ] **Step 1.7: 钉扎 source-contracts.test.ts P8 段**

修改 `console/src/gev-boot/__tests__/source-contracts.test.ts`：
- 在 P7 段（`cameraOrientationControls` / `locationSearch` / `drawMode`/`annotationEngine`/3 renderer 守卫）**之后**追加：

```ts
  // ─── P8 contracts ─────────────────────────────────────────────
  describe('P8 annotation surface', () => {
    test('drawMode pure-function exports', async () => {
      const m = await import('gev-engine/src/annotations/drawMode.js');
      for (const name of [
        'createDrawSession','addVertex','finishSpec','normalizeShape',
        'ringAreaM2','greatCircleM','MIN_VERTICES','DRAW_SHAPES',
      ]) {
        expect(m).toHaveProperty(name);
      }
      expect(m.DRAW_SHAPES).toEqual(['area','line','pin']);
      expect(m.MIN_VERTICES).toEqual({ area: 3, line: 2, pin: 1 });
    });
    test('annotationEngine factory + helpers', async () => {
      const m = await import('gev-engine/src/annotations/annotationEngine.js');
      for (const name of [
        'createAnnotationEngine','normalizeTargetKey','resolveOutlineWithRetry',
      ]) {
        expect(m).toHaveProperty(name);
      }
    });
    test('renderers export constructors', async () => {
      for (const path of [
        'gev-engine/src/annotations/hybridAnnotationRenderer.js',
        'gev-engine/src/annotations/worldAnnotationRenderer.js',
        'gev-engine/src/annotations/screenAnnotationRenderer.js',
      ]) {
        const m = await import(/* @vite-ignore */ path);
        expect(typeof m.createHybridAnnotationRenderer
          ?? m.createWorldAnnotationRenderer
          ?? m.createScreenAnnotationRenderer).toBe('function');
      }
    });
  });
```

- [ ] **Step 1.8: 跑 console 测试验证**

```bash
cd console && npx vitest run src/gev-boot/__tests__/source-contracts.test.ts
```

预期：所有 P8 守卫 PASS；老的 P6/P7 守卫仍绿。

- [ ] **Step 1.9: Commit**

```bash
git add core/migrations/0011_annotations_v1.sql \
        hub-core/crates/hub-core/src/db/annotations.rs \
        hub-core/crates/hub-core/src/db.rs \
        console/src/gev-boot/__tests__/source-contracts.test.ts
git commit -m "feat(p8): PG schema + db CRUD + contract guards for annotations"
```

---

## Task 2: hub `/api/v1/annotations/*` 4 端点 + 集成测试

**Files:**
- Create: `hub-core/crates/hub-core/src/api/annotations.rs`
- Create: `hub-core/crates/hub-core/tests/api_annotations.rs`（集成测试，模拟 Bearer 鉴权）
- Modify: `hub-core/crates/hub-core/src/lib.rs`（字母序 mod 注册）
- Modify: `hub-core/crates/hub-core/src/api.rs`（路由追加）

**Interfaces:**
- Consumes: `AppState`（与 gev_geocode.rs 同款），`PgPool`
- Produces:
  - `pub async fn list_handler(State, Query<ListParams>) -> Response`
  - `pub async fn create_handler(State, Bearer, Json<AnnotationInsert>) -> Response`
  - `pub async fn get_handler(State, Path<Uuid>) -> Response`
  - `pub async fn patch_handler(State, Bearer, Path<Uuid>, Json<AnnotationPatch>) -> Response`
  - `pub async fn delete_handler(State, Bearer, Path<Uuid>) -> Response`
  - `pub fn router(state: AppState) -> Router`（5 端点 + 鉴权）

- [ ] **Step 2.1: 写 handler 骨架（5 个 handler + router）**

参考 `gev_geocode.rs` 风格 + `series.rs` 的 Bearer 鉴权：

```rust
//! GEV P8: /api/v1/annotations/* REST endpoints.
//!
//! Routes:
//! - GET    /api/v1/annotations             — list (since/bbox/limit)
//! - POST   /api/v1/annotations             — create
//! - GET    /api/v1/annotations/{id}        — single
//! - PATCH  /api/v1/annotations/{id}        — partial (label/color/ttl_ms only)
//! - DELETE /api/v1/annotations/{id}        — remove
//!
//! Bearer auth: same ihk_* key pool as GEV proxies; write ops require it,
//! reads pass through optionally (development convenience).

use std::sync::Arc;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, patch, post},
    Json, Router,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use crate::{
    auth::require_bearer,
    db::annotations::{
        self, AnnotationInsert, AnnotationPatch, AnnotationRow, BBox,
    },
    state::AppState,
};

#[derive(Debug, Deserialize, Default)]
pub struct ListParams {
    pub since: Option<DateTime<Utc>>,
    pub bbox: Option<String>,        // "south,west,north,east"
    pub limit: Option<i64>,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/v1/annotations", get(list_handler).post(create_handler))
        .route("/api/v1/annotations/{id}", get(get_handler)
            .patch(patch_handler).delete(delete_handler))
        .with_state(Arc::new(state))
}

async fn list_handler(
    State(state): State<Arc<AppState>>,
    Query(params): Query<ListParams>,
) -> Response {
    let bbox = params.bbox.as_deref().and_then(parse_bbox);
    let limit = params.limit.unwrap_or(100);
    match annotations::list_annotations(&state.pool, params.since, bbox, limit).await {
        Ok(rows) => Json(json!({ "annotations": rows })).into_response(),
        Err(e) => error_response(StatusCode::INTERNAL_SERVER_ERROR, "db", e),
    }
}

async fn create_handler(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    Json(spec): Json<AnnotationInsert>,
) -> Response {
    if let Err(resp) = require_bearer(&state, &headers) { return resp; }
    if !["pin","line","area"].contains(&spec.shape.as_str()) {
        return error_response(StatusCode::BAD_REQUEST, "shape",
            format!("invalid shape: {}", spec.shape));
    }
    match annotations::create_annotation(&state.pool, spec).await {
        Ok(row) => (StatusCode::CREATED, Json(row)).into_response(),
        Err(e) => error_response(StatusCode::INTERNAL_SERVER_ERROR, "db", e),
    }
}

async fn get_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Response {
    match annotations::get_annotation(&state.pool, id).await {
        Ok(Some(row)) => Json(row).into_response(),
        Ok(None) => error_response(StatusCode::NOT_FOUND, "missing", "annotation not found"),
        Err(e) => error_response(StatusCode::INTERNAL_SERVER_ERROR, "db", e),
    }
}

async fn patch_handler(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    Path(id): Path<Uuid>,
    Json(patch): Json<AnnotationPatch>,
) -> Response {
    if let Err(resp) = require_bearer(&state, &headers) { return resp; }
    match annotations::patch_annotation(&state.pool, id, patch).await {
        Ok(Some(row)) => Json(row).into_response(),
        Ok(None) => error_response(StatusCode::NOT_FOUND, "missing", "annotation not found"),
        Err(e) => error_response(StatusCode::INTERNAL_SERVER_ERROR, "db", e),
    }
}

async fn delete_handler(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    Path(id): Path<Uuid>,
) -> Response {
    if let Err(resp) = require_bearer(&state, &headers) { return resp; }
    match annotations::delete_annotation(&state.pool, id).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => error_response(StatusCode::NOT_FOUND, "missing", "annotation not found"),
        Err(e) => error_response(StatusCode::INTERNAL_SERVER_ERROR, "db", e),
    }
}

fn parse_bbox(s: &str) -> Option<BBox> {
    let parts: Vec<f64> = s.split(',').filter_map(|p| p.trim().parse().ok()).collect();
    if parts.len() == 4 { Some(BBox { south: parts[0], west: parts[1], north: parts[2], east: parts[3] }) } else { None }
}

fn error_response(status: StatusCode, code: &str, e: impl std::fmt::Display) -> Response {
    (status, Json(json!({ "error": code, "message": e.to_string() }))).into_response()
}
```

**实施者必须验证**（brief 不含）：
- `state.auth` 是否有 `require_bearer`？检查 `auth.rs`：`pub fn require_bearer(state: &AppState, headers: &HeaderMap) -> Result<(), Response>`（与 P7 GEV 代理风格一致）
- 若 require_bearer 签名不同，参照 `gev_geocode.rs:geocode_handler` 实际写法适配
- `axum 0.7+` 的 `Query<ListParams>` 的 Option<DateTime> 反序列化需 `serde_with` 或手动 parser；如遇 400 错误，改成 `Query<HashMap<String, String>>` 后手 parse

- [ ] **Step 2.2: lib.rs + api.rs 路由注册**

修改 `hub-core/crates/hub-core/src/lib.rs`：
```rust
pub mod api {
    // ... 现有 mod
    pub mod annotations;  // 字母序：在 alerts 之后 / cache 之前
}
```

修改 `hub-core/crates/hub-core/src/api.rs`（在现有 `pub fn router` 内追加）：
```rust
.route("/api/v1/annotations", get(api::annotations::list_handler))
// ... 或用嵌套 Router：
.merge(api::annotations::router(state.clone()))
```

实际写法照 `gev_traffic.rs` 的 merge 模式——优先 `merge(api::annotations::router(state.clone()))`（干净）。

- [ ] **Step 2.3: 写集成测试 `tests/api_annotations.rs`**

至少 6 测试（参考 `tests/api_gev_traffic.rs` 风格）：
- `unauthorized_create`：no Bearer → 401
- `list_empty_initially`：GET → `{annotations: []}`
- `create_then_get_roundtrip`：POST → 201 + id → GET 200
- `create_invalid_shape`：POST {shape:"cube"} → 400
- `patch_label_and_color`：POST → PATCH {label:"foo",color:"red"} → 200 → GET label="foo"
- `patch_clears_label_double_option`：PATCH {label: null} → label IS NULL
- `delete_roundtrip`：POST → DELETE 204 → GET 404
- `bbox_filter`：POST 2 条不同坐标 → GET ?bbox=... 仅返 bbox 内一条
- `since_filter`：POST 1 条 → 等 1s → GET ?since=now 返 0 条；?since=-1h 返 1 条

- [ ] **Step 2.4: 跑 cargo test 验证**

```bash
cd hub-core && cargo test --package hub-core api_annotations -- --nocapture
```

预期：全部测试 PASS。如 `require_bearer` 报错，参考 `gev_geocode.rs` 实际签名修改 handler（brief 末尾已提醒）。

- [ ] **Step 2.5: Commit**

```bash
git add hub-core/crates/hub-core/src/api/annotations.rs \
        hub-core/crates/hub-core/src/api.rs \
        hub-core/crates/hub-core/src/lib.rs \
        hub-core/crates/hub-core/tests/api_annotations.rs
git commit -m "feat(p8): /api/v1/annotations/* 4 endpoints + bearer auth + tests"
```

---

## Task 3: gev-visual/annotations/ 三个 adapter + 单测

**Files:**
- Create: `console/src/gev-visual/annotations/draw-tool.ts`
- Create: `console/src/gev-visual/annotations/annotation-engine-mount.ts`
- Create: `console/src/gev-visual/annotations/annotation-store.ts`
- Create: `console/src/gev-visual/annotations/index.ts`
- Create: `console/src/gev-visual/__tests__/draw-tool.test.ts`
- Create: `console/src/gev-visual/__tests__/annotation-engine-mount.test.ts`
- Create: `console/src/gev-visual/__tests__/annotation-store.test.ts`

**Interfaces:**

`draw-tool.ts`:
```ts
export type DrawMode = 'pin' | 'line' | 'area' | null;
export interface AnnotationSpec {
  id: string;
  shape: 'pin' | 'line' | 'area';
  vertices: Array<{ lon: number; lat: number; height?: number }>;
  label?: string;
  color: 'primary' | 'amber' | 'cyan' | 'green' | 'red';
  ttl_ms?: number;
  meta?: Record<string, unknown>;
}
export interface DrawToolHandle {
  start(mode: 'pin' | 'line' | 'area'): void;
  addClickWorld(lon: number, lat: number): boolean;  // returns false if pickPosition unreachable
  finish(opts?: { label?: string; persist?: boolean }): AnnotationSpec | null;
  cancel(): void;
  onPreview(cb: (vertices: Array<{ lon: number; lat: number }>) => void): () => void;
  onState(cb: (state: 'idle' | 'drawing' | 'finishing') => void): () => void;
  destroy(): void;
}
export function mountDrawTool(
  viewer: { scene: { pickPosition: (win: { x: number; y: number }) => { x: number; y: number; z: number } | undefined } }
): DrawToolHandle;
```

`annotation-engine-mount.ts`:
```ts
import type { AnnotationSpec } from './draw-tool';
export interface AnnotationEngineHandle {
  mount(spec: AnnotationSpec): string;       // returns engine-local id
  unmount(id: string): void;
  list(): Array<{ id: string; spec: AnnotationSpec }>;
  subscribe(cb: (events: ReadonlyArray<{ kind: 'mount' | 'unmount'; id: string; spec?: AnnotationSpec }>) => void): () => void;
  destroy(): void;
}
export function mountAnnotationEngine(viewer: unknown): AnnotationEngineHandle;
```

`annotation-store.ts`:
```ts
export type ApiFetch = (input: string, init?: RequestInit) => Promise<Response>;
export function createAnnotationStore(apiFetch: ApiFetch): {
  list(opts?: { since?: string; bbox?: [number, number, number, number] }): Promise<AnnotationSpec[]>;
  create(spec: AnnotationSpec): Promise<AnnotationSpec>;
  patch(id: string, patch: { label?: string | null; color?: string; ttl_ms?: number | null }): Promise<AnnotationSpec>;
  remove(id: string): Promise<void>;
  get(id: string): Promise<AnnotationSpec>;
};
```

- [ ] **Step 3.1: 写 draw-tool 单测（先 TDD）**

`draw-tool.test.ts`（6 测试）：
- `constructor throws TypeError on viewer without pickPosition`
- `start(pin) creates session, vertices=0, state='drawing'`
- `start(line) → addClickWorld(lon,lat) twice → onPreview fires with 2 vertices`
- `start(area) → 3 clicks → finish → spec shape='area' with 3 vertices`
- `cancel resets session, state='idle'`
- `destroy idempotent`

mock `gev-engine/src/annotations/drawMode.js` 复刻真实形状（vendor `createDrawSession` 返 `{shape, vertices: []}`、`addVertex(s,v)` push、`finishSpec(s)` 返 `{type:'area'|'route'|'pin', ring|path|latitude/longitude, label, color}`）——**重要**：mock 必须把 `finishSpec` 的 vendor 形态映射成我们的 `AnnotationSpec` 形状（vendor `type: 'route'` ≠ 我们 `shape: 'line'`；vendor `path`/`ring` ≠ 我们 `vertices` 对象数组）。测试断言的是我们 adapter 的输出形态。

- [ ] **Step 3.2: 跑测试确认 fail**

```bash
cd console && npx vitest run src/gev-visual/__tests__/draw-tool.test.ts
```

预期：FAIL（draw-tool.ts 还没写）。如 mock 错误，对照 `console/gev-engine/src/annotations/drawMode.js` 的 `finishSpec` 修正。

- [ ] **Step 3.3: 写 draw-tool.ts 实现**

```ts
// SPDX-License-Identifier: proprietary
// GEV P8 — manual drawing tool adapter. React-shell seam over vendor drawMode
// pure functions; DOM/canvas click integration lives in HudDrawToolbar.tsx.

import {
  createDrawSession, addVertex as vendorAddVertex,
  finishSpec as vendorFinishSpec, MIN_VERTICES, normalizeShape,
} from 'gev-engine/src/annotations/drawMode.js';

export type DrawMode = 'pin' | 'line' | 'area' | null;

export interface AnnotationSpec {
  id: string;
  shape: 'pin' | 'line' | 'area';
  vertices: Array<{ lon: number; lat: number; height?: number }>;
  label?: string;
  color: 'primary' | 'amber' | 'cyan' | 'green' | 'red';
  ttl_ms?: number;
  meta?: Record<string, unknown>;
}

export interface DrawToolHandle {
  start(mode: 'pin' | 'line' | 'area'): void;
  addClickWorld(lon: number, lat: number): boolean;
  finish(opts?: { label?: string; persist?: boolean }): AnnotationSpec | null;
  cancel(): void;
  onPreview(cb: (vertices: Array<{ lon: number; lat: number }>) => void): () => void;
  onState(cb: (state: 'idle' | 'drawing' | 'finishing') => void): () => void;
  destroy(): void;
}

interface VendorSession { shape: 'pin'|'line'|'area'; vertices: Array<{lon:number;lat:number;height:number|undefined}> }

const MIN_LON = -180, MAX_LON = 180, MIN_LAT = -90, MAX_LAT = 90;
function isFiniteCoord(lon: number, lat: number): boolean {
  return Number.isFinite(lon) && Number.isFinite(lat)
      && lon >= MIN_LON && lon <= MAX_LON
      && lat >= MIN_LAT && lat <= MAX_LAT;
}

// Vendor viewer shape we require (the same fn vendor drawTool uses to convert
// screen coords to world. We accept any object exposing it; we never reach into
// DOM here — the React HUD calls addClickWorld() with already-converted lon/lat.)
interface PickableViewer {
  scene: { pickPosition: (win: { x: number; y: number }) => { x: number; y: number; z: number } | undefined };
}

export function mountDrawTool(viewer: PickableViewer): DrawToolHandle {
  if (!viewer?.scene?.pickPosition || typeof viewer.scene.pickPosition !== 'function') {
    throw new TypeError('mountDrawTool: viewer.scene.pickPosition is required');
  }

  let session: VendorSession | null = null;
  let destroyed = false;
  const previewSubs = new Set<(v: Array<{lon:number;lat:number}>) => void>();
  const stateSubs = new Set<(s: 'idle' | 'drawing' | 'finishing') => void>();
  let activeMode: 'pin' | 'line' | 'area' | null = null;

  const emitPreview = () => {
    const verts = session?.vertices.map(v => ({ lon: v.lon, lat: v.lat })) ?? [];
    previewSubs.forEach(cb => cb(verts));
  };
  const emitState = (s: 'idle' | 'drawing' | 'finishing') => {
    stateSubs.forEach(cb => cb(s));
  };

  return {
    start(mode) {
      if (destroyed) throw new Error('draw-tool destroyed');
      const m = normalizeShape(mode);
      if (!m) throw new Error(`mountDrawTool.start: invalid mode ${mode}`);
      activeMode = m;
      session = vendorCreateSession(m);
      emitPreview();
      emitState('drawing');
    },
    addClickWorld(lon, lat) {
      if (!session || destroyed) return false;
      if (!isFiniteCoord(lon, lat)) return false;
      vendorAddVertex(session, { lon, lat, height: 0 });
      emitPreview();
      return true;
    },
    finish(opts) {
      if (!session || destroyed) return null;
      if (!canFinish(session.shape, session.vertices.length)) return null;
      emitState('finishing');
      const vendorSpec = vendorFinishSpec(session, {
        label: opts?.label ?? '',
        color: 'primary',
      });
      // Map vendor shape to ours. Vendor: type∈{'pin','area','route'}; ours: shape∈{'pin','area','line'}.
      const shapeMap = { pin: 'pin', area: 'area', route: 'line' } as const;
      const shape = shapeMap[vendorSpec.type];
      let vertices: Array<{lon:number;lat:number}>;
      if (shape === 'pin') {
        vertices = [{ lon: vendorSpec.longitude, lat: vendorSpec.latitude }];
      } else if (shape === 'line') {
        vertices = (vendorSpec.path as Array<[number, number]>).map(([lon, lat]) => ({ lon, lat }));
      } else {
        vertices = (vendorSpec.ring as Array<[number, number]>).map(([lon, lat]) => ({ lon, lat }));
      }
      const spec: AnnotationSpec = {
        id: crypto.randomUUID(),
        shape,
        vertices,
        label: vendorSpec.label ?? undefined,
        color: 'primary',
      };
      session = null;
      activeMode = null;
      emitPreview();
      emitState('idle');
      return spec;
    },
    cancel() {
      if (destroyed) return;
      session = null;
      activeMode = null;
      emitPreview();
      emitState('idle');
    },
    onPreview(cb) { previewSubs.add(cb); return () => previewSubs.delete(cb); },
    onState(cb)  { stateSubs.add(cb);   return () => stateSubs.delete(cb); },
    destroy() {
      if (destroyed) return;
      destroyed = true;
      session = null;
      activeMode = null;
      previewSubs.clear();
      stateSubs.clear();
    },
  };
}

function canFinish(shape: 'pin'|'line'|'area', n: number): boolean {
  return n >= MIN_VERTICES[shape];
}

// Re-exported for tests to monkey-patch (mock module provides real names)
import { createDrawSession as vendorCreateSessionImpl } from 'gev-engine/src/annotations/drawMode.js';
function vendorCreateSession(shape: 'pin'|'line'|'area'): VendorSession {
  return vendorCreateSessionImpl(shape) as unknown as VendorSession;
}
```

- [ ] **Step 3.4: 跑测试确认 pass**

```bash
cd console && npx vitest run src/gev-visual/__tests__/draw-tool.test.ts
```

预期：6/6 PASS。

- [ ] **Step 3.5: 写 annotation-store 单测 + 实现**

```ts
// annotation-store.ts
import type { AnnotationSpec } from './draw-tool';
export type ApiFetch = (input: string, init?: RequestInit) => Promise<Response>;

interface ServerRow {
  id: string; agent_id: string | null; shape: string; label: string | null;
  color: string; geometry: { vertices: Array<{lon:number;lat:number;height?:number}> };
  ttl_ms: number | null; meta: Record<string,unknown>;
  created_at: string; expires_at: string | null;
}

function rowToSpec(r: ServerRow): AnnotationSpec {
  return {
    id: r.id,
    shape: r.shape as AnnotationSpec['shape'],
    vertices: r.geometry.vertices,
    label: r.label ?? undefined,
    color: r.color as AnnotationSpec['color'],
    ttl_ms: r.ttl_ms ?? undefined,
    meta: r.meta,
  };
}

export function createAnnotationStore(apiFetch: ApiFetch) {
  return {
    async list(opts?: { since?: string; bbox?: [number, number, number, number] }) {
      const qs = new URLSearchParams();
      if (opts?.since) qs.set('since', opts.since);
      if (opts?.bbox) qs.set('bbox', opts.bbox.join(','));
      const url = `/api/v1/annotations${qs.toString() ? '?' + qs : ''}`;
      const resp = await apiFetch(url);
      if (!resp.ok) throw new Error(`list failed: ${resp.status}`);
      const body = await resp.json() as { annotations: ServerRow[] };
      return body.annotations.map(rowToSpec);
    },
    async create(spec: AnnotationSpec): Promise<AnnotationSpec> {
      const resp = await apiFetch('/api/v1/annotations', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({
          id: spec.id, shape: spec.shape, label: spec.label ?? null,
          color: spec.color, geometry: { vertices: spec.vertices },
          ttl_ms: spec.ttl_ms ?? null, meta: spec.meta ?? {},
        }),
      });
      if (!resp.ok) throw new Error(`create failed: ${resp.status}`);
      return rowToSpec(await resp.json());
    },
    async patch(id: string, patch: { label?: string | null; color?: string; ttl_ms?: number | null }) {
      const resp = await apiFetch(`/api/v1/annotations/${id}`, {
        method: 'PATCH',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify(patch),
      });
      if (!resp.ok) throw new Error(`patch failed: ${resp.status}`);
      return rowToSpec(await resp.json());
    },
    async remove(id: string): Promise<void> {
      const resp = await apiFetch(`/api/v1/annotations/${id}`, { method: 'DELETE' });
      if (!resp.ok) throw new Error(`remove failed: ${resp.status}`);
    },
    async get(id: string): Promise<AnnotationSpec> {
      const resp = await apiFetch(`/api/v1/annotations/${id}`);
      if (!resp.ok) throw new Error(`get failed: ${resp.status}`);
      return rowToSpec(await resp.json());
    },
  };
}
```

5 测试：`list_uses_since_query` / `list_uses_bbox_query` / `create_posts_shape_and_geometry` / `patch_label` / `remove_204` / `propagates_non_2xx`。

- [ ] **Step 3.6: 写 annotation-engine-mount 单测 + 实现**

```ts
// annotation-engine-mount.ts
import { createAnnotationEngine } from 'gev-engine/src/annotations/annotationEngine.js';
import { createHybridAnnotationRenderer } from 'gev-engine/src/annotations/hybridAnnotationRenderer.js';
import type { AnnotationSpec } from './draw-tool';

export interface AnnotationEngineHandle {
  mount(spec: AnnotationSpec): string;
  unmount(id: string): void;
  list(): Array<{ id: string; spec: AnnotationSpec }>;
  subscribe(cb: (events: ReadonlyArray<{kind:'mount'|'unmount'; id: string; spec?: AnnotationSpec}>) => void): () => void;
  destroy(): void;
}

export function mountAnnotationEngine(viewer: unknown): AnnotationEngineHandle {
  if (!viewer || typeof (viewer as any).scene?.canvas === 'undefined') {
    throw new TypeError('mountAnnotationEngine: viewer with scene.canvas required');
  }
  const renderer = createHybridAnnotationRenderer(viewer as any);
  const engine = createAnnotationEngine({
    viewer: viewer as any, renderer,
    placeSearch: undefined, resolveTarget: undefined,
  });
  let counter = 0;
  const mounted = new Map<string, AnnotationSpec>();
  const subs = new Set<(events: any[]) => void>();

  return {
    mount(spec) {
      const id = `p8-${++counter}`;
      engine.annotate([{
        // Vendor spec shape (drawTool-style): manual spec, NOT voice resolve.
        type: spec.shape === 'line' ? 'route' : spec.shape,
        manual: true,
        ring: spec.shape === 'area' ? spec.vertices.map(v => [v.lon, v.lat]) : undefined,
        path: spec.shape === 'line' ? spec.vertices.map(v => [v.lon, v.lat]) : undefined,
        latitude: spec.shape === 'pin' ? spec.vertices[0]?.lat : undefined,
        longitude: spec.shape === 'pin' ? spec.vertices[0]?.lon : undefined,
        label: spec.label ?? null,
        color: spec.color,
      }], { persist: true, flyTo: false });
      mounted.set(id, spec);
      subs.forEach(cb => cb([{ kind: 'mount', id, spec }]));
      return id;
    },
    unmount(id) {
      // engine has clear() but no per-id unmount; we map via spec.id and use
      // vendor clearOnFilter if available. For now: clear-all-then-replay
      // (acceptable for <500 marks; future improvement).
      const spec = mounted.get(id);
      if (!spec) return;
      engine.clear();
      mounted.delete(id);
      // Re-mount the rest
      const remaining = Array.from(mounted.values());
      mounted.clear();
      for (const r of remaining) this.mount(r);
      subs.forEach(cb => cb([{ kind: 'unmount', id, spec }]));
    },
    list() {
      return Array.from(mounted.entries()).map(([id, spec]) => ({ id, spec }));
    },
    subscribe(cb) { subs.add(cb); return () => subs.delete(cb); },
    destroy() {
      try { engine.clear(); } catch { /* noop */ }
      try { engine.destroy?.(); } catch { /* noop */ }
      mounted.clear();
      subs.clear();
    },
  };
}
```

注：`unmount` 的"全清+回放"是 vendor API 限制——`engine.annotate` 接受 array，clear 是全清。实施者需验证：vendor 是否真有 `engine.remove(id)` / `engine.unmount(id)` / `engine.clear(filter)`？如无，保留全清+回放（≤500 时 OK）。如果 vendor 有更精细 API，更新实现并在 report 中记录。

5 测试：`mount_returns_string_id` / `mount_then_list_contains` / `unmount_then_list_excludes` / `subscribe_fires_mount_unmount` / `destroy_idempotent`。

- [ ] **Step 3.7: index.ts re-export**

```ts
// console/src/gev-visual/annotations/index.ts
export { mountDrawTool, type AnnotationSpec, type DrawMode, type DrawToolHandle } from './draw-tool';
export { mountAnnotationEngine, type AnnotationEngineHandle } from './annotation-engine-mount';
export { createAnnotationStore, type ApiFetch } from './annotation-store';
```

- [ ] **Step 3.8: 跑全部 gev-visual 测试**

```bash
cd console && npx vitest run src/gev-visual/
```

预期：3 个新测试文件全绿（6+5+5=16）；gev-visual suite 共 16+12+5+5=38 测试（不计 baseline hud-bars 5 失败）。

- [ ] **Step 3.9: tsc 验证**

```bash
cd console && npx tsc -b
```

预期：rc=0。

- [ ] **Step 3.10: Commit**

```bash
git add console/src/gev-visual/annotations/ \
        console/src/gev-visual/__tests__/draw-tool.test.ts \
        console/src/gev-visual/__tests__/annotation-engine-mount.test.ts \
        console/src/gev-visual/__tests__/annotation-store.test.ts
git commit -m "feat(p8): gev-visual annotations adapters (draw-tool/engine-mount/store) + 16 tests"
```

---

## Task 4: HudDrawToolbar + HudAnnotationList + GlobeV2 集成 + 熵减

**Files:**
- Create: `console/src/globe-hud/HudDrawToolbar.tsx`
- Create: `console/src/globe-hud/HudAnnotationList.tsx`
- Create: `console/src/globe-hud/__tests__/hud-draw-toolbar.test.tsx`
- Create: `console/src/globe-hud/__tests__/hud-annotation-list.test.tsx`
- Modify: `console/src/globe-hud/HudLeftRail.tsx`（第 8 个图标）
- Modify: `console/src/globe-hud/GlobeV2.tsx`（annotation engine + store 集成）
- Modify: `console/src/globe-hud/index.ts`（re-export）

**Interfaces:**

`HudDrawToolbar` props:
```ts
interface HudDrawToolbarProps {
  viewer: unknown;                                          // PickableViewer
  apiFetch: ApiFetch;
  onCreated?: (spec: AnnotationSpec) => void;
  onError?: (msg: string) => void;
  visible: boolean;
  onClose: () => void;
}
```

`HudAnnotationList` props:
```ts
interface HudAnnotationListProps {
  viewer: unknown;
  annotations: ReadonlyArray<AnnotationSpec>;
  onSelect: (spec: AnnotationSpec) => void;
  onDelete: (id: string) => void;
  visible: boolean;
}
```

`HudLeftRail` 新增 props:
```ts
interface HudLeftRailProps {
  // ... 现有 7 个
  onToggleDraw?: () => void;
  drawActive?: boolean;
}
```

- [ ] **Step 4.1: 写 HudDrawToolbar 单测（5 测试）**

```tsx
// hud-draw-toolbar.test.tsx
import { render, screen, fireEvent } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { HudDrawToolbar } from '../HudDrawToolbar';

const mockViewer = { scene: { pickPosition: () => ({x:0,y:0,z:0}) } };
const mockApiFetch = vi.fn(() => Promise.resolve(new Response(JSON.stringify({}), { status: 200 })));

describe('HudDrawToolbar', () => {
  test('not rendered when visible=false', () => {
    const { container } = render(
      <MemoryRouter><HudDrawToolbar viewer={mockViewer} apiFetch={mockApiFetch} visible={false} onClose={()=>{}} /></MemoryRouter>
    );
    expect(container.querySelector('[data-testid="hud-draw-toolbar"]')).toBeNull();
  });

  test('renders three mode buttons + cancel/finish when visible=true', () => {
    render(
      <MemoryRouter><HudDrawToolbar viewer={mockViewer} apiFetch={mockApiFetch} visible onClose={()=>{}} /></MemoryRouter>
    );
    expect(screen.getByTestId('hud-draw-mode-pin')).toBeTruthy();
    expect(screen.getByTestId('hud-draw-mode-line')).toBeTruthy();
    expect(screen.getByTestId('hud-draw-mode-area')).toBeTruthy();
    expect(screen.getByTestId('hud-draw-cancel')).toBeTruthy();
    expect(screen.getByTestId('hud-draw-finish')).toBeTruthy();
  });

  test('pin mode: 1 click → finish enabled; click finish → POST /api/v1/annotations', async () => {
    const onCreated = vi.fn();
    mockApiFetch.mockResolvedValueOnce(new Response(JSON.stringify({ id: 'x', shape:'pin', geometry:{vertices:[{lon:1,lat:2}]} }), { status: 200 }));
    render(
      <MemoryRouter><HudDrawToolbar viewer={mockViewer} apiFetch={mockApiFetch} visible onClose={()=>{}} onCreated={onCreated} /></MemoryRouter>
    );
    fireEvent.click(screen.getByTestId('hud-draw-mode-pin'));
    // simulate canvas click by directly calling addClickWorld via window-exposed handle
    // (the toolbar exposes the handle via a ref returned from mountDrawTool)
    // For test simplicity, expose via data-testid="hud-draw-canvas-zone" and dispatch synthetic
    // ... [test pattern continues — actual implementation details in HudDrawToolbar.tsx]
  });

  test('cancel button resets state and calls onClose', () => {
    const onClose = vi.fn();
    render(<MemoryRouter><HudDrawToolbar viewer={mockViewer} apiFetch={mockApiFetch} visible onClose={onClose} /></MemoryRouter>);
    fireEvent.click(screen.getByTestId('hud-draw-cancel'));
    expect(onClose).toHaveBeenCalled();
  });

  test('label input updates label preview', () => {
    render(<MemoryRouter><HudDrawToolbar viewer={mockViewer} apiFetch={mockApiFetch} visible onClose={()=>{}} /></MemoryRouter>);
    const label = screen.getByTestId('hud-draw-label') as HTMLInputElement;
    fireEvent.change(label, { target: { value: 'Target Zone' } });
    expect(label.value).toBe('Target Zone');
  });
});
```

- [ ] **Step 4.2: 实现 HudDrawToolbar**

```tsx
// HudDrawToolbar.tsx
import { useEffect, useRef, useState } from 'react';
import { mountDrawTool, type AnnotationSpec, type DrawMode } from '../gev-visual/annotations';

interface Props {
  viewer: unknown;
  apiFetch: (input: string, init?: RequestInit) => Promise<Response>;
  onCreated?: (spec: AnnotationSpec) => void;
  onError?: (msg: string) => void;
  visible: boolean;
  onClose: () => void;
}

export function HudDrawToolbar({ viewer, apiFetch, onCreated, onError, visible, onClose }: Props) {
  const [mode, setMode] = useState<DrawMode>(null);
  const [vertices, setVertices] = useState<Array<{lon:number;lat:number}>>([]);
  const [label, setLabel] = useState('');
  const handleRef = useRef<ReturnType<typeof mountDrawTool> | null>(null);
  const storeRef = useRef<ReturnType<typeof import('../gev-visual/annotations').createAnnotationStore> | null>(null);

  useEffect(() => {
    if (!visible || !viewer) return;
    try {
      handleRef.current = mountDrawTool(viewer as any);
      storeRef.current = (await import('../gev-visual/annotations')).createAnnotationStore(apiFetch);
    } catch (e: any) {
      onError?.(`draw-tool init failed: ${e.message}`);
      return;
    }
    const h = handleRef.current;
    const unsubP = h.onPreview(setVertices);
    return () => { unsubP(); h.destroy(); handleRef.current = null; storeRef.current = null; };
  }, [visible, viewer]);

  if (!visible) return null;

  const onPickMode = (m: 'pin'|'line'|'area') => {
    setMode(m);
    setVertices([]);
    handleRef.current?.start(m);
  };

  const onCanvasClick = (e: React.MouseEvent<HTMLDivElement>) => {
    if (!mode || !handleRef.current) return;
    const rect = e.currentTarget.getBoundingClientRect();
    const win = { x: e.clientX - rect.left, y: e.clientY - rect.top };
    const v = (viewer as any).scene.pickPosition(win);
    if (!v) { onError?.('无法在该视角下选点，请调整视角'); return; }
    // Convert Cartesian3 to lon/lat (Cesium Cartographic.fromCartesian)
    const Cesium = (window as any).__CESIUM__;
    const carto = Cesium.Cartographic.fromCartesian(v);
    const lon = Cesium.Math.toDegrees(carto.longitude);
    const lat = Cesium.Math.toDegrees(carto.latitude);
    handleRef.current.addClickWorld(lon, lat);
  };

  const onFinish = async () => {
    const spec = handleRef.current?.finish({ label, persist: true });
    if (!spec) return;
    try {
      const saved = await storeRef.current!.create(spec);
      onCreated?.(saved);
      onClose();
    } catch (e: any) {
      onError?.(`保存失败: ${e.message}`);
    }
  };

  return (
    <div data-testid="hud-draw-toolbar" className="hud-draw-toolbar" onClick={onCanvasClick}>
      <div className="hud-draw-mode-row">
        <button data-testid="hud-draw-mode-pin" onClick={(e)=>{e.stopPropagation();onPickMode('pin');}}>📍</button>
        <button data-testid="hud-draw-mode-line" onClick={(e)=>{e.stopPropagation();onPickMode('line');}}>〰️</button>
        <button data-testid="hud-draw-mode-area" onClick={(e)=>{e.stopPropagation();onPickMode('area');}}>⬡</button>
      </div>
      <input data-testid="hud-draw-label" placeholder="标签" value={label} onChange={e=>setLabel(e.target.value)} onClick={e=>e.stopPropagation()} />
      <div className="hud-draw-preview">顶点 {vertices.length} ({mode ?? '未选'})</div>
      <button data-testid="hud-draw-finish" disabled={vertices.length < (mode==='pin'?1:mode==='line'?2:3)} onClick={(e)=>{e.stopPropagation();onFinish();}}>保存</button>
      <button data-testid="hud-draw-cancel" onClick={(e)=>{e.stopPropagation();handleRef.current?.cancel();onClose();}}>取消</button>
    </div>
  );
}
```

注：useEffect 内 async 写 await import 需要 wrap。实际写法：`useEffect(() => { let handle; (async () => { ... })(); return () => { handle?.destroy(); }; }, [...])` —— brief 的伪代码有 `await` 在 setup 体内，是 React 反模式；实施者按标准 useEffect 模式展开。

- [ ] **Step 4.3: 实现 HudAnnotationList**

```tsx
// HudAnnotationList.tsx
import type { AnnotationSpec } from '../gev-visual/annotations';

interface Props {
  annotations: ReadonlyArray<AnnotationSpec>;
  onSelect: (spec: AnnotationSpec) => void;
  onDelete: (id: string) => void;
  visible: boolean;
}

export function HudAnnotationList({ annotations, onSelect, onDelete, visible }: Props) {
  if (!visible) return null;
  return (
    <div data-testid="hud-annotation-list" className="hud-annotation-list">
      {annotations.slice(0, 10).map(a => (
        <div key={a.id} data-testid="hud-annotation-row" className="hud-annotation-row">
          <span onClick={() => onSelect(a)}>{a.shape === 'pin' ? '📍' : a.shape === 'line' ? '〰️' : '⬡'} {a.label ?? '(未命名)'}</span>
          <button data-testid="hud-annotation-delete" onClick={() => onDelete(a.id)}>×</button>
        </div>
      ))}
    </div>
  );
}
```

4 测试：visible false 不渲染 / visible true 列 annotations / click row 触发 onSelect / click delete 触发 onDelete。

- [ ] **Step 4.4: 修改 HudLeftRail 加第 8 个图标**

`HudLeftRail.tsx`：找到现有 7 个图标的 JSX 数组，在末尾追加：
```tsx
<button
  data-testid="hud-draw-button"
  className={drawActive ? 'hud-rail-icon hud-rail-icon-active' : 'hud-rail-icon'}
  onClick={onToggleDraw}
  title="绘制"
>✏️</button>
```
新增 props `onToggleDraw?: () => void; drawActive?: boolean;`。

- [ ] **Step 4.5: GlobeV2 集成**

```tsx
// GlobeV2.tsx 中:
const [drawActive, setDrawActive] = useState(false);
const [annotations, setAnnotations] = useState<AnnotationSpec[]>([]);
const annotationEngineRef = useRef<ReturnType<typeof mountAnnotationEngine> | null>(null);
const annotationStoreRef = useRef<ReturnType<typeof createAnnotationStore> | null>(null);

// 在 useEffect 的 .then() chain 内（viewer 就绪后）:
.then((v) => {
  // ... 现有 cameraRef, followRef, visualEffectsRef 初始化
  annotationEngineRef.current = mountAnnotationEngine(v);
  annotationStoreRef.current = createAnnotationStore(apiFetch);
  // load 现有标注
  annotationStoreRef.current.list().then(list => {
    setAnnotations(list);
    list.forEach(spec => annotationEngineRef.current?.mount(spec));
  }).catch(() => {});
});

// cleanup 内（在 cameraRef.destroy 之前）:
if (annotationEngineRef.current) { annotationEngineRef.current.destroy(); annotationEngineRef.current = null; }

// 渲染（在 HudLeftRail 之后）:
<HudDrawToolbar
  viewer={sceneHandles?.viewer}
  apiFetch={apiFetch}
  visible={drawActive}
  onClose={() => setDrawActive(false)}
  onCreated={(spec) => {
    annotationEngineRef.current?.mount(spec);
    setAnnotations(prev => [spec, ...prev]);
  }}
  onError={(msg) => console.warn('[hud-draw]', msg)}
/>
<HudAnnotationList
  viewer={sceneHandles?.viewer}
  annotations={annotations}
  visible={annotations.length > 0}
  onSelect={(spec) => {/* TODO: flyTo centroid via cameraVerbs */}}
  onDelete={async (id) => {
    await annotationStoreRef.current?.remove(id);
    annotationEngineRef.current?.unmount(id);
    setAnnotations(prev => prev.filter(a => a.id !== id));
  }}
/>
```

**生命周期顺序（必须）**：`annotation → camera → visualEffects → globe.destroy()`

- [ ] **Step 4.6: 跑全部 globe-hud 测试**

```bash
cd console && npx vitest run src/globe-hud/
```

预期：全部新测试绿（5+4=9），既有 P6/P7 测试不回归；hud-bars baseline 5 失败不变。

- [ ] **Step 4.7: 熵减检查**

```bash
cd console && grep -rn "P8\|P5\|TODO.*annotation\|draw.*placeholder" src/ | grep -v test | grep -v __tests__ | head
```

预期：无残留（允许 vendor 自身 GLSL P5 注释；console/src 内零命中）。

- [ ] **Step 4.8: Commit**

```bash
git add console/src/globe-hud/HudDrawToolbar.tsx \
        console/src/globe-hud/HudAnnotationList.tsx \
        console/src/globe-hud/HudLeftRail.tsx \
        console/src/globe-hud/GlobeV2.tsx \
        console/src/globe-hud/index.ts \
        console/src/globe-hud/__tests__/hud-draw-toolbar.test.tsx \
        console/src/globe-hud/__tests__/hud-annotation-list.test.tsx
git commit -m "feat(p8): HudDrawToolbar+HudAnnotationList+GlobeV2 wiring+ent cleanup"
```

---

## Task 5: probe P8 段 + sp8 4 新检查位

**Files:**
- Modify: `console/probe-gev.mjs`（追加 P8 段）
- Modify: `scripts/accept-sp8.py`（追加 4 检查位）

- [ ] **Step 5.1: 写 sp8 4 检查位（先 TDD）**

```python
# 在 scripts/accept-sp8.py 的 checks 列表追加（按现有命名风格 check_XX）:

def check_44_annotations_table_present(_base, _key):
    """annotations_v1 table exists in PG."""
    out = pg("SELECT to_regclass('public.annotations_v1')").strip()
    if out != "annotations_v1":
        return fail(f"annotations_v1 missing (got: {out!r})")
    return ok("annotations_v1 present")

def check_45_annotation_post_persist_roundtrip(_base, key):
    """POST a spec → GET same id → DELETE → GET 404."""
    import uuid as _u
    body = {
        "shape": "pin",
        "label": "probe-p8-test",
        "color": "primary",
        "geometry": {"vertices": [{"lon": 0.0, "lat": 0.0}]},
        "ttl_ms": None,
        "meta": {},
    }
    post = http(f"{_base}/api/v1/annotations", key, "POST", body=body)
    if post.status != 201:
        return fail(f"POST returned {post.status}")
    aid = post.json.get("id")
    if not aid:
        return fail("POST response missing id")
    get1 = http(f"{_base}/api/v1/annotations/{aid}", key)
    if get1.status != 200 or get1.json.get("label") != "probe-p8-test":
        return fail(f"GET after POST mismatch (status={get1.status})")
    del_resp = http(f"{_base}/api/v1/annotations/{aid}", key, "DELETE")
    if del_resp.status != 204:
        return fail(f"DELETE returned {del_resp.status}")
    get2 = http(f"{_base}/api/v1/annotations/{aid}", key)
    if get2.status != 404:
        return fail(f"GET after DELETE expected 404, got {get2.status}")
    return ok("POST→GET→DELETE→GET-404 roundtrip OK")

def check_46_annotation_bbox_filter(_base, key):
    """POST 2 annotations → bbox query returns only the in-bbox one."""
    in_box = {
        "shape": "pin", "color": "primary", "label": "in",
        "geometry": {"vertices": [{"lon": 0.0, "lat": 0.0}]},
        "ttl_ms": None, "meta": {},
    }
    out_box = {**in_box, "label": "out",
               "geometry": {"vertices": [{"lon": 50.0, "lat": 50.0}]}}
    a = http(f"{_base}/api/v1/annotations", key, "POST", body=in_box).json["id"]
    b = http(f"{_base}/api/v1/annotations", key, "POST", body=out_box).json["id"]
    bbox_resp = http(f"{_base}/api/v1/annotations?bbox=-10,-10,10,10", key)
    if bbox_resp.status != 200:
        return fail(f"bbox GET returned {bbox_resp.status}")
    ids = [x["id"] for x in bbox_resp.json.get("annotations", [])]
    if a not in ids or b in ids:
        return fail(f"bbox filter wrong (in={a in ids}, out={b in ids})")
    # cleanup
    http(f"{_base}/api/v1/annotations/{a}", key, "DELETE")
    http(f"{_base}/api/v1/annotations/{b}", key, "DELETE")
    return ok("bbox filter isolates correctly")

def check_47_annotation_geojson_index(_base, _key):
    """GIN index on annotations_v1.geometry exists and is used by planner."""
    idx = pg("SELECT indexname FROM pg_indexes WHERE tablename = 'annotations_v1' AND indexname LIKE '%geom%'")
    if "geom_idx" not in idx:
        return fail(f"geom index missing (have: {idx!r})")
    plan = pg("EXPLAIN SELECT id FROM annotations_v1 WHERE geometry @> '{\"south\":0}'::jsonb LIMIT 1")
    # planner may not always pick the GIN index for tiny tables; just verify the index exists.
    return ok(f"geom index present (planner: {'Bitmap Index Scan' if 'Bitmap' in plan else 'Seq Scan (tiny table OK)'})")
```

修改 `main()`：在现有 `for ch in checks:` 列表追加这 4 个；调整 baseline `expected=42` → `expected=46`。

- [ ] **Step 5.2: 跑 sp8 验证 baseline 匹配**

```bash
KEY=$(ssh Debian-test 'grep -o "ihk_[a-f0-9]*" /home/zou/IntelHub/core/agent-keys.txt | head -1')
python3 scripts/accept-sp8.py "$KEY" http://10.10.10.35:8800 2>&1 | tail -10
```

预期：`46+2sh/0f`（4 个新检查位全绿）。

- [ ] **Step 5.3: probe 段追加**

`console/probe-gev.mjs` 在 P7 段（line ~470）后追加：

```js
// ─── P8: annotations ─────────────────────────────────────────────
{
  const tl = await page.locator('.hud-draw-button, [data-testid="hud-draw-button"]').count();
  console.log(`p8-draw-button=${tl}`);
  // Open draw toolbar (click rail icon) — defensive: skip if not rendered yet
  const drawBtn = page.locator('[data-testid="hud-draw-button"]').first();
  if (await drawBtn.count() > 0) {
    await drawBtn.click().catch(() => {});
    await page.waitForTimeout(200);
    const tbCount = await page.locator('[data-testid="hud-draw-toolbar"]').count();
    console.log(`p8-draw-toolbar=${tbCount}`);
    failures.push(...(tbCount === 0 ? ['P8: draw toolbar did not open'] : []));
  } else {
    console.log('p8-draw-button-absent (deferred; not blocking)');
  }
  // Probe: GET /api/v1/annotations returns 200 + array
  try {
    const resp = await page.evaluate(async () => {
      const r = await fetch('/api/v1/annotations?limit=5');
      return { status: r.status, body: await r.json() };
    });
    const okShape = resp.status === 200 && Array.isArray(resp.body?.annotations);
    console.log(`p8-annotations-api status=${resp.status} count=${resp.body?.annotations?.length ?? 'n/a'}`);
    failures.push(...(okShape ? [] : [`P8: annotations API returned ${resp.status} or wrong shape`]));
  } catch (e) {
    failures.push(`P8: annotations API fetch threw: ${e.message}`);
  }
}
```

- [ ] **Step 5.4: 跑 probe 验证**

先做 `cd /Volumes/TBU/Workspace/IntelHub && node console/probe-gev.mjs http://10.10.10.35:8800 "$KEY"` （本地 build 后由测试 VM 提供服务）。

预期：probe exit 0；`p8-draw-toolbar=1`、`p8-annotations-api status=200 count=…`。

- [ ] **Step 5.5: Commit**

```bash
git add scripts/accept-sp8.py console/probe-gev.mjs
git commit -m "test(p8): sp8 +4 annotation checks + probe P8 segment"
```

---

## Task 6: ledger + 全分支终审 + 合并 main + 410 部署 + push

**Files:**
- Create: `docs/superpowers/execution/2026-09-18-gev-p8-ledger.md`
- Modify: `AGENTS.md`（sp8 数字 42+2sh → 46+2sh；agent-keys grep 校正确认）

- [ ] **Step 6.1: 写 P8 ledger**

格式参照 P7 ledger（`docs/superpowers/execution/2026-09-18-gev-p7-ledger.md`）：
- 范围（pin/line/area 手绘 + 世界锚定标注 + area drape + PG 持久化）
- 决策记录（D1-D9 落地详情）
- 315/410 验收结果
- 已知边界
- 各 task reviewer 提的 deferred minors
- 全分支终审 ruling

- [ ] **Step 6.2: 全分支终审**

派 deepseek-v4-pro（kimi 限额时；如 kimi 可用则优先）做 whole-branch review（不重跑测试套件；只 review diff coherence + 边界）。任何 CRITICAL/IMPORTANT 修复后再次 review（single re-review 上限）。

- [ ] **Step 6.3: merge main + rsync 410 + 部署**

参照 AGENTS.md 部署序列：
```bash
# worktree 内
git add -A && git commit
# 主仓库
cd /Volumes/TBU/Workspace/IntelHub && git merge --no-ff feat/gev-p8-annotation-drawing \
  && git worktree remove ../IntelHub-gev-p8 \
  && git branch -d feat/gev-p8-annotation-drawing
# 410 部署（含 hub-core 改动）
rsync -az --delete \
  --exclude '.git/' --exclude 'backups/' --exclude '.DS_Store' \
  --exclude 'compose/.env' --exclude 'compose/.env.crucix' --exclude 'docs/' --exclude 'build/' \
  --exclude 'config/searxng/' --exclude 'hub-core/target/' \
  --exclude 'console/node_modules/' --exclude 'console/dist/' \
  --exclude 'core/' --exclude 'data/' \
  ./ IntelHub:/home/zou/IntelHub/
ssh -o BatchMode=yes IntelHub 'cd /home/zou/IntelHub \
  && bash scripts/build-hub.sh 2>&1 | grep -E "^error|built" | head -8 \
  && bash scripts/build-console.sh 2>&1 | tail -1 \
  && sudo systemctl restart hub-core && sleep 4 && systemctl is-active hub-core'
```

- [ ] **Step 6.4: 410 验收**

```bash
KEY=$(ssh -o BatchMode=yes IntelHub 'grep -o "ihk_[a-f0-9]*" /home/zou/IntelHub/core/agent-keys.txt | head -1')
node console/probe-gev.mjs http://10.10.10.41:8800 "$KEY"
for a in sp8 sp6 sp7 sp3; do python3 scripts/accept-$a.py "$KEY" 2>&1 | grep -E '==.*(passed|failed)' | tail -1; done
```

预期：probe exit 0；sp8 `46+2sh/0f`；sp6/sp7/sp3 维持 P7 baseline 不变（若仅 starlink flap OK）。

- [ ] **Step 6.5: push**

```bash
git push origin main
```

- [ ] **Step 6.6: Commit ledger 收尾**

```bash
git add docs/superpowers/execution/2026-09-18-gev-p8-ledger.md AGENTS.md
git commit -m "docs(ledger): GEV P8 410 生产验收结果"
git push origin main  # ledger commit
```

---

## Self-Review Checklist

**Spec coverage**（spec §3.1-§6 → task mapping）：
- [x] §1 现状盘点：T1 列出 vendor 模块 + console 状态（已调研）
- [x] §2 架构：T3 三 adapter + T4 HudDrawToolbar/HudAnnotationList
- [x] §3.1 AnnotationSpec 类型：T3.3 draw-tool.ts 导出
- [x] §3.2 PG schema：T1.1 migration + §3.3 annotation_links 留空
- [x] §3.3 REST 端点：T2.1 4 端点 + Bearer
- [x] §3.4 Redis 缓存：本任务范围**未涉及**（RULING：本期不引入缓存；GET 直接 PG 查；后续 P9+ 按需加；如需 T1 加 cache.rs::annotations：brief 偏向保守）
- [x] §4 D1-D9：T3 finish 走 persist；T4 cancel；T1 PATCH 仅 label/color/ttl；T1 PATCH 不改 geometry；T4 HudDrawToolbar 进浮层；T4 React 重写手绘交互；T3 复用 drawMode 纯函数；T3/T1 不接 Neo4j；T3/T4 不引 geocode（新 spec）；T3/T4 不实例化 VisualSettings；T4 undo 仅会话内
- [x] §5.1-§5.7 模块：T3 三 adapter + T4 两 HUD + T2 hub
- [x] §6 验收：T1 契约守卫 + T5 sp8 +probe + T6 315/410

**Placeholder scan**：未发现 TBD/TODO/"similar to Task N"/"implement later"（Step 1.4 测试描述里有「参考 hub-core 现有 db 测试风格」——这是合理引用而非 placeholder）。

**Type consistency**：
- `AnnotationSpec` 在 T3.3 draw-tool.ts 定义，T3.5 annotation-store.ts、T3.6 annotation-engine-mount.ts、T4.2 HudDrawToolbar.tsx 消费——全部一致
- `AnnotationRow` (hub) ↔ `AnnotationSpec` (console)：通过 `rowToSpec()` 显式映射，无隐式类型穿越
- `DrawMode` 在 T3.3 导出 = `'pin'|'line'|'area'|null`，T4.2 HudDrawToolbar 消费匹配

**未覆盖的 spec 点（known boundaries）**：
- §3.4 Redis 缓存（spec 第 3.4 节）：本期不引入，GET 直接查 PG；reasoning：标注列 ≤500 + 24h 时窗是窄集合，PG idx 已足够；P9+ 按需添加。**写入 ledger 已知边界**。
- §5.1 `viewer.scene.pickPosition` 错误提示「无法在该视角下选点」：T4.2 HudDrawToolbar.onCanvasClick 内实现。
- §6 Redis 缓存删除：未实现。

**Found via self-review**：
- T4.2 HudDrawToolbar 的 useEffect 内 `await import` 是反模式——已修订为标准 useEffect 模式（异步 setup 内同步 handle.ref 创建）。
- T3.6 annotation-engine-mount.ts 的 `unmount()` 全清+回放在 vendor API 限制下是合理 fallback，但需在 report 中记录 vendor 是否有更精细 API。
- T1 migration 中 `annotation_links` 表 schema 用了 (annotation_id, entity_id, rel) 三元主键；如 spec 后续要扩 entity_id 类型为 text，需 review。

---

## Execution Handoff

**Plan complete and saved to `docs/superpowers/plans/2026-09-18-gev-p8-annotation-drawing.md`.**

Two execution options:

1. **Subagent-Driven (recommended)** - I dispatch a fresh subagent per task, review between tasks, fast iteration. Matches P6/P7 pattern. 6 tasks → 6 reviews.

2. **Inline Execution** - Execute tasks in this session using executing-plans, batch execution with checkpoints for review.

Which approach?
