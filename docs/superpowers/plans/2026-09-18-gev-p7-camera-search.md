# GEV P7 相机姿态 + 地点搜索飞行 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 选中航班/卫星后一键相机跟随 + 斜视 35°/俯视 -89° 两档切换 + 回北；HudTopBar 占位搜索框实装为地点搜索飞行（hub 新建 `/api/v1/gev/geocode` 代理 photon）。

**Architecture:** 渲染核直引 + React 壳（总纲方案 1）。相机核直引 vendor `cameraOrientationControls.js` 纯函数族；追踪走图层 `trackById` API；搜索直引 `LocationSearch` 类 + `searchAndFlyTo`，placeSearch 自建适配器调 hub 代理。hub 新增 `gev_geocode.rs`（照 gev_tomtom_flow 模式）。

**Tech Stack:** Rust(axum/reqwest/redis/sha2) + React 18 + TypeScript + vitest(jsdom) + Cesium + playwright(probe)。

**Spec:** `docs/superpowers/specs/2026-09-18-gev-p7-camera-search-design.md`（红标与决策以此为准）

## Global Constraints

- **vendor 纯净只读**：`console/gev-engine/` 零修改。
- **严禁 scopeMask/圆形遮罩/周边压黑**：全景渲染；相机核与 scopeMask 零耦合已验证，保持。
- **严禁实例化 VisualSettings / LocationNavigation / bindCameraOrientationControls**（DOM/shell 耦合）；只用纯函数族 + `LocationSearch` 类。
- **`searchAndFlyTo` 必须显式传 `options.features = disabledFeatures()` 和 `options.recoverNearView = async () => null`**——默认参数引用 shell 单例 applicationServices（POST 不存在的 /api/overpass）和 annotationResolver 的 Overpass 恢复。
- **Redis 缓存读必须 `Option<Option<Vec<u8>>>` 双层**（redis-rs Nil→vec![] 血泪）。
- **mock 复刻真实构造器契约**（lenient-mock 教训）。
- **俯视档 = -89°**（STRAIGHT_DOWN_PITCH，非 -90）；文案写「俯视」不写「90°」。
- **性能纪律**：`pickViewTarget`/`readCameraTargetFrame` 是 4-12ms 深度缓冲读——只在用户手势后调用，严禁每帧轮询；朝向读数用免费的 `camera.heading`。
- **跟随范围**：仅 kind=flight（flights 层 trackById）+ kind=satellite（satellites 层 trackById）。vessels 无 trackById，不做。
- localStorage/命名空间惯例 `intelhub.*`；testid 逐字（probe 依赖）。
- 部署序列：worktree 隔离 → 315 全绿 → merge → 410 → push。**本期含 hub-core 改动，build-hub.sh 必跑**（315 和 410 都是）。
- 验收基线（315 实测）：sp8 39+2sh/0f、sp6 36+5sh/≤1f（starlink 惩罚箱 flap 已知）、sp7 16+11sh/0f、sp3 19/0f。sp6 starlink 502 若出现不阻塞（P3 值守项）。

## 关键事实（探索验证过，executor 不必重查）

- 相机核（cameraOrientationControls.js）：`toggleCameraTilt(viewer) → {tilted,pitch}|false`、`resetCameraNorth(viewer) → boolean`、`frameIsTilted(frame)`、`createCameraOrientationAnimator(viewer,{now?,duration=650})`。模块**不启动/停止追踪**——只读 `viewer.trackedEntity`。
- 追踪 API：flights `layers/flights/queries.js` `trackById(icao24,{origin='programmatic'})→boolean`、`stopTracking({origin})→true`、`getTrackedInfo()→{icao24,...}|null`；satellites `layers/satellites/controls.js` `trackById(noradId:number)`、`stopTracking()`、`getTrackedInfo()`。console 触达路径：`getComponents().data.dataManager.layers.get('flights').module`（`layers` 是公开 Map 属性；`getAll()` 只有元数据，别用）。
- 选中态（context-bridge.ts）：`useGlobeSelection() → { kind, data }`；kind='flight' 时 `data.id`=icao24 字符串；kind='satellite' 时 `data.noradId`（字符串，trackById 要 Number()）。KIND_BY_LAYER_ID 只映射 flights/satellites/earthquakes/ais-live-vessels/military-installations/cctv。
- LocationSearch（ui/locationSearch.js:5-16）构造：`{ input, begin, isCurrent, beforeFly, search, onStart, onResult, onMissing, onError, onSettled }`；`run(query)` async；`getState()`；`subscribe(listener)`；`destroy()`。DOM 耦合仅 `input.classList.add/remove('searching')` + `input.blur()`。
- `searchAndFlyTo(viewer, query, options)`（src/locations.js:778）：`options.placeSearch.geocode(query, {bias, signal})` 返回 `{ place: {lat,lng,label,types,viewport,exact?} | null, answered: boolean }`；**viewport 直接消费 `{southwest:{lat,lng}, northeast:{lat,lng}}`**（locations.js:795）；`options.recoverNearView` 默认是 annotationResolver 的 Overpass 恢复（必须覆盖为 `async () => null`）；飞行模式引擎自管（geocodeNavigationMode：country/administrative→region-overview，locality→city-overview，其余→精确地物 flyToLandmark）。
- HudTopBar 占位搜索框：console/src/globe-hud/HudTopBar.tsx:98-108，`data-testid="hud-search-p5"`，样式 `.hud-bar-search`（hud.css:459 + :disabled 规则 :471）。
- HudDetailPanel 当前零 props（GlobeV2.tsx:148 `right={<HudDetailPanel />}`）；动作区 `.hud-detail-actions` + `.hud-detail-link` 样式族现成（FlightBody L62-66）。
- hub 代理模板：gev_traffic.rs `gev_tomtom_flow`（:135）+ `traffic_http()`（:49）+ `gev_err`（:60）+ `REDIS_BUDGET_MS=2000`（:44）+ 双层 Option 注释（:148-153）。路由注册 api.rs:63-84（`.route("/api/v1/gev/...", get(crate::gev_geocode::gev_geocode))`）；模块声明 lib.rs:16-20（`pub mod gev_geocode;`）。sha2 依赖现成（crates/hub-core/Cargo.toml:23，用法参照 cache.rs:21）。
- `redis_timed` 签名（state.rs:34）：`pub async fn redis_timed<T>(&self, cmd: redis::Cmd, ms: u64) -> Option<T>`。
- photon 响应 GeoJSON：`features[].geometry.coordinates=[lng,lat]`；`properties.{name,city,district,state,country,osm_key,osm_value}`；`properties.extent=[minLng,maxLat,maxLng,minLat]`（可选）。
- Mac 本地有 cargo 1.96：hub 单测本地跑 `cd hub-core && cargo test -p hub-core gev_geocode`（首次编译慢属正常）。

---

### Task 1: worktree + 契约守卫 P7 钉扎

**Files:**
- Modify: `console/src/gev-boot/__tests__/source-contracts.test.ts`（c1 段追加一个 test）

**Interfaces:**
- Consumes: 现有 `readVendor` helper。
- Produces: P7 全部 vendor import 面钉扎（上游同步保险丝）。

- [ ] **Step 1: 建 worktree**

```bash
cd /Volumes/TBU/Workspace/IntelHub && git worktree add ../IntelHub-gev-p7 -b feat/gev-p7-camera-search && sleep 4
cd ../IntelHub-gev-p7/console && npm ci
```

后续全部改动在 `/Volumes/TBU/Workspace/IntelHub-gev-p7`。

- [ ] **Step 2: c1 段追加钉扎 test**

