# GEV P12 航班层 GEV 对等 实施计划

> 日期：2026-09-21 · 状态：草稿（待 user 确认执行方法）
> 上游 spec：`docs/superpowers/specs/2026-09-21-gev-p12-flight-layer-design.md`
> 上游总纲：`docs/superpowers/specs/2026-09-17-gev-engine-fusion-design.md`
> 参考实现：`/Volumes/TBU/Github/gods-eye-view`（server/providers/aircraft/ 4 文件 + src/layers/flights/ 14 文件）
> Worktree：`feat/p12-flight-layer-parity`（base: `cd07a7b`，main HEAD）

## 0. 范围与边界

**In scope**（5 类交付）：
- **A** hub-core `gev_enrichment.rs` — `/api/adsbdb/<type|route>/<id>` adsbdb proxy + 24h disk cache + dirty 15s flush
- **B** hub-core `gev_tracks.rs` — `/api/opensky-track?icao24=` + `/api/adsblol/trace?hex=` + OpenSky OAuth adaptive TTL + serve-stale
- **C** console `aircraft-source.ts` — 暴露 `getTrack` + `getEnrichment` 给 vendor `standalone.js`（URL 字面量守住）
- **D** cockpit 5 子模块 aggressive UX 抛光：键盘 / camera flyTo / briefing 节奏 / panel-drag 锁定 / viewport lock
- **E** 验收：sp6 +2 + sp8 +5 + probe P12_PROBES + i18n

**Out of scope**（spec §0）：
- **不 fork vendor engine 代码** —— 契约守卫守住
- **不写** Cesium 自定义 ribbon 渲染（vendor `tracking.js:467-560` + `rendering.js` 已自带）
- **不重写** vendor enrichment.js drip dispatch + `_enrichSeen` dedup
- **不做** GEV voice control / cockpitSignal / cloud-precipitation
- **不做** multi-monitor 边界处理 / iframe embed / Cesium WebGL fallback
- **不重做** P9 cockpit 5 个 adapter + 5 个 HUD 组件（保留全部）
- **不做** console 侧缓存（passthrough 到 hub-core disk cache + vendor dedup）

## 1. 总览

| Task | 内容 | 出口 | 文件 |
|---|---|---|---|
| **T1** | hub-core `gev_enrichment.rs` + 持久化 + route 注册 + 8 单测 | `cargo test --package hub-core gev_enrichment` 全绿（≥8 tests） | `hub-core/crates/hub-core/src/gev_enrichment.rs` + `lib.rs` + tests |
| **T2** | hub-core `gev_tracks.rs` (OpenSky OAuth + adaptive TTL + serve-stale) + 10 单测 | `cargo test --package hub-core gev_tracks` 全绿（≥10 tests） | `hub-core/crates/hub-core/src/gev_tracks.rs` + `lib.rs` + tests |
| **T3** | console `aircraft-source.ts` + re-export + gev-boot wiring + 16 单测 | vitest `aircraft-source.test.ts` 全绿 | `console/src/gev-adapters/aircraft-source.ts` + `index.ts` + `gev-boot/application.ts` |
| **T4** | source-contracts 路径字面量守卫 + 3 method 钉扎 | vitest `source-contracts.test.ts` 全绿（含新增 P12 项） | `console/src/gev-boot/__tests__/source-contracts.test.ts` |
| **T5** | cockpit `shortcuts.ts` + 挂载 HudCockpitFrame + i18n + 10 单测 | vitest `shortcuts.test.ts` 全绿 | `console/src/gev-visual/cockpit/shortcuts.ts` + `HudCockpitFrame.tsx` + `i18n/{zh,en}.json` |
| **T6** | cockpit `camera-transition.ts` + 时序协调 useEffect + 6 单测 | vitest `camera-transition.test.ts` 全绿 | `console/src/gev-visual/cockpit/camera-transition.ts` + `HudCockpitFrame.tsx` |
| **T7** | cockpit `viewport-lock.ts` + `panel-drag.ts` 加 disabled + 10 单测（5+5） | vitest 全绿 | `console/src/gev-visual/cockpit/viewport-lock.ts` + `gev-visual/tail/panel-drag.ts` + `HudCockpitFrame.tsx` |
| **T8** | briefing 节奏微调（manual grace 5s + fade CSS + 进度条）+ 3 单测 | vitest `HudCockpitBriefingPanel.test.tsx` 全绿 | `console/src/globe-hud/HudCockpitBriefingPanel.tsx` + `hud.css` |
| **T9** | 验收脚本扩展（sp6 +2 + sp8 +5 + probe P12_PROBES + i18n 验证）+ 315 验收 | sp6 41+5sh/0f + sp8 53+2sh/0f + probe exit 0 | `scripts/accept-sp6.py` + `scripts/accept-sp8.py` + `console/probe-gev.mjs` |
| **T10** | whole-branch review + ledger + AGENTS.md update + merge main + 410 build + 410 验收 + push | main @ P12 commit；410 全绿；pushed | `docs/superpowers/execution/2026-09-21-gev-p12-ledger.md` + `AGENTS.md` |

**Worktree 隔离**（在 spec 写之前已建好）：
```bash
cd /Volumes/TBU/Workspace/IntelHub
git worktree add ../IntelHub-p12 -b feat/p12-flight-layer-parity
sleep 4   # 网络盘同步
cd /Volumes/TBU/Workspace/IntelHub-p12
git log -1 main   # 确认 BASE = cd07a7b
```

---

## 2. 全局约束

| 约束 | 值 | 来源 |
|---|---|---|
| Worktree branch | `feat/p12-flight-layer-parity` | AGENTS.md |
| Base commit | `cd07a7b` (main HEAD) | git |
| 测试 VM | Debian-test (10.10.10.35) | AGENTS.md |
| 生产 VM | IntelHub (10.10.10.41) | AGENTS.md |
| hub-core restart 后等待 | ≥ 5 分钟 (avoid deploy-stampede) | AGENTS.md 2026-09-20 教训 |
| 验收 sp6 baseline | 39+5sh/0f → 41+5sh/0f (+2) | spec §7.1 |
| 验收 sp8 baseline | 48+2sh/0f → 53+2sh/0f (+5) | spec §7.2 |
| 验收 probe | 8 P12 probes | spec §7.3 |
| URL 路径 | vendor 直连 (`/api/opensky-track`, `/api/adsbdb/<kind>/<id>`) | spec §0 + §3 |
| Cache 路径 | `/var/lib/intelhub/adsbdb-cache.json` | spec §4.A |
| OpenSky OAuth | `OPENSKY_CLIENT_ID` + `OPENSKY_CLIENT_SECRET` env (core/secrets.env) | spec §4.B |
| 广告 cache TTL | 24h | spec §4.A |
| Track proxy cache TTL | 60s in-memory LRU 200 entries | spec §4.B |
| Response cap | 5MB | spec §4.B |
| Adapter timeout | 8s (matches vendor `tracking.js:484 AbortSignal.timeout(8000)`) | spec §5.1 |
| Test framework (console) | vitest 1.6+ | P11 precedent |
| Test framework (hub-core) | cargo test | existing |
| 风格 (Rust) | rustfmt + clippy 默认 | existing |
| 风格 (TS) | prettier 2-space + eslint react/recommended | existing |
| i18n | console/src/i18n/{zh,en}.json 双写 | P11 precedent |
| 私有命名空间 | `intelhub.*` (P10 决策：vendor `godsEyeView.*` 前缀留作 P13+ 整理) | AGENTS.md |
| vendor URL 字面量 | `/api/opensky-track?icao24=` + `/api/adsbdb/${query.kind}/${id}` 必须 source-contracts 守住 | spec §3 + §8 R3 |
| vendor import 风格 | bare `gev-engine/...` alias (相对路径禁止) | gev-boot/application.ts 注释 |

---

## 3. Review Focus

Spec 暗示但没有测试覆盖的输入类 / 失败模式，**最可能咬人**的 5 项：

| # | 输入/条件 | 期望行为 | 归属 task |
|---|---|---|---|
| RF1 | OpenSky `/tracks/all` 返 200 但 body 缺 `path` 字段 | adapter 返 `{records: [], complete: false}`（不抛错），vendor 走本地累积 trail | T2 + T3 |
| RF2 | 用户在 cockpit active 时 Ctrl+R 刷新浏览器 | localStorage `intelhub.cockpit.visionMode` 持久化；store.exit 不调用但 active=false 时仍恢复 mode（已有 D2 P9 实现） | T5 验证持久化 + T6 不主动调 store.exit |
| RF3 | adsbdb 返 200 但 `response.aircraft` 字段为 null（异常 schema） | parse_aircraft 返 None → cache None → 响应 `{found: false}`（不 throw） | T1 |
| RF4 | 短时间内连续 enter/exit cockpit（< 0.7s 飞行时间内按 Esc） | camera flyBackToBaseline 取消旧的（_activeFlyPromise 维护），AbortError silent | T6 |
| RF5 | OpenSky OAuth token 恰好过期临界（59s expiry margin 内并发请求） | 第二个 in-flight refresh 复用第一个 promise，不重复拿 token | T2 |

---

## 4. 文件结构

### 新建文件

```
hub-core/crates/hub-core/src/
├── gev_enrichment.rs                            (~250 LoC) — adsbdb proxy + 24h disk cache
└── gev_tracks.rs                                (~350 LoC) — OpenSky OAuth + track backfill proxy

hub-core/crates/hub-core/tests/
├── gev_enrichment.rs                            (~150 LoC) — 8 集成测试
└── gev_tracks.rs                                (~180 LoC) — 10 集成测试

console/src/gev-visual/cockpit/
├── shortcuts.ts                                 (~120 LoC) — 键盘快捷键 hook
├── camera-transition.ts                         (~80 LoC) — Cesium flyTo 包壳
└── viewport-lock.ts                             (~50 LoC) — Cesium input 拦截

console/src/gev-visual/cockpit/__tests__/
├── shortcuts.test.ts                            (~120 LoC)
├── camera-transition.test.ts                    (~80 LoC)
└── viewport-lock.test.ts                        (~50 LoC)

console/src/gev-adapters/
├── aircraft-source.ts                           (~120 LoC) — source 对象（getSnapshot/getTrack/getEnrichment）
└── __tests__/aircraft-source.test.ts            (~180 LoC) — 16 测试 case

console/src/globe-hud/__tests__/
├── HudCockpitBriefingPanel.test.tsx             (~30 LoC 新增) — briefing 节奏微调
└── panel-drag.test.ts                           (~60 LoC 新增) — disabled 参数

scripts/
├── accept-sp6.py                                (~30 LoC 新增) — check_42/check_43
└── accept-sp8.py                                (~50 LoC 新增) — check_49-53

docs/superpowers/execution/
└── 2026-09-21-gev-p12-ledger.md                 (T10 创建)
```

### 修改文件

```
hub-core/crates/hub-core/src/
├── lib.rs                                       (~5 行) — 注册 gev_enrichment + gev_tracks
└── api.rs                                       (~30 行) — 4 个新 route 入口

console/src/gev-adapters/
└── index.ts                                     (~10 行) — re-export aircraft-source

console/src/gev-boot/
├── application.ts                               (~10 行) — 注入 aircraft source
└── __tests__/source-contracts.test.ts           (~30 行) — 3 method 钉扎 + URL 字面量守卫

console/src/gev-visual/cockpit/
├── index.ts                                     (~5 行) — re-export shortcuts/camera-transition/viewport-lock
└── cockpit-store.ts                             (~5 行) — 加 `hidden` 字段（Shift+C 隐藏功能）

console/src/globe-hud/
├── HudCockpitFrame.tsx                          (~30 行) — 挂 useCockpitShortcuts + camera-transition + viewport-lock
└── HudCockpitBriefingPanel.tsx                  (~30 行) — manual grace 5s + fade + 进度条

console/src/gev-visual/tail/
└── panel-drag.ts                                (~80 行) — 加 disabled 参数 + setDisabled method

console/probe-gev.mjs                            (~40 行) — P12_PROBES 8 个
console/src/hud.css                              (~30 行) — cockpit shortcut hint + briefing fade + progress bar
console/src/i18n/{zh,en}.json                    (~10 行/语言) — cockpit.shortcut.hint
core/secrets.env                                 (~3 行) — OPENSKY_CLIENT_ID/SECRET 占位
AGENTS.md                                        (~2 行) — sp6 +2 / sp8 +5 baseline 更新
```