```ts
test("P7 camera orientation + location search render-core exports are pinned", () => {
  const cam = readVendor("src/ui/cameraOrientationControls.js");
  for (const e of [
    "OBLIQUE_PITCH", "STRAIGHT_DOWN_PITCH", "pickViewTarget",
    "readCameraTargetFrame", "setCameraTargetFrame", "frameIsTilted",
    "toggleCameraTilt", "resetCameraNorth", "createCameraOrientationAnimator",
  ]) {
    expect(cam, `cameraOrientationControls.${e}`).toMatch(
      new RegExp(`export (const|function) ${e}\\b`),
    );
  }
  // camera core must never gain a scopeMask/celestialRing dependency (圆圈遮罩禁令)
  expect(cam).not.toMatch(/scopeMask|celestialRing/);

  const ls = readVendor("src/ui/locationSearch.js");
  expect(ls).toMatch(/export class LocationSearch\b/);

  const loc = readVendor("src/locations.js");
  expect(loc).toMatch(/export async function searchAndFlyTo\b/);
  // shell-singleton import we must ALWAYS override via options.features /
  // options.recoverNearView — pin it so upstream changes here trip the guard
  expect(loc).toMatch(/import\s*\{[^}]*applicationServices[^}]*\}\s*from/);

  // layer tracking APIs the follow button drives
  const flights = readVendor("src/layers/flights/queries.js");
  for (const m of ["trackById", "stopTracking", "getTrackedInfo"]) {
    expect(flights, `flights.${m}`).toMatch(new RegExp(`${m}\\s*\\(`));
  }
  const sats = readVendor("src/layers/satellites/controls.js");
  for (const m of ["trackById", "stopTracking", "getTrackedInfo"]) {
    expect(sats, `satellites.${m}`).toMatch(new RegExp(`${m}\\s*\\(`));
  }
});
```

- [ ] **Step 3: 跑守卫验证（钉扎现状，应直接绿；红则停下报告漂移）**

Run: `cd /Volumes/TBU/Workspace/IntelHub-gev-p7/console && npx vitest run src/gev-boot/__tests__/source-contracts.test.ts`
Expected: PASS（含新 test）

- [ ] **Step 4: Commit**

```bash
cd /Volumes/TBU/Workspace/IntelHub-gev-p7 && git add console/src/gev-boot/__tests__/source-contracts.test.ts && git commit -m "test(gev-boot): pin P7 camera/location render-core exports + tracking APIs"
```

---

### Task 2: hub `/api/v1/gev/geocode` 代理（Rust）

**Files:**
- Create: `hub-core/crates/hub-core/src/gev_geocode.rs`
- Modify: `hub-core/crates/hub-core/src/lib.rs`（`pub mod gev_geocode;`，插在 gev_cctv 后保持字母序）
- Modify: `hub-core/crates/hub-core/src/api.rs`（注册路由，追加在 :84 cctv media 行后）

**Interfaces:**
- Consumes: `AppState.redis_timed`（state.rs:34）、axum Query/State 提取器、sha2（cache.rs:21 用法范本）。
- Produces（Task 4 依赖）：`GET /api/v1/gev/geocode?q=<query>` → 200 `{"results":[{"lat":number,"lng":number,"name":string,"label":string,"types":string[],"viewport":{"southwest":{"lat,"lng"},"northeast":{"lat","lng"}}|null}]}`；header `x-geocode-cache: hit|miss`；错误信封 `{"error": string}`（4xx/502）。

- [ ] **Step 1: 写实现 + 单测（一个文件，照 gev_traffic.rs 模式）**

`hub-core/crates/hub-core/src/gev_geocode.rs`：

```rust
//! GEV P7: same-origin geocode proxy for the globe location search.
//!
//! Upstream: photon.komoot.io (keyless). Response normalized to the shape
//! the vendored engine's searchAndFlyTo consumes directly (locations.js:795):
//! viewport as {southwest:{lat,lng}, northeast:{lat,lng}}, types as the
//! engine's navigation-mode tokens (country/administrative/locality/...).
//!
//! Cache: Redis hub:gev:geocode:<sha256(norm-q)[:16]hex>, TTL 1h. The read
//! MUST stay Option<Option<Vec<u8>>> — redis-rs maps Nil → Ok(vec![]) for
//! Vec<u8>, so single-Option reads every miss as hit+empty-body (GEV P3).

use axum::{
    extract::{Query, State},
    http::{HeaderMap, HeaderValue},
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::sync::Arc;

use crate::state::AppState;

const REDIS_BUDGET_MS: u64 = 2000;
const GEOCODE_CACHE_TTL_SECS: u64 = 3600;
const PHOTON_ENDPOINT: &str = "https://photon.komoot.io/api/";
const MAX_QUERY_LEN: usize = 200;
const MAX_RESULTS: usize = 5;

#[derive(Deserialize)]
pub struct GeocodeParams {
    q: String,
}

fn geocode_http() -> &'static reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .user_agent("intelhub-gev-geocode/1.0")
            .build()
            .expect("geocode http client build")
    })
}

fn gev_err(status: StatusCode, msg: &str) -> Response {
    (status, Json(json!({ "error": msg }))).into_response()
}

fn cache_key(query: &str) -> String {
    let norm = query.trim().to_lowercase();
    let digest = Sha256::digest(norm.as_bytes());
    let hex: String = digest[..16].iter().map(|b| format!("{b:02x}")).collect();
    format!("hub:gev:geocode:{hex}")
}

/// photon osm_key/osm_value → engine navigation-mode tokens
/// (locations.js geocodeNavigationMode: country/administrative →
/// region-overview, locality → city-overview, route/park/airport → …).
fn map_types(osm_key: &str, osm_value: &str) -> Vec<&'static str> {
    match (osm_key, osm_value) {
        ("place", "country") => vec!["country"],
        ("place", "state") | ("boundary", "administrative") => vec!["administrative"],
        ("place", "city") | ("place", "town") | ("place", "village") | ("place", "hamlet") => {
            vec!["locality"]
        }
        ("highway", _) => vec!["route"],
        ("aeroway", _) => vec!["airport"],
        ("leisure", "park") | ("boundary", "national_park") => vec!["park"],
        _ => vec!["place"],
    }
}

fn normalize_features(body: &Value) -> Vec<Value> {
    let mut out = Vec::new();
    for feat in body.get("features").and_then(Value::as_array).into_iter().flatten().take(MAX_RESULTS) {
        let coords = feat
            .get("geometry")
            .and_then(|g| g.get("coordinates"))
            .and_then(Value::as_array);
        let (lng, lat) = match coords {
            Some(c) if c.len() >= 2 => (c[0].as_f64(), c[1].as_f64()),
            _ => (None, None),
        };
        let (Some(lat), Some(lng)) = (lat, lng) else { continue };
        let props = feat.get("properties").cloned().unwrap_or(Value::Null);
        let s = |k: &str| props.get(k).and_then(Value::as_str).unwrap_or("");
        let label = [s("name"), s("district"), s("city"), s("state"), s("country")]
            .into_iter()
            .filter(|p| !p.is_empty())
            .collect::<Vec<_>>()
            .join(", ");
        let viewport = props.get("extent").and_then(Value::as_array).and_then(|e| {
            // photon extent: [minLng, maxLat, maxLng, minLat]
            if e.len() == 4 {
                Some(json!({
                    "southwest": { "lat": e[3].as_f64()?, "lng": e[0].as_f64()? },
                    "northeast": { "lat": e[1].as_f64()?, "lng": e[2].as_f64()? },
                }))
            } else {
                None
            }
        });
        out.push(json!({
            "lat": lat,
            "lng": lng,
            "name": s("name"),
            "label": if label.is_empty() { s("name") } else { &label },
            "types": map_types(s("osm_key"), s("osm_value")),
            "viewport": viewport,
        }));
    }
    out
}

pub async fn gev_geocode(
    State(state): State<Arc<AppState>>,
    Query(params): Query<GeocodeParams>,
) -> Result<Response, Response> {
    let q = params.q.trim();
    if q.is_empty() || q.chars().count() > MAX_QUERY_LEN {
        return Err(gev_err(StatusCode::BAD_REQUEST, "invalid query"));
    }
    let key = cache_key(q);
    // Double Option: Nil → Some(None) → miss (see module doc).
    let cached: Option<Option<Vec<u8>>> = state
        .redis_timed(redis::cmd("GET").arg(&key).clone(), REDIS_BUDGET_MS)
        .await;
    if let Some(Some(bytes)) = cached {
        return Ok(json_response(bytes, "hit"));
    }
    let resp = match geocode_http()
        .get(PHOTON_ENDPOINT)
        .query(&[("q", q), ("limit", "5")])
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(error = %e.without_url(), "photon upstream fetch failed");
            return Err(gev_err(StatusCode::BAD_GATEWAY, "geocode upstream failed"));
        }
    };
    if !resp.status().is_success() {
        tracing::warn!(status = %resp.status(), "photon upstream returned error");
        return Err(gev_err(StatusCode::BAD_GATEWAY, "geocode upstream failed"));
    }
    let body: Value = match resp.json().await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e.without_url(), "photon body parse failed");
            return Err(gev_err(StatusCode::BAD_GATEWAY, "geocode upstream failed"));
        }
    };
    let payload = json!({ "results": normalize_features(&body) });
    let bytes = serde_json::to_vec(&payload)
        .map_err(|e| gev_err(StatusCode::INTERNAL_SERVER_ERROR, &format!("encode: {e}")))?;
    let _: Option<String> = state
        .redis_timed(
            redis::cmd("SETEX")
                .arg(&key)
                .arg(GEOCODE_CACHE_TTL_SECS)
                .arg(bytes.as_slice())
                .clone(),
            REDIS_BUDGET_MS,
        )
        .await;
    Ok(json_response(bytes, "miss"))
}

fn json_response(bytes: Vec<u8>, cache: &'static str) -> Response {
    let mut hm = HeaderMap::new();
    hm.insert(axum::http::header::CONTENT_TYPE, HeaderValue::from_static("application/json"));
    hm.insert("x-geocode-cache", HeaderValue::from_static(cache));
    (hm, bytes).into_response()
}
```

文件头部 import 补 `use axum::http::StatusCode;`（上面 gev_err 用到）。

`#[cfg(test)] mod tests`（同文件追加，覆盖纯函数——不碰网络）：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_key_normalizes_case_and_whitespace() {
        assert_eq!(cache_key("  Paris "), cache_key("paris"));
        assert_ne!(cache_key("paris"), cache_key("london"));
    }

    #[test]
    fn normalize_features_maps_photon_geojson() {
        let body = json!({
            "features": [{
                "geometry": { "coordinates": [2.3522, 48.8566] },
                "properties": {
                    "name": "Paris", "city": "Paris", "country": "France",
                    "osm_key": "place", "osm_value": "city",
                    "extent": [2.22, 48.90, 2.47, 48.81]
                }
            }]
        });
        let out = normalize_features(&body);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["lat"], 48.8566);
        assert_eq!(out[0]["lng"], 2.3522);
        assert_eq!(out[0]["types"], json!(["locality"]));
        assert_eq!(out[0]["viewport"]["southwest"], json!({"lat": 48.81, "lng": 2.22}));
        assert_eq!(out[0]["viewport"]["northeast"], json!({"lat": 48.90, "lng": 2.47}));
    }

    #[test]
    fn normalize_features_skips_malformed_and_caps_results() {
        let body = json!({ "features": [
            { "geometry": { "coordinates": [] }, "properties": {} },
            { "geometry": { "coordinates": [0.0, 0.0] }, "properties": { "name": "Null Island", "osm_key": "place", "osm_value": "locality" } }
        ]});
        let out = normalize_features(&body);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["viewport"], Value::Null);
    }

    #[test]
    fn map_types_tokens_match_engine_modes() {
        assert_eq!(map_types("place", "country"), vec!["country"]);
        assert_eq!(map_types("boundary", "administrative"), vec!["administrative"]);
        assert_eq!(map_types("place", "town"), vec!["locality"]);
        assert_eq!(map_types("highway", "residential"), vec!["route"]);
        assert_eq!(map_types("amenity", "cafe"), vec!["place"]);
    }
}
```

- [ ] **Step 2: 注册模块与路由**

lib.rs：`pub mod gev_geocode;`（保持字母序，插在 `pub mod gev_cctv;` 之后）。
api.rs 在 cctv media 路由行（:84）后追加：

```rust
        // GEV P7: location search geocode proxy (photon, keyless, cached 1h)
        .route("/api/v1/gev/geocode", get(crate::gev_geocode::gev_geocode))