### 文件责任边界

| 文件 | 单一职责 |
|---|---|
| `gev_enrichment.rs` | adsbdb API 代理 + 24h 磁盘缓存 + 并发 coalesce |
| `gev_tracks.rs` | OpenSky OAuth + 自适应 TTL + serve-stale + adsb.lol trace |
| `aircraft-source.ts` | vendor source 对象契约 + URL 字面量 |
| `shortcuts.ts` | keyboard event → store/briefing/vision 路由 |
| `camera-transition.ts` | Cesium camera.flyTo 包壳 + baseline capture/restore |
| `viewport-lock.ts` | Cesium input 拦截 + cursor 控制 |
| `panel-drag.ts` | 通用 panel 拖拽（已有，加 disabled 参数） |

---

## 5. 任务详情

### Task 1 — hub-core `gev_enrichment.rs` + 8 集成测试

**Files**：
1. `hub-core/crates/hub-core/src/gev_enrichment.rs`（新）
2. `hub-core/crates/hub-core/src/lib.rs`（改）—— 加 `pub mod gev_enrichment;`
3. `hub-core/crates/hub-core/tests/gev_enrichment.rs`（新）

**Interfaces**：
- Consumes: `AppState` (injected cache + http client)
- Produces:
  ```rust
  pub async fn handle_route(req: Request, state: AppState) -> Result<Response, HubError>;
  pub struct EnrichmentCache { /* routes + aircraft HashMap + dirty + inflight */ }
  pub fn parse_route(json: &serde_json::Value) -> Option<RouteData>;
  pub fn parse_aircraft(json: &serde_json::Value) -> Option<AircraftData>;
  ```

**Steps**：

- [ ] **Step 1: 写 cache 模块测试（failing）**

```rust
// hub-core/crates/hub-core/tests/gev_enrichment.rs
use hub_core::gev_enrichment::{EnrichmentCache, parse_route, parse_aircraft};
use std::collections::HashMap;

#[test]
fn cache_load_persist_roundtrip() {
    let tmp = tempdir_unique("adsbdb-cache");
    let mut cache = EnrichmentCache::new();
    cache.routes.insert("UAL123".to_string(), CachedRoute { at: now_ms(), data: Some(route_fixture()) });
    cache.aircraft.insert("4ca9b1".to_string(), CachedAircraft { at: now_ms(), data: Some(aircraft_fixture()) });
    cache.dirty.store(true, std::sync::atomic::Ordering::Relaxed);
    cache.persist_to(&tmp).unwrap();
    let loaded = EnrichmentCache::load_from(&tmp).unwrap();
    assert_eq!(loaded.routes.len(), 1);
    assert_eq!(loaded.aircraft.len(), 1);
    assert_eq!(loaded.routes["UAL123"].data.as_ref().unwrap().airline, "United Airlines");
}

#[test]
fn cache_negative_404_stores_none() {
    let mut cache = EnrichmentCache::new();
    cache.aircraft.insert("abc123".to_string(), CachedAircraft { at: now_ms(), data: None });
    let json = cache.aircraft_response("abc123");
    assert_eq!(json["found"], false);
}

#[test]
fn cache_fresh_ttl_24h() {
    let mut cache = EnrichmentCache::new();
    let old_at = now_ms() - (25 * 3600 * 1000); // 25h ago
    cache.aircraft.insert("4ca9b1".to_string(), CachedAircraft { at: old_at, data: Some(aircraft_fixture()) });
    assert!(!cache.is_fresh(&cache.aircraft["4ca9b1"]));
}
```

- [ ] **Step 2: 跑测试验失败**

Run: `cd hub-core && cargo test --package hub-core --test gev_enrichment --no-run 2>&1 | head -20`
Expected: FAIL with `unresolved import hub_core::gev_enrichment`

- [ ] **Step 3: 实现 cache 模块（最小通过 Step 1）**

```rust
// hub-core/crates/hub-core/src/gev_enrichment.rs
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use serde::{Serialize, Deserialize};

const TTL_MS: u64 = 24 * 3600 * 1000;
pub const CACHE_PATH: &str = "/var/lib/intelhub/adsbdb-cache.json";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AirportData {
    pub code: String,
    pub name: String,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RouteData {
    pub airline: Option<String>,
    pub origin: AirportData,
    pub destination: AirportData,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AircraftData {
    pub type_code: Option<String>,
    pub type_name: Option<String>,
    pub registration: Option<String>,
}

#[derive(Clone, Debug)]
pub struct CachedRoute {
    pub at: u64,
    pub data: Option<RouteData>, // None = negative cache (404)
}

#[derive(Clone, Debug)]
pub struct CachedAircraft {
    pub at: u64,
    pub data: Option<AircraftData>,
}

pub struct EnrichmentCache {
    pub routes: HashMap<String, CachedRoute>,
    pub aircraft: HashMap<String, CachedAircraft>,
    pub dirty: AtomicBool,
}

impl EnrichmentCache {
    pub fn new() -> Self {
        Self { routes: HashMap::new(), aircraft: HashMap::new(), dirty: AtomicBool::new(false) }
    }
    
    pub fn is_fresh(&self, entry: &impl FreshCheck) -> bool { entry.is_fresh() }
    
    pub fn load_from(path: &std::path::Path) -> Result<Self, std::io::Error> {
        let s = std::fs::read_to_string(path)?;
        let p: PersistedCache = serde_json::from_str(&s).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        Ok(Self {
            routes: p.routes.into_iter().map(|(k, v)| (k, CachedRoute { at: v.at, data: v.data })).collect(),
            aircraft: p.aircraft.into_iter().map(|(k, v)| (k, CachedAircraft { at: v.at, data: v.data })).collect(),
            dirty: AtomicBool::new(false),
        })
    }
    
    pub fn persist_to(&self, path: &std::path::Path) -> Result<(), std::io::Error> {
        let p = PersistedCache {
            routes: self.routes.iter().map(|(k, v)| (k.clone(), PersistedEntry { at: v.at, data: v.data.clone() })).collect(),
            aircraft: self.aircraft.iter().map(|(k, v)| (k.clone(), PersistedEntry { at: v.at, data: v.data.clone() })).collect(),
        };
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(&p)?)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }
    
    pub fn aircraft_response(&self, hex: &str) -> serde_json::Value {
        match self.aircraft.get(hex) {
            Some(CachedAircraft { data: Some(d), .. }) => serde_json::json!({
                "found": true, "typeCode": d.type_code, "typeName": d.type_name, "registration": d.registration
            }),
            Some(CachedAircraft { data: None, .. }) => serde_json::json!({ "found": false }),
            None => serde_json::json!({ "found": false }),
        }
    }
}

trait FreshCheck { fn is_fresh(&self) -> bool; }
impl FreshCheck for CachedRoute { fn is_fresh(&self) -> bool { now_ms().saturating_sub(self.at) < TTL_MS } }
impl FreshCheck for CachedAircraft { fn is_fresh(&self) -> bool { now_ms().saturating_sub(self.at) < TTL_MS } }

#[derive(Serialize, Deserialize)]
struct PersistedCache {
    routes: HashMap<String, PersistedEntry<RouteData>>,
    aircraft: HashMap<String, PersistedEntry<AircraftData>>,
}

#[derive(Serialize, Deserialize)]
struct PersistedEntry<T> { at: u64, data: Option<T> }

pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64
}

pub fn parse_route(json: &serde_json::Value) -> Option<RouteData> {
    let fr = json.get("response")?.get("flightroute")?.as_object()?;
    if fr.get("origin").map(|v| v.is_null()).unwrap_or(true) { return None; }
    if fr.get("destination").map(|v| v.is_null()).unwrap_or(true) { return None; }
    let airport = |a: &serde_json::Value| AirportData {
        code: a.get("iata_code").or_else(|| a.get("icao_code")).and_then(|v| v.as_str()).unwrap_or_default().to_string(),
        name: a.get("municipality").or_else(|| a.get("name")).and_then(|v| v.as_str()).unwrap_or_default().to_string(),
        lat: a.get("latitude").and_then(|v| v.as_f64()).filter(|n| n.is_finite()),
        lon: a.get("longitude").and_then(|v| v.as_f64()).filter(|n| n.is_finite()),
    };
    Some(RouteData {
        airline: fr.get("airline").and_then(|a| a.get("name")).and_then(|v| v.as_str()).map(String::from),
        origin: airport(fr.get("origin").unwrap()),
        destination: airport(fr.get("destination").unwrap()),
    })
}

pub fn parse_aircraft(json: &serde_json::Value) -> Option<AircraftData> {
    let a = json.get("response")?.get("aircraft")?.as_object()?;
    let type_code = a.get("icao_type").and_then(|v| v.as_str()).map(String::from);
    let manufacturer = a.get("manufacturer").and_then(|v| v.as_str());
    let type_name_only = a.get("type").and_then(|v| v.as_str());
    let type_name = match (manufacturer, type_name_only) {
        (Some(m), Some(t)) => Some(format!("{} {}", m, t)),
        (None, Some(t)) => Some(t.to_string()),
        _ => None,
    };
    let registration = a.get("registration").and_then(|v| v.as_str()).map(String::from);
    Some(AircraftData { type_code, type_name, registration })
}

pub fn route_response(data: Option<&RouteData>) -> serde_json::Value {
    match data {
        Some(d) => serde_json::json!({
            "found": true,
            "airline": d.airline,
            "origin": { "code": d.origin.code, "name": d.origin.name, "lat": d.origin.lat, "lon": d.origin.lon },
            "destination": { "code": d.destination.code, "name": d.destination.name, "lat": d.destination.lat, "lon": d.destination.lon },
        }),
        None => serde_json::json!({ "found": false }),
    }
}

// Helpers
fn route_fixture() -> RouteData {
    RouteData {
        airline: Some("United Airlines".to_string()),
        origin: AirportData { code: "KEWR".to_string(), name: "Newark".to_string(), lat: Some(40.69), lon: Some(-74.17) },
        destination: AirportData { code: "KSFO".to_string(), name: "San Francisco".to_string(), lat: Some(37.62), lon: Some(-122.38) },
    }
}
fn aircraft_fixture() -> AircraftData {
    AircraftData { type_code: Some("B738".to_string()), type_name: Some("Boeing 737-800".to_string()), registration: Some("EI-DCL".to_string()) }
}
fn tempdir_unique(name: &str) -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("intelhub-test-{}-{}", name, now_ms()));
    std::fs::create_dir_all(&p).unwrap();
    p
}
```

- [ ] **Step 4: 跑测试验通过**

Run: `cd hub-core && cargo test --package hub-core --test gev_enrichment 2>&1 | tail -15`
Expected: 3 tests passed

- [ ] **Step 5: 加 route handler + inflight coalesce + 5 集成测试**