```

- [ ] **Step 3: 本地编译 + 测试**

Run: `cd /Volumes/TBU/Workspace/IntelHub-gev-p7/hub-core && cargo test -p hub-core gev_geocode 2>&1 | tail -5`
Expected: 4 passed。若编译错误，修正后重跑；不得绕过。

- [ ] **Step 4: Commit**

```bash
cd /Volumes/TBU/Workspace/IntelHub-gev-p7 && git add hub-core/ && git commit -m "feat(hub): /api/v1/gev/geocode photon proxy (Redis 1h cache, normalized engine shape)"
```

---

### Task 3: 相机姿态适配器 `mountCameraOrientation`

**Files:**
- Create: `console/src/gev-visual/camera-orientation.ts`
- Test: `console/src/gev-visual/__tests__/camera-orientation.test.ts`

**Interfaces:**
- Consumes: vendor cameraOrientationControls 纯函数族。
- Produces（Task 5 依赖）：
  - `interface CameraOrientationHandle { toggleTilt(): "oblique" | "down" | null; resetNorth(): boolean; isTilted(): boolean; destroy(): void }`（toggleTilt 返回切换后的档位；null = 无相机目标可切）
  - `mountCameraOrientation(viewer: CameraViewerLike, deps?: { now?: () => number }): CameraOrientationHandle`
  - `interface CameraViewerLike { camera: { heading: number }; trackedEntity?: unknown }`（结构化最小面）

- [ ] **Step 1: 写失败测试**

`console/src/gev-visual/__tests__/camera-orientation.test.ts`：

```ts
import { describe, expect, test, vi } from "vitest";

// Fake vendor module BEFORE import of the adapter: the adapter consumes the
// vendored pure functions, and the fake replicates their REAL contract
// (toggleCameraTilt flips between OBLIQUE -35° and STRAIGHT_DOWN -89° pitch
// states on a target frame read from the viewer; returns false when there is
// no view target). Lenient-mock guard: fake viewer must carry camera.heading.
vi.mock("gev-engine/src/ui/cameraOrientationControls.js", () => {
  let tilted = false;
  return {
    OBLIQUE_PITCH: -35 * (Math.PI / 180),
    STRAIGHT_DOWN_PITCH: -89 * (Math.PI / 180),
    toggleCameraTilt: (viewer: any) => {
      if (!viewer?.camera || typeof viewer.camera.heading !== "number") {
        throw new TypeError("toggleCameraTilt: viewer.camera.heading missing");
      }
      if (viewer.noTarget) return false;
      tilted = !tilted;
      return { tilted, pitch: tilted ? -35 * (Math.PI / 180) : -89 * (Math.PI / 180) };
    },
    resetCameraNorth: (viewer: any) => {
      if (!viewer?.camera) throw new TypeError("resetCameraNorth: viewer.camera missing");
      return true;
    },
    readCameraTargetFrame: (viewer: any) =>
      viewer.noTarget
        ? null
        : { target: {}, range: 1000, heading: 0, pitch: tilted ? -0.61 : -1.55 },
    setCameraTargetFrame: () => true,
    frameIsTilted: (frame: any) => frame.pitch > -60 * (Math.PI / 180),
    createCameraOrientationAnimator: () => ({
      animate: vi.fn(),
      cancel: vi.fn(),
      get destination() { return null; },
    }),
    pickViewTarget: () => null,
  };
});

import { mountCameraOrientation } from "../camera-orientation";

function fakeViewer(noTarget = false) {
  return { camera: { heading: 0.3 }, trackedEntity: undefined, noTarget };
}