```rust
// In gev_enrichment.rs add:
use std::sync::Arc;
use tokio::sync::{RwLock, Mutex};
use std::collections::HashMap as Map;

pub async fn handle_route(req: Request, state: AppState) -> Result<Response, HubError> {
    let path = req.path().to_string();
    if let Some(hex) = path.strip_prefix("/api/adsbdb/type/") {
        return handle_type(state, hex).await;
    }
    if let Some(cs) = path.strip_prefix("/api/adsbdb/route/") {
        return handle_route_type(state, cs).await;
    }
    Err(HubError::not_found("unknown adsbdb endpoint"))
}

async fn handle_type(state: AppState, hex: &str) -> Result<Response, HubError> {
    let hex = hex.to_lowercase();
    let hex6_re = regex::Regex::new(r"^[0-9a-f]{6}$").unwrap();
    if !hex6_re.is_match(&hex) {
        return Ok(json_error(400, "invalid hex"));
    }
    // Cache hit
    if let Some(entry) = state.enrichment.aircraft.get(&hex) {
        if entry.is_fresh() {
            return Ok(json_response(state.enrichment.aircraft_response(&hex)));
        }
    }
    // In-flight coalesce
    let key = format!("type:{}", hex);
    let _guard = state.enrichment.inflight.lock().await.entry(key.clone()).or_insert_with(|| Arc::new(tokio::sync::OnceCell::new())).clone();
    // Upstream fetch
    let url = format!("https://api.adsbdb.com/v0/aircraft/{}", hex);
    let res = state.http.get(&url).timeout(8000).send().await.map_err(|e| HubError::upstream(e))?;
    if res.status() == 404 {
        state.enrichment.aircraft.insert(hex, CachedAircraft { at: now_ms(), data: None });
        state.enrichment.dirty.store(true, Ordering::Relaxed);
        return Ok(json_response(serde_json::json!({ "found": false })));
    }
    if !res.status().is_success() {
        return Ok(json_error(503, "adsbdb upstream unavailable"));
    }
    let body: serde_json::Value = res.json().await?;
    let data = parse_aircraft(&body);
    state.enrichment.aircraft.insert(hex.clone(), CachedAircraft { at: now_ms(), data: data.clone() });
    state.enrichment.dirty.store(true, Ordering::Relaxed);
    Ok(json_response(state.enrichment.aircraft_response(&hex)))
}
```

```rust
// In tests/gev_enrichment.rs add 5 more tests:
#[tokio::test]
async fn invalid_hex_returns_400() {
    let state = make_test_state();
    let req = Request::fake_get("/api/adsbdb/type/xyz");
    let res = handle_route(req, state).await.unwrap();
    assert_eq!(res.status, 400);
}

#[tokio::test]
async fn inflight_coalesce() {
    let state = make_test_state_with_mock_http(mock_adsbdb_once());
    let reqs: Vec<_> = (0..10).map(|_| Request::fake_get("/api/adsbdb/type/4ca9b1")).collect();
    let responses: Vec<_> = futures::future::join_all(reqs.into_iter().map(|r| handle_route(r, state.clone()))).await;
    // 只 1 个上游请求被 mock 接受
    assert!(responses.iter().all(|r| r.as_ref().unwrap().status == 200));
    assert_eq!(state.enrichment.upstream_calls(), 1);
}

#[tokio::test]
async fn upstream_5xx_returns_503() {
    let state = make_test_state_with_mock_http(mock_adsbdb_500());
    let req = Request::fake_get("/api/adsbdb/type/4ca9b1");
    let res = handle_route(req, state).await.unwrap();
    assert_eq!(res.status, 503);
}

#[test]
fn parse_route_minimal() {
    let json = serde_json::json!({
        "response": {
            "flightroute": {
                "airline": { "name": "UA" },
                "origin": { "iata_code": "KEWR", "municipality": "Newark", "latitude": 40.69, "longitude": -74.17 },
                "destination": { "iata_code": "KSFO", "municipality": "San Francisco", "latitude": 37.62, "longitude": -122.38 }
            }
        }
    });
    let r = parse_route(&json).unwrap();
    assert_eq!(r.airline.as_deref(), Some("UA"));
    assert_eq!(r.origin.code, "KEWR");
    assert_eq!(r.destination.code, "KSFO");
}

#[test]
fn parse_aircraft_minimal() {
    let json = serde_json::json!({
        "response": {
            "aircraft": {
                "icao_type": "B738",
                "manufacturer": "Boeing",
                "type": "737-800",
                "registration": "EI-DCL"
            }
        }
    });
    let a = parse_aircraft(&json).unwrap();
    assert_eq!(a.type_code.as_deref(), Some("B738"));
    assert_eq!(a.type_name.as_deref(), Some("Boeing 737-800"));
    assert_eq!(a.registration.as_deref(), Some("EI-DCL"));
}
```

- [ ] **Step 6: lib.rs 注册 + 启动加载 + dirty 15s flush**

```rust
// hub-core/crates/hub-core/src/lib.rs
pub mod gev_enrichment;
```

```rust
// In gev_enrichment.rs add:
pub async fn start(state: AppState) -> Result<(), HubError> {
    if let Ok(cache) = EnrichmentCache::load_from(std::path::Path::new(CACHE_PATH)) {
        state.enrichment.routes = cache.routes;
        state.enrichment.aircraft = cache.aircraft;
    }
    // dirty 标记 + 15s 刷盘
    let cache_weak = state.enrichment.weak_handle();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(15));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        interval.tick().await; // skip immediate
        loop {
            interval.tick().await;
            if cache_weak.dirty.swap(false, Ordering::Relaxed) {
                let _ = cache_weak.persist_to(std::path::Path::new(CACHE_PATH)).await;
            }
        }
    });
    Ok(())
}
```

- [ ] **Step 7: api.rs 注册 route**

```rust
// hub-core/crates/hub-core/src/api.rs
.route("/api/adsbdb/type/:hex", get(gev_enrichment::handle_type_route))
.route("/api/adsbdb/route/:callsign", get(gev_enrichment::handle_route_type_route))
```

(具体 router 写法取决于现有 axum / actix 风格 —— **Step 7 实施者读 api.rs 现有 route 注册模式后照搬**)

- [ ] **Step 8: 跑全部 8 测试 + commit**

Run: `cd hub-core && cargo test --package hub-core gev_enrichment 2>&1 | tail -20`
Expected: 8 tests passed

```bash
git add hub-core/crates/hub-core/src/gev_enrichment.rs hub-core/crates/hub-core/src/lib.rs hub-core/crates/hub-core/src/api.rs hub-core/crates/hub-core/tests/gev_enrichment.rs
git commit -m "feat(gev-p12-t1): hub-core gev_enrichment — adsbdb proxy + 24h disk cache + dirty flush"
```

**Verify**：
- `cargo test --package hub-core gev_enrichment` 全绿（8 tests）
- `cargo build --release --workspace` rc=0
- vendor 0 行改动
- `/var/lib/intelhub/adsbdb-cache.json` 文件存在 + JSON 有效

---

### Task 2 — hub-core `gev_tracks.rs` (OpenSky OAuth + adaptive TTL) + 10 集成测试

**Files**：
1. `hub-core/crates/hub-core/src/gev_tracks.rs`（新）
2. `hub-core/crates/hub-core/src/lib.rs`（改）—— 加 `pub mod gev_tracks;`
3. `hub-core/crates/hub-core/tests/gev_tracks.rs`（新）

**Interfaces**：
- Consumes: `AppState` (injected cache + http client + env)
- Produces:
  ```rust
  pub async fn handle_route(req: Request, state: AppState) -> Result<Response, HubError>;
  pub struct OpenSkyClient { /* http, token, expiry, inflight_refresh, adaptive_ttl, cooldown_until */ }
  pub fn adaptive_ttl(remaining: Option<u64>) -> u64;
  pub fn parse_retry_after_secs(v: &str, now_ms: u64) -> Option<u64>;
  pub fn normalize_track_path(path: &[serde_json::Value]) -> Vec<TrackRecord>;
  ```

**Steps**：

- [ ] **Step 1: 写 adaptive_ttl + parse_retry_after 测试（failing）**

```rust
// hub-core/crates/hub-core/tests/gev_tracks.rs
use hub_core::gev_tracks::{adaptive_ttl, parse_retry_after_secs};

#[test]
fn adaptive_ttl_4_tiers() {
    assert_eq!(adaptive_ttl(Some(3000)), 9000);
    assert_eq!(adaptive_ttl(Some(2000)), 30_000);
    assert_eq!(adaptive_ttl(Some(800)), 90_000);
    assert_eq!(adaptive_ttl(Some(200)), 300_000);
    assert_eq!(adaptive_ttl(None), 9000);
}

#[test]
fn parse_retry_after_secs_delta() {
    let now = 1_000_000_000_000u64; // epoch ms
    assert_eq!(parse_retry_after_secs("30", now), Some(30_000));
    let future = ((now / 1000) + 60).to_string();
    assert_eq!(parse_retry_after_secs(&future, now), Some(60_000));
    assert_eq!(parse_retry_after_secs("invalid", now), None);
}
```

- [ ] **Step 2: 实现 adaptive_ttl + parse_retry_after_secs**

```rust
// hub-core/crates/hub-core/src/gev_tracks.rs
const OPENSKY_CACHE_MS: u64 = 9000;
const TRACK_CACHE_MS: u64 = 60_000;
const TRACK_CACHE_MAX: usize = 200;
const RESPONSE_CAP_BYTES: usize = 5 * 1024 * 1024;
const COOLDOWN_MIN_MS: u64 = 5_000;
const COOLDOWN_MAX_MS: u64 = 30 * 60 * 1000;

pub fn adaptive_ttl(remaining: Option<u64>) -> u64 {
    let r = remaining.unwrap_or(u64::MAX);
    if r > 2400 { OPENSKY_CACHE_MS }
    else if r > 1200 { 30_000 }
    else if r > 400 { 90_000 }
    else { 300_000 }
}

pub fn parse_retry_after_secs(v: &str, now_ms: u64) -> Option<u64> {
    if let Ok(secs) = v.parse::<u64>() {
        return Some(secs.saturating_mul(1000));
    }
    if let Ok(epoch) = v.parse::<u64>() {
        return Some(epoch.saturating_sub(now_ms / 1000).saturating_mul(1000));
    }
    None
}
```

- [ ] **Step 3: 跑测试验通过**

Run: `cd hub-core && cargo test --package hub-core --test gev_tracks 2>&1 | tail -10`
Expected: 2 tests passed

- [ ] **Step 4: 实现 OpenSkyClient (OAuth + adaptive TTL + cooldown + cache)**

```rust
// In gev_tracks.rs add:
use std::sync::Arc;
use tokio::sync::{RwLock, Mutex};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

pub struct OpenSkyClient {
    pub http: reqwest::Client,
    pub token: Arc<RwLock<Option<String>>>,
    pub token_expiry: Arc<RwLock<u64>>,
    pub inflight_refresh: Arc<Mutex<Option<Arc<tokio::sync::OnceCell<String>>>>>,
    pub adaptive_ttl_ms: AtomicU64,
    pub cooldown_until: AtomicU64,
}

impl OpenSkyClient {
    pub fn new(http: reqwest::Client) -> Self {
        Self {
            http,
            token: Arc::new(RwLock::new(None)),
            token_expiry: Arc::new(RwLock::new(0)),
            inflight_refresh: Arc::new(Mutex::new(None)),
            adaptive_ttl_ms: AtomicU64::new(OPENSKY_CACHE_MS),
            cooldown_until: AtomicU64::new(0),
        }
    }
    
    pub async fn get_token(&self) -> Result<Option<String>, HubError> {
        let now = crate::gev_enrichment::now_ms();
        {
            let token = self.token.read().await;
            let expiry = self.token_expiry.read().await;
            if token.is_some() && now < expiry.saturating_sub(60_000) {
                return Ok(token.clone());
            }
        }
        // In-flight coalesce
        let mut guard = self.inflight_refresh.lock().await;
        if let Some(once) = guard.as_ref() {
            let result = once.get_or_init(|| async {
                self._do_refresh().await
            }).await;
            return result.clone();
        }
        let once = Arc::new(tokio::sync::OnceCell::new());
        *guard = Some(once.clone());
        drop(guard);
        let result = self._do_refresh().await;
        once.set(result.clone()).ok();
        *self.inflight_refresh.lock().await = None;
        Ok(result)
    }
    
    async fn _do_refresh(&self) -> Option<String> {
        let client_id = std::env::var("OPENSKY_CLIENT_ID").ok();
        let client_secret = std::env::var("OPENSKY_CLIENT_SECRET").ok();
        let (Some(cid), Some(sec)) = (client_id, client_secret) else { return None; };
        let res = self.http.post("https://auth.opensky-network.org/auth/realms/opensky-network/protocol/openid-connect/token")
            .form(&[("grant_type", "client_credentials"), ("client_id", cid.as_str()), ("client_secret", sec.as_str())])
            .send().await.ok()?;
        let body: serde_json::Value = res.json().await.ok()?;
        let token = body["access_token"].as_str().map(String::from);
        let expires_in = body["expires_in"].as_u64().unwrap_or(1800);
        if let Some(ref t) = token {
            *self.token.write().await = Some(t.clone());
            *self.token_expiry.write().await = crate::gev_enrichment::now_ms() + expires_in * 1000;
        }
        token
    }
}
```

- [ ] **Step 5: 实现 handle_route (cache + cooldown + serve-stale + LRU eviction)**

```rust
// In gev_tracks.rs add:
pub async fn handle_opensky_track(req: Request, state: AppState) -> Result<Response, HubError> {
    let query = parse_query(&req);
    let icao24 = query.get("icao24").map(|s| s.to_lowercase()).unwrap_or_default();
    let hex_re = regex::Regex::new(r"^[0-9a-f]{6}$").unwrap();
    if !hex_re.is_match(&icao24) {
        return Ok(json_error(400, "icao24 must be 6-char hex"));
    }
    
    // Cache hit
    if let Some(entry) = state.tracks.cache.lock().await.get(&icao24) {
        if crate::gev_enrichment::now_ms() - entry.at < TRACK_CACHE_MS {
            return Ok(track_response(200, &entry.body, "HIT"));
        }
    }
    
    // Cooldown check
    let now = crate::gev_enrichment::now_ms();
    if now < state.tracks.opensky.cooldown_until.load(Ordering::Relaxed) {
        if let Some(entry) = state.tracks.cache.lock().await.get(&icao24) {
            return Ok(track_response(200, &entry.body, "STALE"));
        }
        return Ok(json_error(503, "opensky upstream cooling down"));
    }
    
    // Upstream fetch
    let token = state.tracks.opensky.get_token().await?;
    if token.is_none() {
        return Ok(json_error(503, "OPENSKY_CLIENT_ID/SECRET not configured"));
    }
    let url = format!("https://opensky-network.org/api/tracks/all?icao24={}&time=0", icao24);
    let res = state.tracks.opensky.http.get(&url)
        .bearer_auth(token.unwrap())
        .timeout(Duration::from_secs(12))
        .send().await?;
    
    // Adaptive TTL from X-Rate-Limit-Remaining
    let remaining = res.headers().get("X-Rate-Limit-Remaining")
        .and_then(|v| v.to_str().ok()).and_then(|s| s.parse::<u64>().ok());
    let new_ttl = adaptive_ttl(remaining);
    state.tracks.opensky.adaptive_ttl_ms.store(new_ttl, Ordering::Relaxed);
    
    if res.status() == 429 {
        let retry_after = res.headers().get("retry-after")
            .and_then(|v| v.to_str().ok()).and_then(|v| parse_retry_after_secs(v, now))
            .unwrap_or(30_000);
        let cooldown = retry_after.clamp(COOLDOWN_MIN_MS, COOLDOWN_MAX_MS);
        state.tracks.opensky.cooldown_until.store(now + cooldown, Ordering::Relaxed);
        if let Some(entry) = state.tracks.cache.lock().await.get(&icao24) {
            return Ok(track_response(200, &entry.body, "STALE"));
        }
        return Ok(json_error(429, "rate limited"));
    }
    
    if !res.status().is_success() {
        return Ok(json_error(res.status().as_u16(), &format!("track source HTTP {}", res.status().as_u16())));
    }
    
    // Cap response body
    let body_bytes = res.bytes().await?;
    if body_bytes.len() > RESPONSE_CAP_BYTES {
        return Ok(json_error(502, "track response too large"));
    }
    
    let body_str = String::from_utf8_lossy(&body_bytes).to_string();
    let mut cache = state.tracks.cache.lock().await;
    cache.insert(icao24.clone(), CachedTrack { at: now, body: body_str.clone() });
    if cache.len() > TRACK_CACHE_MAX {
        if let Some((oldest_key, _)) = cache.iter().min_by_key(|(_, v)| v.at).map(|(k, v)| (k.clone(), v.at)) {
            cache.remove(&oldest_key);
        }
    }
    Ok(track_response(200, &body_str, "MISS"))
}

pub fn normalize_track_path(path: &[serde_json::Value]) -> Vec<TrackRecord> {
    let mut records = Vec::new();
    for waypoint in path {
        if let Some(arr) = waypoint.as_array() {
            if arr.len() < 3 { continue; }
            let time = arr[0].as_f64();
            let lat = arr[1].as_f64();
            let lon = arr[2].as_f64();
            if !time.is_some_and(|n| n.is_finite()) || !lat.is_some_and(|n| n.is_finite()) || !lon.is_some_and(|n| n.is_finite()) {
                continue;
            }
            records.push(TrackRecord {
                observed_at_ms: (time.unwrap() * 1000.0) as u64,
                latitude: lat.unwrap(),
                longitude: lon.unwrap(),
                baro_altitude_m: arr.get(3).and_then(|v| v.as_f64()).filter(|n| n.is_finite()),
                course_deg: arr.get(4).and_then(|v| v.as_f64()).filter(|n| n.is_finite()),
                on_ground: arr.get(5).and_then(|v| v.as_bool()).unwrap_or(false),
            });
        }
    }
    records
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct TrackRecord {
    pub observed_at_ms: u64,
    pub latitude: f64,
    pub longitude: f64,
    pub baro_altitude_m: Option<f64>,
    pub course_deg: Option<f64>,
    pub on_ground: bool,
}
```

- [ ] **Step 6: 加 6 集成测试**

```rust
// In tests/gev_tracks.rs add:
#[tokio::test]
async fn cooldown_429_honors_retry_after() {
    let state = make_test_state_with_mock_opensky_429("30");
    let req = Request::fake_get("/api/opensky-track?icao24=4ca9b1");
    let _res = handle_route(req, state.clone()).await.unwrap();
    let cooldown = state.tracks.opensky.cooldown_until.load(Ordering::Relaxed);
    let expected = crate::gev_enrichment::now_ms() + 30_000;
    assert!((cooldown - expected).abs() < 1000);
}

#[tokio::test]
async fn cooldown_429_clamps_max_30min() {
    let state = make_test_state_with_mock_opensky_429("7200"); // 2h
    let req = Request::fake_get("/api/opensky-track?icao24=4ca9b1");
    let _res = handle_route(req, state.clone()).await.unwrap();
    let cooldown = state.tracks.opensky.cooldown_until.load(Ordering::Relaxed);
    assert!(cooldown - crate::gev_enrichment::now_ms() <= 30 * 60 * 1000);
}

#[tokio::test]
async fn serve_stale_during_cooldown() {
    let state = make_test_state_with_cache("4ca9b1", "{\"path\":[]}");
    state.tracks.opensky.cooldown_until.store(crate::gev_enrichment::now_ms() + 60_000, Ordering::Relaxed);
    let req = Request::fake_get("/api/opensky-track?icao24=4ca9b1");
    let res = handle_route(req, state.clone()).await.unwrap();
    assert_eq!(res.status, 200);
    assert_eq!(res.headers.get("X-ADS-B-Cache").unwrap(), "STALE");
}

#[tokio::test]
async fn cache_lru_eviction() {
    let state = make_test_state_with_empty_cache();
    state.tracks.cache.lock().await.insert("aaaaaa".to_string(), CachedTrack { at: 100, body: "{}".to_string() });
    // Fill to TRACK_CACHE_MAX
    for i in 0..TRACK_CACHE_MAX {
        let hex = format!("{:06x}", i);
        state.tracks.cache.lock().await.insert(hex, CachedTrack { at: i as u64, body: "{}".to_string() });
    }
    // Add one more — should evict "aaaaaa" (oldest at=100)
    state.tracks.cache.lock().await.insert("ffffff".to_string(), CachedTrack { at: 999_999, body: "{}".to_string() });
    assert!(state.tracks.cache.lock().await.get("aaaaaa").is_none());
    assert!(state.tracks.cache.lock().await.get("ffffff").is_some());
}

#[tokio::test]
async fn response_cap_5mb() {
    // Mock 6MB body
    let state = make_test_state_with_mock_opensky_huge(6 * 1024 * 1024);
    let req = Request::fake_get("/api/opensky-track?icao24=4ca9b1");
    let res = handle_route(req, state).await.unwrap();
    assert_eq!(res.status, 502);
}

#[tokio::test]
async fn env_missing_returns_503() {
    std::env::remove_var("OPENSKY_CLIENT_ID");
    std::env::remove_var("OPENSKY_CLIENT_SECRET");
    let state = make_test_state();
    let req = Request::fake_get("/api/opensky-track?icao24=4ca9b1");
    let res = handle_route(req, state).await.unwrap();
    assert_eq!(res.status, 503);
}

#[test]
fn normalizes_track_path() {
    let path = vec![
        serde_json::json!([1726845215.0, 40.69, -74.17, 10500.0, 91.0, false]),
        serde_json::json!([1726845300.0, 40.70, -74.18, 10600.0, 92.0, false]),
    ];
    let records = normalize_track_path(&path);
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].observed_at_ms, 1726845215000);
    assert!((records[0].latitude - 40.69).abs() < 0.01);
    assert_eq!(records[1].baro_altitude_m, Some(10600.0));
}
```

- [ ] **Step 7: adsb.lol trace handler + 2 测试**

```rust
// In gev_tracks.rs add:
pub async fn handle_adsblol_trace(req: Request, state: AppState) -> Result<Response, HubError> {
    let query = parse_query(&req);
    let hex = query.get("hex").map(|s| s.to_lowercase()).unwrap_or_default();
    let hex_re = regex::Regex::new(r"^[0-9a-f~]{6,7}$").unwrap();
    if !hex_re.is_match(&hex) {
        return Ok(json_error(400, "hex must be 6-7 char"));
    }
    // Cache hit
    if let Some(entry) = state.tracks.adsblol_cache.lock().await.get(&hex) {
        if crate::gev_enrichment::now_ms() - entry.at < TRACK_CACHE_MS {
            return Ok(track_response(200, &entry.body, "HIT"));
        }
    }
    let suffix = &hex[hex.len()-2..];
    let url = format!("https://adsb.lol/data/traces/{}/trace_full_{}.json", suffix, hex);
    let res = state.http.get(&url)
        .header("User-Agent", "IntelHub/dev")
        .timeout(Duration::from_secs(12))
        .send().await?;
    if !res.status().is_success() {
        return Ok(json_error(res.status().as_u16(), &format!("adsblol trace HTTP {}", res.status().as_u16())));
    }
    let body_bytes = res.bytes().await?;
    if body_bytes.len() > RESPONSE_CAP_BYTES {
        return Ok(json_error(502, "trace response too large"));
    }
    let body_str = String::from_utf8_lossy(&body_bytes).to_string();
    state.tracks.adsblol_cache.lock().await.insert(hex, CachedTrack { at: crate::gev_enrichment::now_ms(), body: body_str.clone() });
    Ok(track_response(200, &body_str, "MISS"))
}
```

```rust
// In tests/gev_tracks.rs add:
#[tokio::test]
async fn adsblol_trace_invalid_hex_400() {
    let state = make_test_state();
    let req = Request::fake_get("/api/adsblol/trace?hex=x");
    let res = handle_adsblol_trace(req, state).await.unwrap();
    assert_eq!(res.status, 400);
}

#[tokio::test]
async fn oauth_inflight_coalesce() {
    let state = make_test_state_with_mock_opensky_auth_once();
    let reqs: Vec<_> = (0..10).map(|_| state.tracks.opensky.get_token()).collect();
    let tokens: Vec<_> = futures::future::join_all(reqs).await;
    // 只 1 个 upstream auth 请求被 mock 接受
    assert!(tokens.iter().all(|t| t.as_ref().unwrap().is_some()));
    assert_eq!(state.tracks.opensky.auth_calls(), 1);
}
```

- [ ] **Step 8: lib.rs + api.rs 注册 + commit**

```rust
// hub-core/crates/hub-core/src/lib.rs
pub mod gev_tracks;
```

```rust
// hub-core/crates/hub-core/src/api.rs
.route("/api/opensky-track", get(gev_tracks::handle_opensky_track_route))
.route("/api/adsblol/trace", get(gev_tracks::handle_adsblol_trace_route))
```

Run: `cd hub-core && cargo test --package hub-core gev_tracks 2>&1 | tail -20`
Expected: 10 tests passed