describe("mountCameraOrientation", () => {
  test("constructor contract: rejects viewer without camera.heading", () => {
    expect(() => mountCameraOrientation({} as any)).toThrow(TypeError);
  });

  test("toggleTilt flips oblique → down → oblique and reports the new档位", () => {
    const h = mountCameraOrientation(fakeViewer() as any);
    expect(h.toggleTilt()).toBe("oblique");
    expect(h.isTilted()).toBe(true);
    expect(h.toggleTilt()).toBe("down");
    expect(h.isTilted()).toBe(false);
    h.destroy();
  });

  test("toggleTilt returns null when there is no view target", () => {
    const h = mountCameraOrientation(fakeViewer(true) as any);
    expect(h.toggleTilt()).toBeNull();
    h.destroy();
  });

  test("resetNorth delegates and reports success", () => {
    const h = mountCameraOrientation(fakeViewer() as any);
    expect(h.resetNorth()).toBe(true);
    h.destroy();
  });

  test("destroy is idempotent and cancels the animator", () => {
    const h = mountCameraOrientation(fakeViewer() as any);
    expect(() => { h.destroy(); h.destroy(); }).not.toThrow();
  });
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cd /Volumes/TBU/Workspace/IntelHub-gev-p7/console && npx vitest run src/gev-visual/__tests__/camera-orientation.test.ts`
Expected: FAIL（模块不存在）

- [ ] **Step 3: 实现适配器**

`console/src/gev-visual/camera-orientation.ts`：

```ts
// P7 camera orientation adapter — React-shell seam over the vendored
// cameraOrientationControls pure functions. The vendor module NEVER starts
// or stops tracking (it only reads viewer.trackedEntity); tracking ownership
// stays with the layer trackById APIs (see follow-controller, Task 5).
// scopeMask/celestialRing are not imported anywhere here (圆圈遮罩禁令).
import {
  toggleCameraTilt,
  resetCameraNorth,
  readCameraTargetFrame,
  frameIsTilted,
  createCameraOrientationAnimator,
} from "gev-engine/src/ui/cameraOrientationControls.js";

export interface CameraViewerLike {
  camera: { heading: number };
  trackedEntity?: unknown;
}

export interface CameraOrientationHandle {
  /** Flip oblique(-35°) ↔ straight-down(-89°). Returns the NEW档位, or null
   *  when there is no view target (nothing tracked, no ground under center). */
  toggleTilt(): "oblique" | "down" | null;
  resetNorth(): boolean;
  isTilted(): boolean;
  destroy(): void;
}

export function mountCameraOrientation(
  viewer: CameraViewerLike,
  deps: { now?: () => number } = {},
): CameraOrientationHandle {
  if (!viewer?.camera || typeof viewer.camera.heading !== "number") {
    throw new TypeError("mountCameraOrientation: viewer.camera.heading missing");
  }
  const animator = createCameraOrientationAnimator(viewer, deps.now ? { now: deps.now } : {});
  let destroyed = false;

  return {
    toggleTilt() {
      const result = toggleCameraTilt(viewer);
      if (result === false) return null;
      return result.tilted ? "oblique" : "down";
    },
    resetNorth() {
      return resetCameraNorth(viewer);
    },
    isTilted() {
      // Paid read (4-12ms depth buffer) — callers must be user gestures or
      // post-gesture state syncs, NEVER per-frame (engine perf discipline).
      const frame = readCameraTargetFrame(viewer);
      return frame ? frameIsTilted(frame) : false;
    },
    destroy() {
      if (destroyed) return;
      destroyed = true;
      animator.cancel();
    },
  };
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cd /Volumes/TBU/Workspace/IntelHub-gev-p7/console && npx vitest run src/gev-visual/__tests__/camera-orientation.test.ts`
Expected: PASS 5/5（注意 vi.mock 的 module-level `tilted` 状态跨 test 泄漏——在实现侧无状态，若测试间串台，在 fake 里加 `__reset()` 导出并在 beforeEach 调用；此调整允许，记入报告）

- [ ] **Step 5: Commit**

```bash
cd /Volumes/TBU/Workspace/IntelHub-gev-p7 && git add console/src/gev-visual/ && git commit -m "feat(gev-visual): camera orientation adapter (tilt toggle + reset north)"
```

---

### Task 4: 地点搜索适配器 `mountLocationSearch`

**Files:**
- Create: `console/src/gev-visual/location-search.ts`
- Test: `console/src/gev-visual/__tests__/location-search.test.ts`

**Interfaces:**
- Consumes: Task 2 的 hub geocode 端点；vendor `LocationSearch` 类 + `searchAndFlyTo`；`disabledFeatures()`（console/src/gev-boot/request-services.ts:30-47 现成导出——执行时 grep 确认导出名）；console/src/gev-adapters/http.ts 的 `ApiFetch` 类型。
- Produces（Task 6 依赖）：
  - `type SearchState = "idle" | "searching" | "found" | "missing" | "failed"`
  - `interface LocationSearchHandle { run(query: string): Promise<void>; getState(): SearchState; subscribe(fn: (s: SearchState) => void): () => void; destroy(): void }`
  - `mountLocationSearch(viewer: unknown, input: HTMLInputElement, apiFetch: ApiFetch): LocationSearchHandle`

- [ ] **Step 1: 写失败测试**

`console/src/gev-visual/__tests__/location-search.test.ts`：

```ts
import { describe, expect, test, vi, beforeEach } from "vitest";

// Fake vendor LocationSearch: replicate the real state machine contract
// (run → search callback → onResult/onMissing/onError; input gets
// classList 'searching' during flight and blur() on settle). Lenient-mock
// guard: constructor asserts input has classList + blur.
const instances: any[] = [];
vi.mock("gev-engine/src/ui/locationSearch.js", () => ({
  LocationSearch: class {
    state = "idle";
    listeners = new Set<Function>();
    constructor(opts: any) {
      if (!opts.input?.classList || typeof opts.input.blur !== "function") {
        throw new TypeError("LocationSearch: input must have classList and blur()");
      }
      this.opts = opts;
      instances.push(this);
    }
    async run(query: string) {
      this.set("searching");
      this.opts.input.classList.add("searching");
      try {
        const dest = await this.opts.search(query, { signal: undefined });
        this.set(dest ? "found" : "missing");
      } catch {
        this.set("failed");
      } finally {
        this.opts.input.classList.remove("searching");
        this.opts.input.blur();
      }
    }
    set(s: string) { this.state = s; this.listeners.forEach((f) => f(s)); }
    getState() { return this.state; }
    subscribe(fn: Function) { this.listeners.add(fn); return () => this.listeners.delete(fn); }
    destroy() { this.listeners.clear(); }
    private opts: any;
  },
}));

// searchAndFlyTo fake: records options so the test can assert the
// applicationServices bypass (features + recoverNearView overrides).
const flyCalls: any[] = [];
vi.mock("gev-engine/src/locations.js", () => ({
  searchAndFlyTo: vi.fn(async (_viewer: any, query: string, options: any) => {
    flyCalls.push({ query, options });
    const outcome = await options.placeSearch.geocode(query, { signal: undefined });
    return outcome.place ? { label: outcome.place.label, navigationMode: "city-overview", rangeM: 50000 } : null;
  }),
}));

import { mountLocationSearch } from "../location-search";
import type { ApiFetch } from "../../gev-adapters/http";

function fakeInput() {
  return {
    classList: { add: vi.fn(), remove: vi.fn() },
    blur: vi.fn(),
  } as unknown as HTMLInputElement;
}

function apiFetchReturning(body: unknown, ok = true): ApiFetch {
  return vi.fn(async () => ({
    ok,
    json: async () => body,
  })) as unknown as ApiFetch;
}

beforeEach(() => { instances.length = 0; flyCalls.length = 0; });

describe("mountLocationSearch", () => {
  test("found: hub result flies and state ends at found", async () => {
    const fetch = apiFetchReturning({ results: [{ lat: 48.85, lng: 2.35, name: "Paris", label: "Paris, France", types: ["locality"], viewport: null }] });
    const h = mountLocationSearch({} as any, fakeInput(), fetch);
    const states: string[] = [];
    h.subscribe((s) => states.push(s));
    await h.run("Paris");
    expect(h.getState()).toBe("found");
    expect(states).toEqual(["searching", "found"]);
    expect((fetch as any).mock.calls[0][0]).toBe("/api/v1/gev/geocode?q=Paris");
    // applicationServices bypass: explicit overrides present
    const opts = flyCalls[0].options;
    expect(typeof opts.features).toBeDefined();
    expect(await opts.recoverNearView()).toBeNull();
    h.destroy();
  });

  test("missing: empty results → state missing, no fly", async () => {
    const fetch = apiFetchReturning({ results: [] });
    const h = mountLocationSearch({} as any, fakeInput(), fetch);
    await h.run("zzz-no-such-place");
    expect(h.getState()).toBe("missing");
    h.destroy();
  });

  test("failed: hub 5xx → state failed", async () => {
    const fetch = apiFetchReturning({ error: "geocode upstream failed" }, false);
    const h = mountLocationSearch({} as any, fakeInput(), fetch);
    await h.run("paris");
    expect(h.getState()).toBe("failed");
    h.destroy();
  });

  test("query is URL-encoded", async () => {
    const fetch = apiFetchReturning({ results: [] });
    const h = mountLocationSearch({} as any, fakeInput(), fetch);
    await h.run("São Paulo & beyond");
    expect((fetch as any).mock.calls[0][0]).toBe("/api/v1/gev/geocode?q=S%C3%A3o%20Paulo%20%26%20beyond");
    h.destroy();
  });
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cd /Volumes/TBU/Workspace/IntelHub-gev-p7/console && npx vitest run src/gev-visual/__tests__/location-search.test.ts`
Expected: FAIL（模块不存在）

- [ ] **Step 3: 实现适配器**

`console/src/gev-visual/location-search.ts`：

```ts
// P7 location search adapter — wraps the vendored LocationSearch state
// machine with our hub geocode proxy. Two mandatory overrides keep the
// vendored searchAndFlyTo off the shell singletons we do not vendor:
//   options.features       — default reads applicationServices.features and
//                            would POST a non-existent same-origin /api/overpass
//   options.recoverNearView — default pulls annotationResolver's Overpass
//                            recovery; P7 disables it (async () => null)
import { LocationSearch } from "gev-engine/src/ui/locationSearch.js";
import { searchAndFlyTo } from "gev-engine/src/locations.js";
import { disabledFeatures } from "../gev-boot/request-services";
import type { ApiFetch } from "../gev-adapters/http";

export type SearchState = "idle" | "searching" | "found" | "missing" | "failed";

export interface LocationSearchHandle {
  run(query: string): Promise<void>;
  getState(): SearchState;
  subscribe(fn: (s: SearchState) => void): () => void;
  destroy(): void;
}

/** Engine placeSearch contract: geocode(query, {bias, signal}) →
 *  { place: {lat,lng,label,types,viewport(southwest/northeast)} | null, answered }. */
export function createHubPlaceSearch(apiFetch: ApiFetch) {
  return {
    async geocode(query: string, _ctx: { bias?: unknown; signal?: AbortSignal }) {
      let resp: Response;
      try {
        resp = await apiFetch(`/api/v1/gev/geocode?q=${encodeURIComponent(query)}`, {
          signal: _ctx.signal,
        });
      } catch {
        return { place: null, answered: false };
      }
      if (!resp.ok) return { place: null, answered: false };
      const body = (await resp.json()) as { results?: Array<Record<string, unknown>> };
      const first = body.results?.[0] ?? null;
      return { place: first, answered: true };
    },
  };
}

export function mountLocationSearch(
  viewer: unknown,
  input: HTMLInputElement,
  apiFetch: ApiFetch,
): LocationSearchHandle {
  const placeSearch = createHubPlaceSearch(apiFetch);
  const engine = new LocationSearch({
    input,
    search: (query: string, opts: { signal?: AbortSignal }) =>
      searchAndFlyTo(viewer, query, {
        placeSearch,
        features: disabledFeatures(),
        recoverNearView: async () => null,
        signal: opts?.signal,
      }),
  });
  let destroyed = false;
  return {
    run: (query: string) => engine.run(query),
    getState: () => engine.getState() as SearchState,
    subscribe: (fn: (s: SearchState) => void) => engine.subscribe(fn),
    destroy() {
      if (destroyed) return;
      destroyed = true;
      engine.destroy();
    },
  };
}
```

注意：`disabledFeatures` 的精确导出名/签名以 console/src/gev-boot/request-services.ts 实际为准（探索报告 :30-47 有 `disabledFeatures()` 九方法全 disabled）；若实际签名不同（比如是对象不是工厂），适配并以报告注明。

- [ ] **Step 4: 跑测试确认通过**

Run: `cd /Volumes/TBU/Workspace/IntelHub-gev-p7/console && npx vitest run src/gev-visual/__tests__/location-search.test.ts`
Expected: PASS 4/4

- [ ] **Step 5: Commit**

```bash
cd /Volumes/TBU/Workspace/IntelHub-gev-p7 && git add console/src/gev-visual/ && git commit -m "feat(gev-visual): location search adapter over hub geocode proxy"
```

---

### Task 5: 跟随/姿态按钮进详情面板 + 回北进顶条

**Files:**
- Create: `console/src/gev-visual/follow-controller.ts` + `console/src/gev-visual/__tests__/follow-controller.test.ts`
- Modify: `console/src/globe-hud/HudDetailPanel.tsx`（零 props → 接收 `follow` prop；动作区加按钮）
- Modify: `console/src/globe-hud/HudTopBar.tsx`（加回北按钮，可选 prop）
- Modify: `console/src/pages/GlobeV2.tsx`（穿 props：viewer/railManager → HudDetailPanel follow controller；cameraOrientation → HudTopBar）
- Test: `console/src/globe-hud/__tests__/hud-follow.test.tsx`

**Interfaces:**
- Consumes: Task 3 `CameraOrientationHandle`；`useGlobeSelection()`（context-bridge）；GlobeV2 的 `sceneHandles.viewer` + `railManager` state。
- Produces：
  - `interface FollowHandle { follow(kind: "flight" | "satellite", id: string): boolean; unfollow(): void; trackedId(): string | null; }`——follow 内部走 `dataManager.layers.get('<flights|satellites>').module.trackById(id)`（satellite 要 `Number(id)`）；失败（layer 未启用/trackById 返 false）返回 false
  - `mountFollowController(dataManager: unknown): FollowHandle`
  - HudDetailPanel 新 props：`{ follow?: FollowHandle | null; camera?: CameraOrientationHandle | null }`（可选，现有调用零破坏）
  - HudTopBar 新 props：`camera?: CameraOrientationHandle | null`（可选）

- [ ] **Step 1: follow-controller 失败测试**

`console/src/gev-visual/__tests__/follow-controller.test.ts`：

```ts
import { describe, expect, test, vi } from "vitest";
import { mountFollowController } from "../follow-controller";

function fakeDataManager(opts: { flightsTrackOk?: boolean; satsTrackOk?: boolean } = {}) {
  const flightsModule = {
    trackById: vi.fn((id: unknown, opts?: unknown) => {
      if (typeof id !== "string") throw new TypeError("flights.trackById: icao24 must be string");
      return opts?.flightsTrackOk ?? true;
    }),
    stopTracking: vi.fn(() => true),
    getTrackedInfo: vi.fn(() => ({ icao24: "abc123" })),
  };
  const satellitesModule = {
    trackById: vi.fn((id: unknown) => {
      if (typeof id !== "number") throw new TypeError("satellites.trackById: noradId must be number");
      return opts?.satsTrackOk ?? true;
    }),
    stopTracking: vi.fn(() => true),
    getTrackedInfo: vi.fn(() => null),
  };
  const layers = new Map([
    ["flights", { module: flightsModule }],
    ["satellites", { module: satellitesModule }],
  ]);
  return { layers, flightsModule, satellitesModule };
}

describe("mountFollowController", () => {
  test("constructor contract: rejects dataManager without layers Map", () => {
    expect(() => mountFollowController({} as any)).toThrow(TypeError);
  });

  test("follow flight tracks via flights module with string icao24", () => {
    const dm = fakeDataManager();
    const h = mountFollowController(dm as any);
    expect(h.follow("flight", "abc123")).toBe(true);
    expect(dm.flightsModule.trackById).toHaveBeenCalledWith("abc123", { origin: "programmatic" });
    expect(h.trackedId()).toBe("abc123");
  });

  test("follow satellite converts noradId to number", () => {
    const dm = fakeDataManager();
    const h = mountFollowController(dm as any);
    expect(h.follow("satellite", "25544")).toBe(true);
    expect(dm.satellitesModule.trackById).toHaveBeenCalledWith(25544, { origin: "programmatic" });
  });

  test("follow returns false when layer missing or trackById declines", () => {
    const dm = fakeDataManager({ flightsTrackOk: false });
    const h = mountFollowController(dm as any);
    expect(h.follow("flight", "abc123")).toBe(false);
    expect(h.trackedId()).toBeNull();
    dm.layers.delete("flights");
    expect(h.follow("flight", "abc123")).toBe(false);
  });

  test("unfollow stops tracking on the owning layer and clears trackedId", () => {
    const dm = fakeDataManager();
    const h = mountFollowController(dm as any);
    h.follow("flight", "abc123");
    h.unfollow();
    expect(dm.flightsModule.stopTracking).toHaveBeenCalledWith({ origin: "programmatic" });
    expect(h.trackedId()).toBeNull();
  });

  test("follow switches layers: previous layer stopTracking called", () => {
    const dm = fakeDataManager();
    const h = mountFollowController(dm as any);
    h.follow("flight", "abc123");
    h.follow("satellite", "25544");
    expect(dm.flightsModule.stopTracking).toHaveBeenCalled();
    expect(h.trackedId()).toBe("25544");
  });
});
```

- [ ] **Step 2: 跑失败 → 实现 follow-controller.ts**

```ts
// P7 follow controller — owns "which layer is tracking which id" on behalf of
// the HUD. Tracking itself lives in the engine layers (trackById sets
// viewer.trackedEntity + applyTrackedCameraFrame internally); this adapter
// only routes to the right layer module and keeps the book.
export type FollowKind = "flight" | "satellite";

const LAYER_BY_KIND: Record<FollowKind, string> = {
  flight: "flights",
  satellite: "satellites",
};

export interface FollowHandle {
  follow(kind: FollowKind, id: string): boolean;
  unfollow(): void;
  trackedId(): string | null;
}

interface LayerModule {
  trackById(id: string | number, opts: { origin: string }): boolean;
  stopTracking(opts: { origin: string }): void;
}

export function mountFollowController(dataManager: { layers?: Map<string, { module: LayerModule }> }): FollowHandle {
  if (!(dataManager?.layers instanceof Map)) {
    throw new TypeError("mountFollowController: dataManager.layers must be a Map");
  }
  let current: { kind: FollowKind; id: string } | null = null;

  const moduleFor = (kind: FollowKind): LayerModule | null =>
    dataManager.layers!.get(LAYER_BY_KIND[kind])?.module ?? null;

  return {
    follow(kind, id) {
      const module = moduleFor(kind);
      if (!module) return false;
      const arg: string | number = kind === "satellite" ? Number(id) : id;
      if (kind === "satellite" && !Number.isFinite(arg)) return false;
      const ok = module.trackById(arg, { origin: "programmatic" });
      if (!ok) return false;
      if (current && current.kind !== kind) {
        moduleFor(current.kind)?.stopTracking({ origin: "programmatic" });
      }
      current = { kind, id };
      return true;
    },
    unfollow() {
      if (!current) return;
      moduleFor(current.kind)?.stopTracking({ origin: "programmatic" });
      current = null;
    },
    trackedId: () => current?.id ?? null,
  };
}
```

Run: `cd /Volumes/TBU/Workspace/IntelHub-gev-p7/console && npx vitest run src/gev-visual/__tests__/follow-controller.test.ts` → PASS 6/6

- [ ] **Step 3: HudDetailPanel 动作区按钮 + 测试**

HudDetailPanel.tsx：
- 新增可选 props：`{ follow = null, camera = null }: { follow?: FollowHandle | null; camera?: CameraOrientationHandle | null }`（import type 自 ../gev-visual/follow-controller 与 ../gev-visual/camera-orientation）。
- 内部用 `useGlobeSelection()` 现有返回值：kind ∈ {flight, satellite} 且 follow 非空时，在 `.hud-detail-actions` 容器内「进图谱」链接旁渲染：
  - 未跟随该对象：`<button data-testid="hud-follow-button" className="hud-detail-link" onClick={...}>跟随</button>`——点击 `follow(kind === "flight" ? "flight" : "satellite", kind === "flight" ? String(data.id) : String(data.noradId))`；返回 false 时按钮文案短暂变「图层未启用」（本地 state，2s 后还原）
  - 已跟随该对象（`follow.trackedId() === 当前 id`）：渲染「解除跟随」`data-testid="hud-unfollow-button"` + 「斜视/俯视」`data-testid="hud-tilt-button"`（点击 `camera?.toggleTilt()`，按钮文案随返回值变「俯视」/「斜视」——初始文案「斜视」）
- 组件内部用 `useState` 跟踪 trackedId/tilt 档位；selection 变化时（useEffect on selection）若 trackedId 存在且不等于新选中 id 则保持（引擎层追踪继续），按钮态按 trackedId === 当前 id 计算。
- 注意 HudDetailPanel 现有结构是内部 switch kind → Body 组件；按钮统一放在外层（`HudDetailPanel` 函数体内、Body 之下）而非每个 Body 里改——先看代码选最自然的挂载点，保持 diff 最小。

`console/src/globe-hud/__tests__/hud-follow.test.tsx`：

```tsx
import "@testing-library/jest-dom/vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, test, vi } from "vitest";

// Mock the selection hook BEFORE importing the panel: drives kind/data per test.
const selection = { current: { kind: null as string | null, data: null as any } };
vi.mock("../../gev-boot/context-bridge", async (importOriginal) => {
  const orig = await importOriginal<any>();
  return { ...orig, useGlobeSelection: () => selection.current };
});

import { HudDetailPanel } from "../HudDetailPanel";
import type { FollowHandle } from "../../gev-visual/follow-controller";
import type { CameraOrientationHandle } from "../../gev-visual/camera-orientation";

function fakeFollow(): FollowHandle & { follow: ReturnType<typeof vi.fn> } {
  let tracked: string | null = null;
  return {
    follow: vi.fn((_k: string, id: string) => { tracked = id; return true; }),
    unfollow: vi.fn(() => { tracked = null; }),
    trackedId: () => tracked,
  } as any;
}

function fakeCamera(): CameraOrientationHandle & { toggleTilt: ReturnType<typeof vi.fn> } {
  return {
    toggleTilt: vi.fn(() => "down" as const),
    resetNorth: vi.fn(() => true),
    isTilted: vi.fn(() => false),
    destroy: vi.fn(),
  };
}

afterEach(() => { cleanup(); selection.current = { kind: null, data: null }; });

describe("HudDetailPanel follow buttons", () => {
  test("flight selection shows 跟随; click tracks with icao24", () => {
    selection.current = { kind: "flight", data: { id: "abc123", callsign: "TEST1" } };
    const follow = fakeFollow();
    render(<HudDetailPanel follow={follow} camera={fakeCamera()} />);
    fireEvent.click(screen.getByTestId("hud-follow-button"));
    expect(follow.follow).toHaveBeenCalledWith("flight", "abc123");
  });

  test("tracked flight shows 解除跟随 + 斜视 buttons; tilt delegates to camera", () => {
    selection.current = { kind: "flight", data: { id: "abc123" } };
    const follow = fakeFollow();
    const camera = fakeCamera();
    render(<HudDetailPanel follow={follow} camera={camera} />);
    fireEvent.click(screen.getByTestId("hud-follow-button")); // now tracked
    fireEvent.click(screen.getByTestId("hud-tilt-button"));
    expect(camera.toggleTilt).toHaveBeenCalled();
    fireEvent.click(screen.getByTestId("hud-unfollow-button"));
    expect(follow.unfollow).toHaveBeenCalled();
  });

  test("satellite selection follows with stringified noradId", () => {
    selection.current = { kind: "satellite", data: { noradId: "25544", name: "ISS" } };
    const follow = fakeFollow();
    render(<HudDetailPanel follow={follow} camera={fakeCamera()} />);
    fireEvent.click(screen.getByTestId("hud-follow-button"));
    expect(follow.follow).toHaveBeenCalledWith("satellite", "25544");
  });

  test("quake selection shows no follow button", () => {
    selection.current = { kind: "quake", data: { id: "us7000" } };
    render(<HudDetailPanel follow={fakeFollow()} camera={fakeCamera()} />);
    expect(screen.queryByTestId("hud-follow-button")).not.toBeInTheDocument();
  });

  test("follow failure surfaces 图层未启用 feedback", () => {
    selection.current = { kind: "flight", data: { id: "abc123" } };
    const follow = fakeFollow();
    follow.follow.mockReturnValue(false as any);
    render(<HudDetailPanel follow={follow} camera={fakeCamera()} />);
    fireEvent.click(screen.getByTestId("hud-follow-button"));
    expect(screen.getByTestId("hud-follow-button")).toHaveTextContent("图层未启用");
  });
});
```

注意：现有 hud-detail-panel 测试文件（globe-hud/__tests__/）若已 mock context-bridge，参照其 mock 模式保持一致；本测试文件独立 mock 不冲突。

- [ ] **Step 4: HudTopBar 回北按钮**

HudTopBar.tsx 新增可选 prop `camera?: CameraOrientationHandle | null`；在滤镜切换器旁渲染（camera 非空时）：

```tsx
<button
  type="button"
  className="hud-bar-back"
  onClick={() => camera?.resetNorth()}
  title="回北 / Reset north"
  aria-label="Reset north"
  data-testid="hud-north-button"
>
  ▲ N
</button>
```

- [ ] **Step 5: GlobeV2 接线**

- boot effect 的 .then() 链内（setSceneHandles 之后、visualEffects 创建旁）：`cameraRef.current = mountCameraOrientation(components?.scene?.viewer as any)`（同样 if(viewer) 守卫 + try/catch console.warn）；`followRef.current = mountFollowController(components?.data?.dataManager)`（dataManager 存在性守卫）；分别 setState 下穿。
- cleanup：camera/follow 的 destroy 在 `globe.destroy()` 之前（与 visualEffects 同位置，顺序：follow → camera → visualEffects → globe）。
- JSX：`right={<HudDetailPanel follow={follow} camera={cameraOrientation} />}`、`<HudTopBar ... camera={cameraOrientation} />`。
- follow controller 无引擎资源（纯记账），destroy 可为空实现——handle 接口不加 destroy。

- [ ] **Step 6: 全量验证**

Run: `cd /Volumes/TBU/Workspace/IntelHub-gev-p7/console && npx tsc -b && npx vitest run 2>&1 | tail -5`
Expected: tsc clean；vitest 相比基线（174 passed / 5 既有 hud-bars 失败 + P6 新增）无新增失败，新测试文件全绿

- [ ] **Step 7: Commit**

```bash
cd /Volumes/TBU/Workspace/IntelHub-gev-p7 && git add console/src/ && git commit -m "feat(hud): follow/tilt buttons in detail panel + reset-north in top bar"
```

---

### Task 6: HudTopBar 搜索框实装 + P5 占位熵减

**Files:**
- Modify: `console/src/globe-hud/HudTopBar.tsx`（搜索框启用 + LocationSearch 接线）
- Modify: `console/src/globe-hud/hud.css`（删 `.hud-bar-search:disabled` 死规则；补搜索状态样式）
- Modify: `console/src/pages/GlobeV2.tsx`（创建 locationSearch handle 下穿）
- Test: `console/src/globe-hud/__tests__/hud-search.test.tsx`
- Modify（若有）: 引用 `hud-search-p5` 的既有测试（先 grep）

**Interfaces:**
- Consumes: Task 4 `mountLocationSearch` / `LocationSearchHandle` / `SearchState`。
- Produces: testid `hud-search-location`（输入框）、`hud-search-status`（状态提示，found/missing/failed 文案 zh/en）。

- [ ] **Step 1: grep 旧 testid 引用**

Run: `cd /Volumes/TBU/Workspace/IntelHub-gev-p7 && grep -rn "hud-search-p5\|P5 待实现\|(P5)" console/src console/probe-gev.mjs`
全部列出并随本任务同步更新（熵减：占位语义零残留）。

- [ ] **Step 2: 写失败测试**

`console/src/globe-hud/__tests__/hud-search.test.tsx`：

```tsx
import "@testing-library/jest-dom/vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { afterEach, describe, expect, test, vi } from "vitest";
import { HudTopBar } from "../HudTopBar";
import type { LocationSearchHandle, SearchState } from "../../gev-visual/location-search";

function fakeSearch(initial: SearchState = "idle"): LocationSearchHandle & {
  run: ReturnType<typeof vi.fn>;
  setState(s: SearchState): void;
} {
  let state = initial;
  const listeners = new Set<(s: SearchState) => void>();
  return {
    run: vi.fn(async () => {}),
    getState: () => state,
    subscribe: (fn) => { listeners.add(fn); return () => { listeners.delete(fn); }; },
    setState(s: SearchState) { state = s; listeners.forEach((f) => f(s)); },
    destroy: vi.fn(),
  };
}

// HudTopBar uses useNavigate/useLocation (Back button) — tests must wrap in
// MemoryRouter. (The 5 pre-existing hud-bars failures are exactly this
// missing-wrapper bug; do not replicate it.)
function renderTopBar(locationSearch: LocationSearchHandle | null) {
  return render(
    <MemoryRouter>
      <HudTopBar overview={null} locationSearch={locationSearch} />
    </MemoryRouter>,
  );}

afterEach(() => cleanup());

describe("HudTopBar location search", () => {
  test("search input is enabled with location placeholder", () => {
    renderTopBar(fakeSearch());
    const input = screen.getByTestId("hud-search-location");
    expect(input).toBeEnabled();
    expect(input).toHaveAttribute("placeholder", expect.stringContaining("地点"));
  });

  test("Enter triggers run with the typed query", () => {
    const search = fakeSearch();
    renderTopBar(search);
    const input = screen.getByTestId("hud-search-location");
    fireEvent.change(input, { target: { value: "Paris" } });
    fireEvent.keyDown(input, { key: "Enter" });
    expect(search.run).toHaveBeenCalledWith("Paris");
  });

  test("missing state renders zh/en feedback", () => {
    renderTopBar(fakeSearch("missing"));
    expect(screen.getByTestId("hud-search-status")).toHaveTextContent("未找到");
  });

  test("null handle keeps the input disabled (engine not ready)", () => {
    renderTopBar(null);
    expect(screen.getByTestId("hud-search-location")).toBeDisabled();
  });
});
```

- [ ] **Step 3: 跑失败 → 实现**

HudTopBar.tsx：
- 替换 :98-108 占位 input：去 `disabled`，`data-testid="hud-search-location"`，placeholder `地点搜索 / Location`，aria-label/title 同步（删 P5 字样）。
- 新增可选 prop `locationSearch?: LocationSearchHandle | null`；input 的 onKeyDown Enter → `locationSearch?.run(value)`；非空校验（trim 后空串不调）。
- 搜索状态提示：subscribe 到 handle，state !== idle && !== searching 时在 input 旁渲染 `<span data-testid="hud-search-status" className="hud-search-status">`，文案映射：searching→`搜索中…`、found→空（飞行即反馈，不渲染）、missing→`未找到 / Not found`、failed→`搜索失败 / Search failed`；status 在 4s 后自动清除（useEffect setTimeout + cleanup）或下一次输入时清除。
- `locationSearch` 为 null 时 input 渲染但 disabled（引擎未就绪的诚实降级，不是 P5 占位）。

hud.css：删 `.hud-bar-search:disabled` 整条死规则（:471）；`.hud-search-status` 样式照 `.hud-bar-stat` token（font-size 11px、玻璃色）。

GlobeV2.tsx：boot effect .then() 链内创建 locationSearch handle——需要真实 input DOM 元素。模式：HudTopBar 内建 handle 而非 GlobeV2（input ref 在 HudTopBar 手里）——**修正方案**：HudTopBar 接收 `apiFetch` + `viewer`（可选 props），内部用 `useRef<HTMLInputElement>` + `useEffect`（viewer 就绪后）自调 `mountLocationSearch(viewer, inputRef.current, apiFetch)`，cleanup destroy。GlobeV2 只需把现成 apiFetch（:77 已创建）与 sceneHandles.viewer 下穿。选这个方案（组件自管理，GlobeV2 diff 最小）。

- [ ] **Step 4: 全量验证**

Run: `cd /Volumes/TBU/Workspace/IntelHub-gev-p7/console && npx tsc -b && npx vitest run 2>&1 | tail -5`
Expected: 无新增失败（基线 5 个 hud-bars 既有失败）；若本任务给 HudTopBar 包了 Router 修复样板导致 hud-bars 5 个失败意外转绿——这是意外收获，在报告注明（不算违规）。

- [ ] **Step 5: Commit**

```bash
cd /Volumes/TBU/Workspace/IntelHub-gev-p7 && git add console/src/ && git commit -m "feat(hud): location search live in top bar (hub geocode proxy); P5 placeholder retired"
```

---

### Task 7: probe 断言 + 315 验收（含 hub 构建）

**Files:**
- Modify: `console/probe-gev.mjs`（滤镜段后追加搜索+跟随段）

**Interfaces:**
- Consumes: testid `hud-search-location` / `hud-search-status` / `hud-follow-button`；hub 端点 `/api/v1/gev/geocode`。
- Produces: probe 退出码覆盖 P7 面。

- [ ] **Step 1: probe 追加段**

滤镜段之后追加：

```js
// ── P7: location search flies the camera; geocode proxy answers ──
const geo = await page.request.get(`${base}/api/v1/gev/geocode?q=Paris`, {
  headers: { Authorization: `Bearer ${key}` },
});
if (geo.status() !== 200) {
  failures.push(`geocode proxy http=${geo.status()}`);
} else {
  const body = await geo.json();
  if (!body.results?.[0]?.label) failures.push("geocode proxy returned no results for Paris");
  const geo2 = await page.request.get(`${base}/api/v1/gev/geocode?q=Paris`, {
    headers: { Authorization: `Bearer ${key}` },
  });
  if (geo2.headers()["x-geocode-cache"] !== "hit") {
    warnings.push("geocode second request did not hit cache");
  }
}
const searchInput = await gate('[data-testid="hud-search-location"]', DATA_TIMEOUT_MS, "search input");
if (searchInput) {
  const before = await page.evaluate(() => {
    const c = (window as any).__gevViewer?.camera;   // 若引擎未暴露 viewer 全局，见下行替代
    return c ? [c.position.x, c.position.y, c.position.z] : null;
  }).catch(() => null);
  await page.fill('[data-testid="hud-search-location"]', "Paris");
  await page.press('[data-testid="hud-search-location"]', "Enter");
  await page.waitForTimeout(4500); // geocode + 3s flyTo duration
  const moved = await page.evaluate(() => {
    const c = (window as any).__gevViewer?.camera;
    return c ? [c.position.x, c.position.y, c.position.z] : null;
  }).catch(() => null);
  if (before && moved && JSON.stringify(before) === JSON.stringify(moved)) {
    failures.push("camera did not move after location search");
  } else if (!before || !moved) {
    // viewer 无全局暴露——降级为状态提示断言
    const status = await page.textContent('[data-testid="hud-search-status"]').catch(() => null);
    if (status && /未找到|失败/.test(status)) failures.push(`location search surfaced: ${status.trim()}`);
  }
  if (pageErrors.length) failures.push("page errors during location search");
}
```

注意：`window.__gevViewer` 大概率不存在——实现时先读 console/src/pages/GlobeV2.tsx 确认是否有 viewer 全局暴露；没有则用 `page.evaluate` 读 canvas 截图字节对比（同滤镜段模式）作为 moved 判据，或直接依赖「无 failure 状态提示 + 无 pageerror」弱断言并在报告注明。probe 的 `base`/`key` 变量名以文件实际为准。

- [ ] **Step 2: rsync → 315 构建（hub + console 都构建）→ 重启**

照 brief 标准序列，**本期必须跑 `bash scripts/build-hub.sh`**（hub-core 有改动）+ `bash scripts/build-console.sh` + `sudo systemctl restart hub-core`。构建后确认 hub 二进制更新：`ssh Debian-test 'ls -la /home/zou/IntelHub/core/hub'`。

- [ ] **Step 3: 315 上验证 geocode 端点 + probe**

```bash
KEY=$(ssh -o BatchMode=yes Debian-test 'grep -o "ihk_[a-f0-9]*" /home/zou/IntelHub/core/agent-keys.txt | head -1')
curl -s -H "Authorization: Bearer $KEY" "http://10.10.10.35:8800/api/v1/gev/geocode?q=Paris" | head -c 300
curl -s -o /dev/null -w "%{http_code} %header{x-geocode-cache}\n" -H "Authorization: Bearer $KEY" "http://10.10.10.35:8800/api/v1/gev/geocode?q=Paris"   # 第二次应 hit
cd /Volumes/TBU/Workspace/IntelHub-gev-p7 && node console/probe-gev.mjs http://10.10.10.35:8800 "$KEY"
```

Expected: 端点 200 + results 非空 + 第二次 x-geocode-cache: hit；probe exit 0。
**若 photon 从 315 出口被限流/不通**（curl 观察）：切 fallback `https://nominatim.openstreetmap.org/search?format=jsonv2&q=...&limit=5`（User-Agent 头必须保留；响应 shape 不同——normalize 函数重写并在报告注明），重新走 Step 2-3。只允许这一次上游切换，再不通则 BLOCKED 上报。

- [ ] **Step 4: 315 全量验收**

```bash
KEY=$(ssh -o BatchMode=yes Debian-test 'grep -o "ihk_[a-f0-9]*" /home/zou/IntelHub/core/agent-keys.txt | head -1')
for a in sp8 sp6 sp7 sp3; do
  echo "── $a: $(INTELHUB_SSH=Debian-test python3 scripts/accept-$a.py "$KEY" http://10.10.10.35:8800 2>&1 | grep -E '==.*(passed|failed)' | tail -1)"
done
```

Expected: sp8 39+2sh/0f、sp6 ≤1f（仅 starlink flap 可接受）、sp7 16+11sh/0f、sp3 19/0f。

- [ ] **Step 5: Commit**

```bash
cd /Volumes/TBU/Workspace/IntelHub-gev-p7 && git add console/probe-gev.mjs && git commit -m "test(probe): P7 location-search + geocode-proxy assertions"
```

---

### Task 8: 熵减 + ledger + 合并部署

**Files:**
- Create: `docs/superpowers/execution/2026-09-18-gev-p7-ledger.md`
- Modify: `AGENTS.md`（验收基线行若 probe 检查位变化则同步）

- [ ] **Step 1: 熵减检查**

```bash
cd /Volumes/TBU/Workspace/IntelHub-gev-p7 && grep -rn "hud-search-p5\|P5 待实现\|(P5)" console/src console/probe-gev.mjs; \
grep -rn "scopeMask\|VisualSettings\|LocationNavigation" console/src/ --include="*.ts" --include="*.tsx" | grep -v __tests__ | grep -v "setScopeMaskEnabled(false)"
```

Expected: 两组全空（除 application.ts 既有 setScopeMaskEnabled(false) 防御行）。无未使用导出、无注释掉的临时代码。

- [ ] **Step 2: 写 P7 ledger**（范围/决策/315+410 验收结果/已知边界：photon 上游选择及是否切换过、probe viewer 全局缺失的弱断言降级、follow 仅 flight+satellite）

- [ ] **Step 3: 合并 main + 清理 worktree**

```bash
cd /Volumes/TBU/Workspace/IntelHub-gev-p7 && git add -A && git commit -m "docs(ledger): GEV P7 终态" || true
cd /Volumes/TBU/Workspace/IntelHub && git merge --no-ff feat/gev-p7-camera-search && git worktree remove ../IntelHub-gev-p7 && git branch -d feat/gev-p7-camera-search
```

- [ ] **Step 4: 410 生产部署 + 验收**

rsync（exclude 清单同 Task 7 brief）→ **build-hub.sh + build-console.sh** → restart hub-core → probe + sp8/sp6/sp7/sp3（默认主机 IntelHub，BASE http://10.10.10.41:8800）。sp6 starlink flap 不阻塞，其余全绿才可 push。

- [ ] **Step 5: push**

```bash
cd /Volumes/TBU/Workspace/IntelHub && git push origin main
```