```bash
git add hub-core/crates/hub-core/src/gev_tracks.rs hub-core/crates/hub-core/src/lib.rs hub-core/crates/hub-core/src/api.rs hub-core/crates/hub-core/tests/gev_tracks.rs
git commit -m "feat(gev-p12-t2): hub-core gev_tracks — OpenSky OAuth adaptive TTL + serve-stale + adsblol trace"
```

**Verify**：
- `cargo test --package hub-core gev_tracks` 全绿（10 tests）
- `cargo build --release --workspace` rc=0
- vendor 0 行改动
- env `OPENSKY_CLIENT_ID` / `OPENSKY_CLIENT_SECRET` 占位加到 `core/secrets.env` 模板（运行时由 user 配置；测试用 mock）

---

### Task 3 — console `aircraft-source.ts` + wiring + 16 单测

**Files**：
1. `console/src/gev-adapters/aircraft-source.ts`（新）—— 完整 spec §5.1 代码
2. `console/src/gev-adapters/index.ts`（改）—— re-export 新 source
3. `console/src/gev-adapters/__tests__/aircraft-source.test.ts`（新）

**Interfaces**：
- Consumes: `ApiFetch` from `gev-adapters/http`
- Produces:
  ```ts
  createIntelHubAircraftSource({ apiFetch }) → IntelHubAircraftSource
  IntelHubAircraftSource.getSnapshot(query, {signal}) → Promise<SnapshotEnvelope<GevAircraftRecord>>
  IntelHubAircraftSource.getTrack(reference, {signal}) → Promise<{ records: TrackRecord[]; complete: false }>
  IntelHubAircraftSource.getEnrichment({kind,id}, {signal}) → Promise<EnrichmentPayload>
  ```

**Steps**：

- [ ] **Step 1: 写 constructor contract 测试（failing）**

```ts
// console/src/gev-adapters/__tests__/aircraft-source.test.ts
import { describe, expect, it, vi } from "vitest";
import { createIntelHubAircraftSource } from "../aircraft-source";

describe("createIntelHubAircraftSource constructor contract", () => {
  it("throws if apiFetch is missing", () => {
    expect(() => createIntelHubAircraftSource({ apiFetch: null as any })).toThrow(TypeError);
  });
  it("throws if apiFetch is not a function", () => {
    expect(() => createIntelHubAircraftSource({ apiFetch: 42 as any })).toThrow(TypeError);
  });
  it("label is hub.intelhub", () => {
    const src = createIntelHubAircraftSource({ apiFetch: vi.fn() });
    expect(src.label).toBe("hub.intelhub");
  });
});
```

- [ ] **Step 2: 跑测试验失败**

Run: `cd console && npx vitest run src/gev-adapters/__tests__/aircraft-source.test.ts 2>&1 | tail -10`
Expected: FAIL with `Cannot find module '../aircraft-source'`

- [ ] **Step 3: 实现 aircraft-source.ts**

```ts
// console/src/gev-adapters/aircraft-source.ts
// (完整 spec §5.1 代码，全文粘贴)
```

(实施者复制 spec §5.1 全文 ~120 LoC)

- [ ] **Step 4: 跑测试验通过**

Run: `cd console && npx vitest run src/gev-adapters/__tests__/aircraft-source.test.ts 2>&1 | tail -10`
Expected: 3 tests passed

- [ ] **Step 5: 加 getTrack 测试（9 cases）**

```ts
// In aircraft-source.test.ts add:
describe("getTrack", () => {
  let mockFetch: ReturnType<typeof vi.fn>;
  let source: ReturnType<typeof createIntelHubAircraftSource>;
  
  beforeEach(() => {
    mockFetch = vi.fn();
    source = createIntelHubAircraftSource({ apiFetch: mockFetch as any });
  });
  
  it("sends /api/opensky-track with lowercase hex", async () => {
    mockFetch.mockResolvedValue({ ok: true, json: () => Promise.resolve({ path: [[1726845215, 40.69, -74.17, 10500, 91, false]] }) });
    const result = await source.getTrack("4CA9B1");
    expect(mockFetch).toHaveBeenCalledWith("/api/opensky-track?icao24=4ca9b1", expect.objectContaining({ signal: expect.any(AbortSignal) }));
    expect(result.records).toHaveLength(1);
    expect(result.records[0]).toMatchObject({ observedAtMs: 1726845215000, latitude: 40.69, longitude: -74.17 });
  });
  
  it("silent fallback on 404", async () => {
    mockFetch.mockResolvedValue({ ok: false, status: 404, json: () => Promise.resolve({}) });
    const result = await source.getTrack("4ca9b1");
    expect(result.records).toEqual([]);
  });
  
  it("silent fallback on 5xx", async () => {
    mockFetch.mockResolvedValue({ ok: false, status: 503 });
    const result = await source.getTrack("4ca9b1");
    expect(result.records).toEqual([]);
  });
  
  it("silent fallback on network error", async () => {
    mockFetch.mockRejectedValue(new Error("network"));
    const result = await source.getTrack("4ca9b1");
    expect(result.records).toEqual([]);
  });
  
  it("respects AbortSignal", async () => {
    const controller = new AbortController();
    mockFetch.mockRejectedValue(new DOMException("aborted", "AbortError"));
    await expect(source.getTrack("4ca9b1", { signal: controller.signal })).resolves.toEqual({ records: [], complete: false });
  });
  
  it("8s timeout via AbortSignal.timeout", async () => {
    vi.useFakeTimers();
    mockFetch.mockImplementation(() => new Promise<any>((_, reject) => setTimeout(() => reject(new Error("timeout")), 10000)));
    const promise = source.getTrack("4ca9b1");
    vi.advanceTimersByTime(8000);
    await expect(promise).resolves.toEqual({ records: [], complete: false });
    vi.useRealTimers();
  });
  
  it("returns empty for non-hex reference (defensive)", async () => {
    const result = await source.getTrack("not-hex");
    expect(result.records).toEqual([]);
    expect(mockFetch).not.toHaveBeenCalled();
  });
  
  it("normalizes multi-waypoint response", async () => {
    mockFetch.mockResolvedValue({ ok: true, json: () => Promise.resolve({ path: [
      [1726845215, 40.69, -74.17, 10500, 91, false],
      [1726845300, 40.70, -74.18, 10600, 92, false],
    ] }) });
    const result = await source.getTrack("4ca9b1");
    expect(result.records).toHaveLength(2);
    expect(result.records[1].observedAtMs).toBe(1726845300000);
  });
  
  it("skips malformed waypoints", async () => {
    mockFetch.mockResolvedValue({ ok: true, json: () => Promise.resolve({ path: [
      [1726845215, 40.69, -74.17, 10500, 91, false],
      [null, null, null],  // invalid
      "not-array",         // invalid
      [1726845500, 40.71, -74.19],  // too short, skip
    ] }) });
    const result = await source.getTrack("4ca9b1");
    expect(result.records).toHaveLength(1);
  });
});
```

- [ ] **Step 6: 加 getEnrichment 测试（4 cases）**

```ts
// In aircraft-source.test.ts add:
describe("getEnrichment", () => {
  let mockFetch: ReturnType<typeof vi.fn>;
  let source: ReturnType<typeof createIntelHubAircraftSource>;
  
  beforeEach(() => {
    mockFetch = vi.fn();
    source = createIntelHubAircraftSource({ apiFetch: mockFetch as any });
  });
  
  it("type path: lowercase hex", async () => {
    mockFetch.mockResolvedValue({ ok: true, json: () => Promise.resolve({ found: true, typeCode: "B738", typeName: "Boeing 737-800" }) });
    const r = await source.getEnrichment({ kind: "type", id: "4CA9B1" });
    expect(mockFetch).toHaveBeenCalledWith("/api/adsbdb/type/4ca9b1", expect.any(Object));
    expect(r).toEqual({ found: true, typeCode: "B738", typeName: "Boeing 737-800" });
  });
  
  it("route path: uppercase callsign", async () => {
    mockFetch.mockResolvedValue({ ok: true, json: () => Promise.resolve({ found: true, airline: "UA", origin: { code: "KEWR" }, destination: { code: "KSFO" } }) });
    const r = await source.getEnrichment({ kind: "route", id: "ual123" });
    expect(mockFetch).toHaveBeenCalledWith("/api/adsbdb/route/UAL123", expect.any(Object));
  });
  
  it("passes through found:false (negative result)", async () => {
    mockFetch.mockResolvedValue({ ok: true, json: () => Promise.resolve({ found: false }) });
    const r = await source.getEnrichment({ kind: "type", id: "4ca9b1" });
    expect(r).toEqual({ found: false });
  });
  
  it("throws on 5xx (vendor records cooldown)", async () => {
    mockFetch.mockResolvedValue({ ok: false, status: 503 });
    await expect(source.getEnrichment({ kind: "type", id: "4ca9b1" })).rejects.toThrow();
  });
  
  it("throws on invalid hex (type)", async () => {
    await expect(source.getEnrichment({ kind: "type", id: "xyz" })).rejects.toThrow(/6-char hex/);
  });
  
  it("throws on invalid callsign (route)", async () => {
    await expect(source.getEnrichment({ kind: "route", id: "x" })).rejects.toThrow(/2-8 char/);
  });
  
  it("throws on unsupported kind", async () => {
    await expect(source.getEnrichment({ kind: "weather" as any, id: "x" })).rejects.toThrow(/unsupported/);
  });
});
```

- [ ] **Step 7: gev-boot 注入**

```ts
// console/src/gev-boot/application.ts
+ import { createIntelHubAircraftSource } from "../gev-adapters";

  createControls: ({ scene, signal, defer }: any) => {
    const sources = createIntelHubLayerSources({ apiFetch: opts.apiFetch });
+   // P12: 注入 aircraft source（替换 P1 stub）
+   const intelAircraft = createIntelHubAircraftSource({ apiFetch: opts.apiFetch });
+   // T3 实施者验证 createIntelHubLayerSources 内 flights slot 注入点
+   // 通常是 sources.flights = intelAircraft 或类似（取决于 P1 实现）
+   // 如果现有结构不允许直接替换，可能需要扩展 createIntelHubLayerSources 接口
+   // （参考现有 P11 cctv-bridge.ts 的注入模式）
    const catalog = createApplicationCatalog({ surface: scene.operations.surface, sources, signal });
    ...
  }
```

- [ ] **Step 8: 跑全部 16 测试 + commit**

Run: `cd console && npx vitest run src/gev-adapters/__tests__/aircraft-source.test.ts 2>&1 | tail -10`
Expected: 16 tests passed

```bash
git add console/src/gev-adapters/aircraft-source.ts console/src/gev-adapters/index.ts console/src/gev-adapters/__tests__/aircraft-source.test.ts console/src/gev-boot/application.ts
git commit -m "feat(gev-p12-t3): console aircraft-source — getTrack + getEnrichment exposed to vendor"
```

**Verify**：
- vitest `aircraft-source.test.ts` 全绿（16 tests）
- `npx tsc -b console` rc=0
- vendor 0 行改动
- 模拟 wiring 后浏览器 console 不报 source injection 错误

---

### Task 4 — source-contracts 路径字面量守卫

**Files**：
1. `console/src/gev-boot/__tests__/source-contracts.test.ts`（改）—— 加 3 method 钉扎 + URL 字面量守卫

**Steps**：

- [ ] **Step 1: 读现有 source-contracts.test.ts 了解守卫风格**

```bash
cat console/src/gev-boot/__tests__/source-contracts.test.ts | head -100
```

**记录现有守卫模式**（基于 P9/P11 经验）：
- 用 `import { ... } from "gev-engine/..."` 列 vendor 模块 + 函数
- 用 mock 对象验证 adapter 提供 vendor 期望的方法
- 用 vi.fn() + spyOn 验证 import 存在

- [ ] **Step 2: 加 P12 守卫（4 cases）**

```ts
// In source-contracts.test.ts add:
import { createIntelHubAircraftSource } from "../../gev-adapters/aircraft-source";

describe("intelhub aircraft source contract (P12)", () => {
  it("exposes getTrack method", () => {
    const src = createIntelHubAircraftSource({ apiFetch: vi.fn() });
    expect(typeof src.getTrack).toBe("function");
  });
  
  it("exposes getEnrichment method", () => {
    const src = createIntelHubAircraftSource({ apiFetch: vi.fn() });
    expect(typeof src.getEnrichment).toBe("function");
  });
  
  it("exposes getSnapshot method (P1 unchanged)", () => {
    const src = createIntelHubAircraftSource({ apiFetch: vi.fn() });
    expect(typeof src.getSnapshot).toBe("function");
  });
  
  it("hardcoded vendor URL strings preserved in aircraft adapter", () => {
    // vendor standalone.js:86,99 hardcoded URLs — must be in adapter source
    const fs = require("fs");
    const path = require("path");
    const content = fs.readFileSync(
      path.resolve(__dirname, "../../gev-adapters/aircraft-source.ts"),
      "utf8",
    );
    expect(content).toContain("/api/opensky-track?icao24=");
    expect(content).toMatch(/\/api\/adsbdb\/\$\{query\.kind\}/);
  });
});
```

- [ ] **Step 3: 跑守卫测试 + commit**

Run: `cd console && npx vitest run src/gev-boot/__tests__/source-contracts.test.ts 2>&1 | tail -10`
Expected: 4 new tests + existing tests all passed

```bash
git add console/src/gev-boot/__tests__/source-contracts.test.ts
git commit -m "test(gev-p12-t4): source-contracts guards for aircraft source + URL path literals"
```

**Verify**：
- vitest `source-contracts.test.ts` 全绿
- 如果有人 refactor 改了 URL 路径，这个测试立即 fail（spec §8 R3）

---

### Task 5 — cockpit `shortcuts.ts` + HudCockpitFrame 挂载 + i18n + 10 单测

**Files**：
1. `console/src/gev-visual/cockpit/shortcuts.ts`（新）—— spec §6.E1 完整代码
2. `console/src/gev-visual/cockpit/__tests__/shortcuts.test.ts`（新）
3. `console/src/gev-visual/cockpit/index.ts`（改）—— re-export
4. `console/src/globe-hud/HudCockpitFrame.tsx`（改）—— 挂 useCockpitShortcuts
5. `console/src/i18n/zh.json` + `en.json`（改）—— 加 cockpit.shortcut.hint

**Steps**：

- [ ] **Step 1: 写 shortcuts hook + 测试（failing）**

```ts
// console/src/gev-visual/cockpit/__tests__/shortcuts.test.ts
import { describe, expect, it, vi, beforeEach } from "vitest";
import { renderHook } from "@testing-library/react";
import { useCockpitShortcuts } from "../shortcuts";

function makeStore(active: boolean, briefingPaused = false) {
  return {
    getState: () => ({ active, trackedId: "x", visionMode: "optical" as const, briefingPaused }),
    enter: vi.fn(), exit: vi.fn(), setVisionMode: vi.fn(), pauseBriefing: vi.fn(), resumeBriefing: vi.fn(),
    subscribe: vi.fn(() => () => {}),
  };
}

describe("useCockpitShortcuts", () => {
  beforeEach(() => {
    localStorage.clear();
  });
  
  it("calls briefing.next on ArrowRight", () => {
    const store = makeStore(true);
    const briefing = { next: vi.fn(), prev: vi.fn() };
    const vision = { setMode: vi.fn() };
    renderHook(() => useCockpitShortcuts({ store: store as any, briefing: briefing as any, vision: vision as any }));
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true }));
    expect(briefing.next).toHaveBeenCalled();
  });
  
  it("calls store.exit on Escape", () => { /* ... */ });
  it("calls vision.setMode + store.setVisionMode on number keys 1-5", () => { /* ... */ });
  it("toggles briefingPaused on Space", () => { /* ... */ });
  it("does not respond when cockpit inactive", () => { /* ... */ });
  it("does not intercept when input/textarea focused", () => { /* ... */ });
  it("does not intercept with Ctrl/Meta/Alt modifier", () => { /* ... */ });
  it("does not repeat on long press (e.repeat=true)", () => { /* ... */ });
  it("persists vision mode to localStorage", () => { /* ... */ });
  it("cleanup removes keydown listener", () => { /* ... */ });
});
```

- [ ] **Step 2: 跑测试验失败**

Run: `cd console && npx vitest run src/gev-visual/cockpit/__tests__/shortcuts.test.ts 2>&1 | tail -10`
Expected: FAIL with `Cannot find module '../shortcuts'`

- [ ] **Step 3: 实现 shortcuts.ts**

```ts
// console/src/gev-visual/cockpit/shortcuts.ts
// (完整 spec §6.E1 代码 ~120 LoC)
```

- [ ] **Step 4: 跑测试验通过（10 tests passed）**

Run: `cd console && npx vitest run src/gev-visual/cockpit/__tests__/shortcuts.test.ts 2>&1 | tail -10`
Expected: 10 tests passed

- [ ] **Step 5: HudCockpitFrame 挂载 + i18n**

```tsx
// In HudCockpitFrame.tsx add:
import { useCockpitShortcuts } from "../gev-visual/cockpit/shortcuts";
import { useTranslation } from "../i18n";

export function HudCockpitFrame({ store, getTrackedInfo, instruments, briefing, vision }: HudCockpitFrameProps) {
  const state = useCockpitStore(store);
  const { t } = useTranslation();
  
  useCockpitShortcuts({
    store,
    briefing: briefing ?? null,
    vision: vision ?? null,
    onToggleHidden: () => store.dispatch?.({ type: "toggleHidden" }),
  });
  
  if (!state.active) return null;
  
  return (
    <div className="hud-cockpit-frame" data-testid="hud-cockpit-frame">
      {/* existing 5 sub-components */}
      <div className="hud-cockpit-shortcut-hint" data-testid="hud-cockpit-shortcut-hint">
        {t("cockpit.shortcut.hint")}
      </div>
    </div>
  );
}
```

```json
// console/src/i18n/zh.json add:
{ "cockpit": { "shortcut": { "hint": "← → 简报 · Esc 退出 · 1-5 视觉 · Space 暂停 · Tab 切换 · Shift+C 隐藏" } } }

// console/src/i18n/en.json add:
{ "cockpit": { "shortcut": { "hint": "← → briefing · Esc exit · 1-5 vision · Space pause · Tab switch · Shift+C hide" } } }
```

- [ ] **Step 6: cockpit-store 加 hidden 字段**

```ts
// In cockpit-store.ts add:
//   hidden: boolean,
//   action: { type: "toggleHidden" }
```

(详细代码 ~10 行变化)

- [ ] **Step 7: commit**

```bash
git add console/src/gev-visual/cockpit/shortcuts.ts console/src/gev-visual/cockpit/__tests__/shortcuts.test.ts console/src/gev-visual/cockpit/index.ts console/src/globe-hud/HudCockpitFrame.tsx console/src/i18n/zh.json console/src/i18n/en.json console/src/gev-visual/cockpit/cockpit-store.ts
git commit -m "feat(gev-p12-t5): cockpit keyboard shortcuts (←→/Esc/1-5/Space/Tab/Shift+C)"
```

**Verify**：
- vitest `shortcuts.test.ts` 全绿（10 tests）
- `npx tsc -b console` rc=0
- i18n 双语字典完整

---

### Task 6 — cockpit `camera-transition.ts` + 时序协调 + 6 单测

**Files**：
1. `console/src/gev-visual/cockpit/camera-transition.ts`（新）—— spec §6.E2 完整代码
2. `console/src/gev-visual/cockpit/__tests__/camera-transition.test.ts`（新）
3. `console/src/globe-hud/HudCockpitFrame.tsx`（改）—— useEffect 时序协调

**Steps**：

- [ ] **Step 1: 写测试（failing, 6 cases）**

```ts
// console/src/gev-visual/cockpit/__tests__/camera-transition.test.ts
import { describe, expect, it, vi } from "vitest";

describe("mountCockpitCameraTransition", () => {
  function makeViewer() {
    return {
      scene: { canvas: {} },
      camera: {
        position: { clone: () => ({ x: 1, y: 2, z: 3 }) },
        heading: 0.5, pitch: -0.3, roll: 0,
        flyTo: vi.fn().mockResolvedValue(true),
      },
    };
  }
  
  it("throws if viewer.camera.flyTo missing", () => {
    expect(() => mountCockpitCameraTransition({ viewer: { scene: { canvas: {} }, camera: {} as any } })).toThrow(TypeError);
  });
  
  it("flyToTracked calls camera.flyTo with destination + orientation + duration 0.7", async () => {
    const viewer = makeViewer();
    const t = mountCockpitCameraTransition({ viewer: viewer as any });
    await t.flyToTracked({ longitude: -74.17, latitude: 40.69, altitude: 10000 });
    expect(viewer.camera.flyTo).toHaveBeenCalled();
    const call = viewer.camera.flyTo.mock.calls[0][0];
    expect(call.duration).toBe(0.7);
    // destination is a Cartesian3 fromDegrees(lon, lat-0.5, alt+1500)
  });
  
  it("flyBackToBaseline returns to captured baseline", async () => {
    const viewer = makeViewer();
    const t = mountCockpitCameraTransition({ viewer: viewer as any });
    await t.flyToTracked({ longitude: -74.17, latitude: 40.69, altitude: 10000 });
    viewer.camera.flyTo.mockClear();
    await t.flyBackToBaseline();
    expect(viewer.camera.flyTo).toHaveBeenCalledWith(expect.objectContaining({ duration: 0.7 }));
  });
  
  it("captureBaseline updates baseline without flying", () => {
    const viewer = makeViewer();
    const t = mountCockpitCameraTransition({ viewer: viewer as any });
    t.captureBaseline();
    expect(t.flyBackToBaseline).toBeDefined();
  });
  
  it("destroy clears baseline and nulls active fly", async () => {
    const viewer = makeViewer();
    const t = mountCockpitCameraTransition({ viewer: viewer as any });
    await t.flyToTracked({ longitude: -74.17, latitude: 40.69, altitude: 10000 });
    t.destroy();
    await t.flyBackToBaseline();
    expect(viewer.camera.flyTo).not.toHaveBeenCalled();
  });
  
  it("concurrent fly calls: new fly aborts old", async () => {
    const viewer = makeViewer();
    let resolveFirst: any;
    viewer.camera.flyTo.mockImplementationOnce(() => new Promise(r => { resolveFirst = r; }));
    const t = mountCockpitCameraTransition({ viewer: viewer as any });
    const first = t.flyToTracked({ longitude: -74.17, latitude: 40.69, altitude: 10000 });
    viewer.camera.flyTo.mockResolvedValueOnce(true);
    const second = t.flyToTracked({ longitude: -122.4, latitude: 37.6, altitude: 10000 });
    resolveFirst?.(true);
    await first;
    await second;
    expect(viewer.camera.flyTo).toHaveBeenCalledTimes(2);
  });
});
```

- [ ] **Step 2: 跑测试验失败**

Run: `cd console && npx vitest run src/gev-visual/cockpit/__tests__/camera-transition.test.ts 2>&1 | tail -10`

- [ ] **Step 3: 实现 camera-transition.ts**

```ts
// console/src/gev-visual/cockpit/camera-transition.ts
// (完整 spec §6.E2 代码 ~80 LoC)
```

- [ ] **Step 4: 跑测试验通过（6 tests passed）**

- [ ] **Step 5: HudCockpitFrame 时序协调**

```tsx
// In HudCockpitFrame.tsx add:
import { mountCockpitCameraTransition } from "../gev-visual/cockpit/camera-transition";
import type { CockpitCameraTransition } from "../gev-visual/cockpit/camera-transition";

const cameraTransitionRef = useRef<CockpitCameraTransition | null>(null);

useEffect(() => {
  if (!viewerRef.current) return;
  cameraTransitionRef.current = mountCockpitCameraTransition({ viewer: viewerRef.current });
  return () => cameraTransitionRef.current?.destroy();
}, []);

useEffect(() => {
  if (state.active && state.trackedId && cameraTransitionRef.current) {
    requestAnimationFrame(async () => {
      const info = getTrackedInfo?.();
      if (info?.longitude != null && info?.latitude != null && info?.altitudeM != null) {
        await cameraTransitionRef.current!.flyToTracked({
          longitude: info.longitude,
          latitude: info.latitude,
          altitude: info.altitudeM,
        });
      }
    });
  } else if (!state.active && cameraTransitionRef.current) {
    cameraTransitionRef.current.flyBackToBaseline();
  }
}, [state.active, state.trackedId]);
```

- [ ] **Step 6: commit**

```bash
git add console/src/gev-visual/cockpit/camera-transition.ts console/src/gev-visual/cockpit/__tests__/camera-transition.test.ts console/src/globe-hud/HudCockpitFrame.tsx
git commit -m "feat(gev-p12-t6): cockpit camera flyTo transition on enter/exit"
```

**Verify**：
- vitest `camera-transition.test.ts` 全绿（6 tests）
- `npx tsc -b console` rc=0

---

### Task 7 — cockpit `viewport-lock.ts` + `panel-drag.ts` disabled + 10 单测（5+5）

**Files**：
1. `console/src/gev-visual/cockpit/viewport-lock.ts`（新）—— spec §6.E5 完整代码
2. `console/src/gev-visual/cockpit/__tests__/viewport-lock.test.ts`（新）
3. `console/src/gev-visual/tail/panel-drag.ts`（改）—— 加 disabled 参数
4. `console/src/gev-visual/tail/__tests__/panel-drag.test.ts`（改或新）—— 5 新 case
5. `console/src/globe-hud/HudCockpitFrame.tsx`（改）—— 挂 viewport-lock + 调 panel-drag.setDisabled

**Steps**：

- [ ] **Step 1: 写 viewport-lock 测试（5 cases，failing）**

```ts
// console/src/gev-visual/cockpit/__tests__/viewport-lock.test.ts
describe("mountCockpitViewportLock", () => {
  function makeViewer() {
    return {
      scene: { screenSpaceCameraController: { enableInputs: true } },
      cesiumWidget: { canvas: document.createElement("canvas") },
    };
  }
  
  it("throws if screenSpaceCameraController missing", () => { /* ... */ });
  it("throws if canvas missing", () => { /* ... */ });
  it("lock: enableInputs=false + cursor=none", () => { /* ... */ });
  it("unlock: enableInputs=true + cursor reset", () => { /* ... */ });
  it("double-lock no-op; destroy auto-unlock", () => { /* ... */ });
});
```

- [ ] **Step 2: 实现 viewport-lock.ts**

```ts
// console/src/gev-visual/cockpit/viewport-lock.ts
// (完整 spec §6.E5 代码 ~50 LoC)
```

- [ ] **Step 3: 跑 5 tests passed**

- [ ] **Step 4: panel-drag.ts 加 disabled**

```ts
// In panel-drag.ts add: (参考 spec §6.E4 ~80 行变化)
//   disabled?: boolean
//   setDisabled(d: boolean) method
//   startDrag 检查 disabled 时调 endDrag()
```

- [ ] **Step 5: 写 5 panel-drag 测试**

```ts
// In panel-drag.test.ts add:
describe("disabled parameter (P12)", () => {
  it("disabled=true 不响应 pointerdown", () => { /* mount with disabled, dispatch, verify no movement */ });
  it("drag 中 disabled=true → 立即 release", () => { /* mount, start drag, setDisabled(true), verify endDrag called */ });
  it("setDisabled(false) 恢复拖拽", () => { /* verify next pointerdown works */ });
  it("destroy 清理所有 listeners", () => { /* verify pointerup no-op after destroy */ });
  it("边界 clamp 越界坐标 clamp 到 viewport", () => { /* drag past edge, verify clamped position */ });
});
```

- [ ] **Step 6: HudCockpitFrame 挂 viewport-lock + 调 panel-drag**

```tsx
// In HudCockpitFrame.tsx add:
import { mountCockpitViewportLock } from "../gev-visual/cockpit/viewport-lock";

useEffect(() => {
  if (!viewerRef.current) return;
  const lock = mountCockpitViewportLock(viewerRef.current);
  if (state.active) lock.lock();
  else lock.unlock();
  return () => lock.destroy();
}, [state.active]);

useEffect(() => {
  // Iterate panel drag handles and set disabled
  if (state.active) {
    panelDragHandles.forEach(h => h.setDisabled(true));
  } else {
    panelDragHandles.forEach(h => h.setDisabled(false));
  }
}, [state.active]);
```

- [ ] **Step 7: commit**

```bash
git add console/src/gev-visual/cockpit/viewport-lock.ts console/src/gev-visual/cockpit/__tests__/viewport-lock.test.ts console/src/gev-visual/tail/panel-drag.ts console/src/gev-visual/tail/__tests__/panel-drag.test.ts console/src/globe-hud/HudCockpitFrame.tsx
git commit -m "feat(gev-p12-t7): cockpit viewport lock + panel-drag disabled while cockpit active"
```

**Verify**：
- vitest `viewport-lock.test.ts` 全绿（5 tests）
- vitest `panel-drag.test.ts` 全绿（含 5 新 case）
- `npx tsc -b console` rc=0

---

### Task 8 — briefing 节奏微调 + 3 单测

**Files**：
1. `console/src/globe-hud/HudCockpitBriefingPanel.tsx`（改）—— manual grace 5s
2. `console/src/hud.css`（改）—— fade + progress bar classes
3. `console/src/globe-hud/__tests__/HudCockpitBriefingPanel.test.tsx`（改或新）—— 3 新 case

**Steps**：

- [ ] **Step 1: 写 3 briefing 测试（failing）**

```tsx
// In HudCockpitBriefingPanel.test.tsx add:
describe("briefing manual grace (P12)", () => {
  it("manual nav sets manualUntilMs to now+5000", async () => {
    const briefing = make_briefing_with_5_bullets();
    render(<HudCockpitBriefingPanel briefing={briefing} />);
    fireEvent.click(screen.getByLabelText("下一页"));
    // Advance 4s — should NOT auto-rotate
    vi.advanceTimersByTime(4000);
    expect(briefing.next).toHaveBeenCalledTimes(1); // only the manual call
    // Advance 2s more (total 6s > 5s grace) — should auto-rotate
    vi.advanceTimersByTime(2000);
    expect(briefing.next).toHaveBeenCalledTimes(2);
  });
  
  it("during grace period auto-rotate skipped", async () => { /* ... */ });
  it("grace expires after 5s", async () => { /* ... */ });
});
```

- [ ] **Step 2: 实现 manual grace + fade + 进度条**

```tsx
// In HudCockpitBriefingPanel.tsx:
const manualUntilRef = useRef(0);

useEffect(() => {
  if (!briefing || paused || briefing.total() === 0) return;
  const timer = setInterval(() => {
    if (Date.now() < manualUntilRef.current) return;
    briefing.next();
    setIndex(briefing.index());
  }, COCKPIT_BRIEF_ROTATE_MS);
  return () => clearInterval(timer);
}, [briefing, paused, data]);

const onNext = () => {
  briefing?.next();
  setIndex(briefing?.index() ?? 0);
  manualUntilRef.current = Date.now() + 5000;
};
```

```css
/* In hud.css: */
.hud-cockpit-summary-bullet {
  transition: opacity 200ms ease-in-out, transform 200ms ease-in-out;
}
.hud-cockpit-summary-bullet.fading {
  opacity: 0;
  transform: translateY(-4px);
}
.hud-cockpit-progress {
  position: relative;
  height: 2px;
  background: rgba(255,255,255,0.1);
}
.hud-cockpit-progress-bar {
  position: absolute;
  height: 100%;
  background: rgba(255,200,0,0.8);
  transition: width 100ms linear;
}
```

- [ ] **Step 3: 跑 3 tests passed**

```bash
git add console/src/globe-hud/HudCockpitBriefingPanel.tsx console/src/hud.css console/src/globe-hud/__tests__/HudCockpitBriefingPanel.test.tsx
git commit -m "feat(gev-p12-t8): cockpit briefing manual grace + fade + progress bar"
```

**Verify**：
- vitest `HudCockpitBriefingPanel.test.tsx` 全绿（含 3 新 case）
- `npx tsc -b console` rc=0

---

### Task 9 — 验收脚本扩展 + 315 验收

**Files**：
1. `scripts/accept-sp6.py`（改）—— 加 check_42 + check_43
2. `scripts/accept-sp8.py`（改）—— 加 check_49-53
3. `console/probe-gev.mjs`（改）—— 加 P12_PROBES
4. （如需要）`scripts/probe-cockpit-p12.mjs`（新）—— Playwright 跑 cockpit probe

**Steps**：

- [ ] **Step 1: 读现有 accept-sp6.py / accept-sp8.py 了解检查风格**

```bash
cat scripts/accept-sp6.py | head -100
cat scripts/accept-sp8.py | head -100
```

**记录**：现有检查用 `_remote.sh()` / `_remote.pg()` 等 helper；新检查沿用风格

- [ ] **Step 2: 加 sp6 check_42 + check_43**

```python
# In accept-sp6.py add:
def check_42_adsbdb_route(self):
    res = self.fetch("/api/adsbdb/route/UAL123")
    assert res.status == 200, f"expected 200, got {res.status}"
    body = res.json()
    assert "found" in body
    if body["found"]:
        for k in ("airline", "origin", "destination"):
            assert k in body, f"missing {k}"
        for port_name in ("origin", "destination"):
            port = body[port_name]
            for k in ("code", "name", "lat", "lon"):
                assert k in port

def check_43_opensky_track(self):
    TEST_HEX = "4ca9b1"
    res = self.fetch(f"/api/opensky-track?icao24={TEST_HEX}")
    assert res.status == 200
    body = res.json()
    assert "records" in body and isinstance(body["records"], list)
    if body["records"]:
        for rec in body["records"][:3]:
            for k in ("observedAtMs", "latitude", "longitude"):
                assert k in rec
```

- [ ] **Step 3: 加 sp8 check_49-53**

(由于 sp8 现有可能用静态分析或 Playwright probe，**Step 3 实施者先看 sp8 baseline**。如现有是静态分析：check_49-50 静态（验 dist bundle 含 getTrack + opensky-track），check_51-53 标 deferred + 在 probe-cockpit-p12.mjs 用 Playwright 验。)

```python
def check_49_aircraft_source_getTrack(self):
    bundle = self.vm.read_console_dist_assets()
    assert "getTrack" in bundle
    assert "/api/opensky-track" in bundle

def check_50_aircraft_source_getEnrichment(self):
    bundle = self.vm.read_console_dist_assets()
    assert "getEnrichment" in bundle
    assert "/api/adsbdb" in bundle

def check_51_cockpit_keyboard_shortcut(self):
    result = self.vm.probe_cockpit_keyboard()  # via probe-cockpit-p12.mjs
    assert result["left_arrow_calls_prev"] is True
    # ... etc

def check_52_cockpit_camera_transition(self):
    result = self.vm.probe_cockpit_camera()
    assert result["flyTo_called"] is True
    assert 0.6 <= result["duration"] <= 0.8

def check_53_cockpit_viewport_lock(self):
    result = self.vm.probe_cockpit_viewport()
    assert result["enableInputs_false"] is True
```

- [ ] **Step 4: 加 probe-gev P12_PROBES + probe-cockpit-p12.mjs**

```js
// In console/probe-gev.mjs add:
const P12_PROBES = [
  ["p12-aircraft-source-getTrack",      () => !!window.__gevAircraftSource?.getTrack],
  ["p12-aircraft-source-getEnrichment", () => !!window.__gevAircraftSource?.getEnrichment],
  ["p12-cockpit-active",                () => !!document.querySelector('[data-testid="hud-cockpit-frame"]')],
  ["p12-cockpit-exit-btn",              () => !!document.querySelector('[data-testid="hud-cockpit-exit"]')],
  ["p12-cockpit-vision-keys",           () => document.querySelectorAll('[data-testid^="hud-cockpit-vision-"]').length === 5],
  ["p12-cockpit-shortcut-hint",         () => /← →/.test(document.body.innerText)],
  ["p12-cockpit-viewport-lock",         () => window.__cockpitStore?.getState?.().viewportLocked === true],
  ["p12-tracks-endpoint-reachable",     async () => {
    const res = await fetch("/api/opensky-track?icao24=4ca9b1");
    return res.status === 200;
  }],
];
```

(如 Playwright probe 需要：`scripts/probe-cockpit-p12.mjs` ~80 LoC)

- [ ] **Step 5: rsync → 315 + build + restart + 5min wait + 验收**

```bash
rsync -az --delete \
  --exclude '.git/' --exclude 'backups/' --exclude '.DS_Store' \
  --exclude 'compose/.env' --exclude 'compose/.env.crucix' --exclude 'docs/' --exclude 'build/' \
  --exclude 'config/searxng/' --exclude 'hub-core/target/' \
  --exclude 'console/node_modules/' --exclude 'console/dist/' \
  --exclude 'core/' --exclude 'data/' \
  ../IntelHub-p12/ Debian-test:/home/zou/IntelHub/

ssh -o BatchMode=yes Debian-test 'cd /home/zou/IntelHub && \
  bash scripts/build-hub.sh 2>&1 | grep -E "^error|built" | head -8 && \
  bash scripts/build-console.sh 2>&1 | tail -1 && \
  sudo systemctl restart hub-core && sleep 300 && systemctl is-active hub-core'

KEY=$(ssh -o BatchMode=yes Debian-test 'grep -o "ihk_[a-f0-9]*" /home/zou/IntelHub/core/agent-keys.txt | head -1')

python3 scripts/accept-sp6.py "$KEY"   # 期望 41+5sh/0f
python3 scripts/accept-sp8.py "$KEY"   # 期望 53+2sh/0f
python3 scripts/accept-sp3.py "$KEY"   # 回归
python3 scripts/accept-sp7.py "$KEY"   # 回归
```

- [ ] **Step 6: commit**

```bash
git add scripts/accept-sp6.py scripts/accept-sp8.py console/probe-gev.mjs scripts/probe-cockpit-p12.mjs
git commit -m "test(gev-p12-t9): sp6+2 + sp8+5 + probe P12_PROBES for flight layer parity acceptance"
```

**Verify**：
- 315 sp6 41+5sh/0f
- 315 sp8 53+2sh/0f
- 315 sp3 19/0 + sp7 16+11sh/0f（回归无破坏）
- probe exit 0
- sp6 check_42/43 PASS
- sp8 check_49/50 PASS（如 check_51-53 deferred 则标 acceptance-deferred）

---

### Task 10 — 熵减 + whole-branch review + ledger + 410 + push

**Files**：
1. `docs/superpowers/execution/2026-09-21-gev-p12-ledger.md`（新）—— 实施 ledger
2. `AGENTS.md`（改）—— sp6 +2 / sp8 +5 baseline 更新
3. `docs/agents/issue-tracker.md`（改）—— P12 closed entry

**Steps**：

- [ ] **Step 1: 熵减 grep**

```bash
# 这些应在 console/src 范围内检查：
grep -rn "TODO.*p12\|TODO.*flight.*layer\|TODO.*enrichment\|TODO.*opensky" console/src/ hub-core/src/ | head -10 || echo 'NO TODOs'
grep -rn "cockpitController\|cockpitCoordinator\|cockpitTrackingController" console/src/ | head -5
grep -rn "GEV P12\|gev-p12" console/src/ hub-core/src/ | wc -l  # 应至少 30+ 处（commit 历史 + 注释）
```

- [ ] **Step 2: whole-branch final review（dispatch subagent）**

```bash
/Users/zouguojun/.pi/agent/git/github.com/obra/superpowers/skills/subagent-driven-development/scripts/dispatch-task \
  --task task-10-review --base cd07a7b --head feat/p12-flight-layer-parity \
  --model deepseek-v4-pro
```

Brief 含：spec 路径 + plan 路径 + 全 diff + T1-T9 reports + 验证命令。Ask: coherence / type consistency / entropy / spec coverage / 已知边界 triage。

- [ ] **Step 3: 写 ledger**

```bash
cat > docs/superpowers/execution/2026-09-21-gev-p12-ledger.md <<'EOF'
# GEV P12 Flight Layer Parity — Ledger

> 日期：2026-09-21 · 状态：completed
> Worktree：feat/p12-flight-layer-parity（已合并到 main）
> 最终 commit：$(git rev-parse HEAD)

## 任务完成
- T1 gev_enrichment.rs: 8/8 tests passed
- T2 gev_tracks.rs: 10/10 tests passed
- T3 aircraft-source.ts: 16/16 tests passed
- T4 source-contracts: 4/4 new guards
- T5 shortcuts: 10/10 tests passed
- T6 camera-transition: 6/6 tests passed
- T7 viewport-lock + panel-drag: 10/10 tests passed
- T8 briefing: 3/3 tests passed
- T9 acceptance: sp6 41+5sh/0f, sp8 53+2sh/0f

## 已知边界状态
- adsbdb 24h cache: ✓ 持久化
- OpenSky OAuth: env 占位 + fail-fast 503
- Cockpit 键盘 i18n: ✓ 双语
- viewport lock 已知边界：touch 设备单指 tap 保留（coarse pointer）

## 验收
- 315 baseline: sp6 41+5sh/0f, sp8 53+2sh/0f, sp3 19/0
- 410 baseline: 同上
EOF
```

- [ ] **Step 4: AGENTS.md + issue-tracker 更新**

```bash
# AGENTS.md: 改 sp6 + sp8 baseline 数（仅这两行）
# docs/agents/issue-tracker.md: 加 P12 closed entry
```

- [ ] **Step 5: git merge --no-ff + 410 部署 + 410 验收 + push**

```bash
cd /Volumes/TBU/Workspace/IntelHub
git merge --no-ff feat/p12-flight-layer-parity
git worktree remove ../IntelHub-p12 && git branch -d feat/p12-flight-layer-parity

rsync -az --delete <excludes> ./ IntelHub:/home/zou/IntelHub/

ssh IntelHub 'cd /home/zou/IntelHub && \
  bash scripts/build-hub.sh 2>&1 | tail -5 && \
  bash scripts/build-console.sh 2>&1 | tail -1 && \
  sudo systemctl restart hub-core && sleep 300 && systemctl is-active hub-core'

KEY=$(ssh IntelHub 'grep -o "ihk_[a-f0-9]*" /home/zou/IntelHub/core/agent-keys.txt | head -1')
python3 scripts/accept-sp6.py "$KEY"   # 410 期望 41+5sh/0f
python3 scripts/accept-sp8.py "$KEY"   # 410 期望 53+2sh/0f
python3 scripts/accept-sp3.py "$KEY"   # 回归
python3 scripts/accept-sp7.py "$KEY"   # 回归

git push origin main
```

**Verify**：
- 410 sp6 41+5sh/0f
- 410 sp8 53+2sh/0f
- 410 sp3 19/0
- 410 sp7 16+11sh/0f
- ledger commit + push 全部已发
- Final review verdict APPROVE

---

## 6. 风险与缓解

| # | 风险 | 缓解 | 归属 task |
|---|---|---|---|
| R1 | OpenSky OAuth 上游契约漂移 | token parse 防御性 + 测试断言必填字段 + env 缺失 fail-fast 503 | T2 |
| R2 | adsbdb 速率限制（首次 cold cache burst） | in-flight coalesce + 永久负缓存 + dirty 串行刷盘 | T1 |
| R3 | Vendor URL 路径写死（refactor 静默破坏） | source-contracts.test.ts 加路径字面量断言 | T4 |
| R4 | Cesium camera.flyTo + vendor tracking 冲突 | enter 时机：vendor 1 帧后再 flyTo（短距离 delta）；exit 时机：flyBack resolve 后再 store.exit | T6 |
| R5 | Cockpit-active 时 Cesium selection 事件 | viewport-lock.ts 在 lock() 时 setInputAction(noop, LEFT_CLICK/DOUBLE_CLICK/RIGHT_CLICK) | T7 |
| R6 | 快速 enter/exit race（< 0.7s 飞行时间内） | _activeFlyPromise 维护，新 fly 取消旧的；AbortError silent | T6 |
| R7 | panel-drag 锁定后未释放的拖拽 | disabled=true 时同步 endDrag() 释放 pointer capture | T7 |
| R8 | briefing fade + manual nav race | CSS transition + manual nav 重置 transform/opacity | T8 |
| R9 | i18n 遗漏 | 双语字典 + Linter 检查所有新增 cockpit 文案 | T5 |
| R10 | acceptance 静态 vs runtime probe | T9 step 3 先看 sp8 baseline；如不支持 Playwright，check_51-53 标 deferred | T9 |

## 7. 验证协议

每 task 实施者须在 dispatch 后**先打开 vendor + 已上线代码**，纠正 plan 错误：

| Task | 必须验证的假设 |
|---|---|
| T1 | api.rs 当前 route 注册风格（axum / actix / 自定义）→ 照搬 |
| T1 | `gev_enrichment` 模块在 lib.rs 中的字母序插入位置 |
| T1 | `AppState` 是否有 `enrichment` 字段；如无，扩展 struct |
| T2 | T1 同 + OpenSky OAuth env 注入路径（secrets.env 还是 hub.env） |
| T2 | adsblol trace 响应是否真是 array-of-arrays（GEV 假设） |
| T3 | `createIntelHubLayerSources` 内部 flights slot 注入点（P1 实际是 stub 还是已实现） |
| T3 | `gev-adapters/http` 的 `ApiFetch` 实际签名（signal 处理细节） |
| T4 | 现有 source-contracts.test.ts 的守卫风格（mock 还是 spy） |
| T5 | cockpit-store 是否已有 `hidden` 字段；如无需加 |
| T6 | Cesium 的 `Camera.flyTo` 返回 Promise 的版本（1.x vs 2.x 行为差异） |
| T7 | panel-drag 现有 listener 结构（pointer vs mouse events） |
| T7 | Cesium `ScreenSpaceEventHandler` 在 jsdom 测试环境的可用性 |
| T8 | 现有 HudCockpitBriefingPanel useEffect 依赖 |
| T9 | sp8 现有 baseline 是 Playwright 还是静态分析 |

每 task 报告 → `/Volumes/TBU/Workspace/IntelHub-p12/.superpowers/sdd/2026-09-21-gev-p12-flight-layer-parity/task-{N}-report.md`

## 8. 熵减清单（task 9 / 10 必做）

- `grep -rn "TODO.*p12\|TODO.*flight.*layer\|TODO.*enrichment\|TODO.*opensky" console/src/ hub-core/src/` 应为空
- `grep -rn "cockpitController\|cockpitCoordinator\|cockpitTrackingController" console/src/` 仅 adapter 层允许
- `grep -rn "GEV P12\|gev-p12" console/src/ hub-core/src/` 应至少 30+ 处（commit msg + comment references）
- `console/src/hud.css` 增 `.hud-cockpit-*`；不删除 P9/P10 已有类

## 9. 任务派发模板

每 task dispatch（subagent-driven）：
```bash
/Users/zouguojun/.pi/agent/git/github.com/obra/superpowers/skills/subagent-driven-development/scripts/dispatch-task \
  --task task-{N} --base {prev-commit-or-cd07a7b} \
  --model deepseek-v4-flash  # 或 deepseek-v4-pro for final review
```

Brief 含：plan 链接 + 验证协议 + cross-task context + 工作目录 `/Volumes/TBU/Workspace/IntelHub-p12/` + 验证命令 + 报告路径 `.superpowers/sdd/2026-09-21-gev-p12-flight-layer-parity/task-{N}-report.md`

## 10. 文档同步

- `docs/superpowers/specs/2026-09-21-gev-p12-flight-layer-design.md` ← 已提交（3504e81）
- `docs/superpowers/plans/2026-09-21-gev-p12-flight-layer-implementation.md` ← 本文档
- `docs/superpowers/execution/2026-09-21-gev-p12-ledger.md` ← T10 创建
- `AGENTS.md` ← T10 更新 sp6+2/sp8+5 baseline
- `docs/agents/issue-tracker.md` ← T10 P12 closed entry
