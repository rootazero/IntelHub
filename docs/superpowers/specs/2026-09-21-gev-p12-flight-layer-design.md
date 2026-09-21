# GEV P12 航班层 GEV 对等 设计

> 日期：2026-09-21 · 状态：草稿（待用户批准）
> 性质：分期 P12，隶属于 `docs/superpowers/specs/2026-09-17-gev-engine-fusion-design.md` 总纲的「航班层对等」分期
> 起点：main `cd07a7b`（P1-P11 已落地 11 个分期：CCTV / 标注 / 视觉 / Tail / HUD / Cockpit / Camera / etc.）
> 参考：`/Volumes/TBU/Github/gods-eye-view`（GEV 独立项目，server/ 4 文件提供完整航班层数据后端 + 客户端 trails/enrichment 渲染）

## 0. 目标与边界

### 目标

将 GEV 项目的航班层能力**对等移植**到 IntelHub：

1. **Enrichment 后端代理** — adsbdb.com callsign → route（airline + origin/destination）+ hex → type（机型 + 注册号），磁盘持久化 24h 缓存
2. **Track backfill 后端代理** — OpenSky `/tracks/all` OAuth + 自适应 TTL + 429 cooldown + serve-stale；adsb.lol trace 长尾 fallback
3. **Console 适配层扩展** — 暴露 `getTrack` + `getEnrichment` 给 vendor engine（vendor `standalone.js` 已写死 URL 契约）
4. **Ribbon trail 渲染** — vendor 自带 `tracking.js:467-560` 已做 fire-and-forget backfill + splice，我们**不写自定义 Cesium ribbon**
5. **Cockpit UX Aggressive 抛光** — 键盘快捷键（← → / Esc / 1-5 / Space / Tab / Shift+C）+ Cesium camera.flyTo 进出过渡 + briefing 节奏微调（manual 后 5s grace + fade 动画 + 进度条）+ panel 拖拽 cockpit-active 锁定 + viewport 锁定（Cesium input 拦截）

### 显式不做的（与总纲一致）

- **不 fork vendor engine 代码** — 契约守卫 `console/src/gev-boot/__tests__/source-contracts.test.ts` 保持现状
- **不做** GEV voice control（vendor 独有 demo 功能）
- **不做** cockpitSignal ↔ Signals/Neo4j 联动（P9 已禁）
- **不做** cloud/precipitation vendor 渲染（要数据源+独立阶段）
- **不重做** P9 cockpit 已有 5 个 adapter + 5 个 HUD 组件（P9 不变）
- **不重做** P6 visual-effects、P7 follow-controller / camera-orientation（P12 是消费者）
- **不重做** briefing data shape（P9 已定）
- **不写** Cesium ribbon 渲染（vendor 自带）
- **不重写** vendor `enrichment.js` drip dispatch + `_enrichSeen` dedup（vendor 自带）
- **不做** panel-drag 惯性数学（CSS transition + easeOutCubic 足够）
- **不做** multi-monitor 边界处理（截断到 viewport）

### 设计原则

- **URL 路径 vendor 直连**：`/api/opensky-track` 和 `/api/adsbdb/<kind>/<id>` 是 vendor `standalone.js:86,99` 写死的字符串，不能重写（source-contracts.test.ts 加路径字面量断言守住）
- **薄适配层**：console 侧**不加缓存**（passthrough 到 hub-core 后由 disk cache + vendor `_enrichSeen` dedup）
- **silent fallback**：trail backfill 失败时返 `{records: [], complete: false}`，vendor `tracking.js:486-492 catch { return; }` 走本地累积 trail
- **显式错误**：enrichment 5xx throw（vendor 期望记 cooldown），4xx 透传（vendor 期望 silently skip）
- **8s timeout**：trail backfill 用 `AbortSignal.timeout(8000)`，与 vendor 一致
- **vendor URL 写死**：`/api/opensky-track?icao24=` 和 `/api/adsbdb/<kind>/<id>` 是 vendor `standalone.js` 写死的字符串，**不能重写**；source-contracts 加路径字面量断言

## 1. 现状盘点

### IntelHub 现状（main `cd07a7b`）

| 模块 | 文件 | 状态 |
|---|---|---|
| Cockpit 适配层 | `console/src/gev-visual/cockpit/` (5 文件, ~600 LoC) | ✅ P9 |
| Cockpit HUD | `console/src/globe-hud/HudCockpit*` (5 文件, ~700 LoC) | ✅ P9 |
| Aircraft adapter (snapshot) | `console/src/gev-adapters/aircraft-map.ts` (125 LoC) | ✅ P1 (only mapper, no source object) |
| Aircraft source (track/enrichment) | — | ❌ **缺** |
| OpenSky OAuth | hub-core ADS-B collector 自带 OpenSky 抓取 | ✅ 已存在（不暴露 API） |
| OpenSky `/tracks/all` 代理 | — | ❌ **缺** |
| adsb.lol trace 代理 | — | ❌ **缺** |
| adsbdb enrichment 代理 | — | ❌ **缺** |
| Cockpit 键盘快捷键 | — | ❌ **缺** |
| Cockpit 相机过渡 | — | ❌ **缺**（直接 setView） |
| Cockpit viewport 锁定 | — | ❌ **缺** |
| Panel drag 锁定 | `console/src/gev-visual/tail/panel-drag.ts` P10 | ⚠️ 无 disabled 参数 |

### Vendor engine 现状（`console/gev-engine/`）

**已写死 URL 契约**（`gev-engine/src/sources/live/standalone.js`）：
```js
// Line 86
'/api/opensky-track?icao24=' + encodeURIComponent(reference)
// Line 99
`/api/adsbdb/${query.kind}/${encodeURIComponent(query.id)}`  // kind ∈ {type, route}
```

**已实现**：
- `enrichment.js` (149 行) — drip dispatch + ambient budget + `_enrichSeen` dedup + priority queue
- `tracking.js:467-560` — fire-and-forget trail backfill（调 `feed._source.getTrack()`）
- `rendering.js` — ribbon trail 渲染（读 `flightState.history`）
- `motion.js:175` — track 时间插值
- `standalone.js:59-103` — aircraft source 对象契约（`getSnapshot/getTrack/getEnrichment`）

### GEV 项目对照（`/Volumes/TBU/Github/gods-eye-view`）

| GEV 文件 | 行数 | 移植策略 |
|---|---|---|
| `server/providers/aircraft/opensky.js` | 701 | ✅ Rust 重写（hub-core `gev_tracks.rs`） |
| `server/providers/aircraft/adsb-lol.js` | 158 | ✅ 部分复用（trace endpoint 复用到 `gev_tracks.rs`） |
| `server/providers/aircraft/tracks.js` | 128 | ✅ Rust 重写（hub-core `gev_tracks.rs`） |
| `server/providers/aircraft/enrichment.js` | 147 | ✅ Rust 重写（hub-core `gev_enrichment.rs`） |

### 当前验收 baseline

- sp6 39+5sh/0f（共 44 项）
- sp8 48+2sh/0f（共 50 项）
- sp3 19 passed
- sp2a 19 / sp2b 33 / sp4 25 / sp5 9 / sp7 16+11sh / sp9 14

## 2. 总体架构

```
┌────────────────────────────────────────────────────────────────┐
│ Console (前端)                                                  │
├────────────────────────────────────────────────────────────────┤
│ HudCockpitFrame + 5 子组件（已有，P9）                          │
│   + useCockpitShortcuts()  ←── Section 6 E1 (~120 LoC)         │
│   + mountCockpitCameraTransition ←── Section 6 E2 (~80 LoC)    │
│   + viewport-lock.ts  ←── Section 6 E5 (~50 LoC)               │
│   + briefing panel 微调  ←── Section 6 E3 (~30 LoC)            │
│                                                                  │
│ gev-adapters/aircraft-source.ts (NEW, ~120 LoC)                 │
│   + getSnapshot (从 aircraft-map.ts re-export)                  │
│   + getTrack(icao24, {signal}) → 代理 /api/opensky-track        │
│   + getEnrichment({kind,id},{signal}) → 代理 /api/adsbdb/..     │
│   (Section 5)                                                    │
│                                                                  │
│ gev-boot/application.ts                                         │
│   + 注入 aircraft source (Section 5 wiring, ~10 行)             │
├────────────────────────────────────────────────────────────────┤
│ Hub-core (后端新增 2 模块)                                       │
├────────────────────────────────────────────────────────────────┤
│ gev_enrichment.rs (NEW, ~250 LoC)                                │
│   GET /api/adsbdb/type/<hex6>                                    │
│     → adsbdb.com /v0/aircraft/{hex}                              │
│   GET /api/adsbdb/route/<cs>                                     │
│     → adsbdb.com /v0/callsign/{cs}                               │
│   缓存：/var/lib/intelhub/adsbdb-cache.json (24h TTL)            │
│   + 启动 loadOnce + dirty + setInterval(15s, .unref()) 刷盘    │
│   (Section 4 A)                                                  │
│                                                                  │
│ gev_tracks.rs (NEW, ~350 LoC)                                    │
│   GET /api/opensky-track?icao24=<hex6>                           │
│     → OpenSky /api/tracks/all (OAuth + adaptive TTL + cooldown)  │
│   GET /api/adsblol/trace?hex=<hex>                               │
│     → adsb.lol /data/traces/{xx}/trace_full_<hex>.json          │
│   缓存：内存 Map<key, {at, status, body}> 60s TTL                │
│   (Section 4 B)                                                  │
└────────────────────────────────────────────────────────────────┘

数据流（用户视角）：
  1. 用户点击飞机 → vendor `tracking.js:467` 启动追踪
  2. vendor 自动调 `feed._source.getTrack(icao24, {signal})`
     → adapter → hub-core /api/opensky-track → OpenSky
     → 历史 trail waypoints splice 到本地累积 trail
     → vendor `rendering.js` 自动渲染 ribbon
  3. vendor `enrichment.js:60` drip-dispatch 调用 `getEnrichment({kind, id})`
     → adapter → hub-core /api/adsbdb/{type|route}/{id} → adsbdb
     → 记录 meta.typeCode / meta.typeName / meta.registration / meta.route
     → UI 显示 "United Airlines B738 Newark→San Francisco"
  4. vendor `motion.js:175` 时间插值渲染逐帧位置
```

## 3. 数据契约

### 3.1 OpenSky track backfill

**请求**：`GET /api/opensky-track?icao24=<hex6>`  
**响应**：
```json
{
  "records": [
    { "observedAtMs": 1726845215000,
      "latitude": 40.69, "longitude": -74.17,
      "baroAltitudeM": 10500, "courseDeg": 91.2, "onGround": false },
    ...
  ],
  "complete": false
}
```
**字段映射**：OpenSky `/tracks/all` 返回 array-of-arrays `[time, lat, lon, baro_alt, true_track, on_ground]`，`gev_tracks.rs` normalize 成对象数组。

**OpenSky 错误响应**：
- 404 → 透传 404 + `{error: "Track source HTTP 404"}`（vendor 期望 silent fallback）
- 429 → serve-stale（200 + 上次缓存 + `X-OpenSky-Stale: 1` header）
- 5xx → 透传 5xx + 错误 body

**认证**：OAuth2 client_credentials，env `OPENSKY_CLIENT_ID` / `OPENSKY_CLIENT_SECRET`（写入 `core/secrets.env`）

### 3.2 adsbdb enrichment

**请求 type**：`GET /api/adsbdb/type/<hex6>`（小写）  
**响应 type**：
```json
{ "found": true, "typeCode": "B738", "typeName": "Boeing 737-800", "registration": "EI-DCL" }
```
或 `404` → `{found: false}`（vendor 视为合法 empty result，UI 显示「未知」）

**请求 route**：`GET /api/adsbdb/route/<callsign>`（大写，2-8 字符）  
**响应 route**：
```json
{
  "found": true,
  "airline": "United Airlines",
  "origin":      { "code": "KEWR", "name": "Newark", "lat": 40.69, "lon": -74.17 },
  "destination": { "code": "KSFO", "name": "San Francisco", "lat": 37.62, "lon": -122.38 }
}
```

**字段映射**：
- type：`adsbdb.com /v0/aircraft/{hex}` → `response.aircraft.{icao_type, manufacturer, type, registration}`
- route：`adsbdb.com /v0/callsign/{cs}` → `response.flightroute.{airline, origin, destination}`

### 3.3 adsb.lol trace fallback

**请求**：`GET /api/adsblol/trace?hex=<hex6|hex7>`（小写）  
**响应**：同 OpenSky track shape（`{records: [...]}`），但来自 adsb.lol tar1090 readsb trace

**错误响应**：upstream 4xx/5xx → 透传 + `{error: "..."}`（vendor silent fallback）

### 3.4 Vendor source 接口（console）

```ts
export interface IntelHubAircraftSource {
  label: string;  // "hub.intelhub"
  getSnapshot(query: SnapshotQuery, opts: { signal?: AbortSignal })
    : Promise<SnapshotEnvelope<GevAircraftRecord>>;
  getTrack(reference: string, opts: { signal?: AbortSignal })
    : Promise<{ records: TrackRecord[]; complete: false }>;
  getEnrichment(query: { kind: "type" | "route"; id: string }, opts: { signal?: AbortSignal })
    : Promise<EnrichmentPayload>;
}

export interface TrackRecord {
  observedAtMs: number;
  latitude: number;
  longitude: number;
  baroAltitudeM: number | null;
  courseDeg?: number | null;
  onGround?: boolean;
}

export type EnrichmentPayload =
  | { found: true; typeCode?: string; typeName?: string; registration?: string }
  | { found: true; airline?: string; origin?: AirportInfo; destination?: AirportInfo }
  | { found: false };
```

## 4. 后端模块

### 4.A `gev_enrichment.rs` — adsbdb enrichment 代理

**文件位置**：`hub-core/src/gev_enrichment.rs`（new file）  
**注册**：`hub-core/src/lib.rs::modules()` 添加 `register_module("gev_enrichment", Box::new(gev_enrichment::module()))`

**路由表**：
```rust
async fn route(req: Request, state: AppState) -> Result<Response> {
    let path = req.path();
    if let Some(hex) = path.strip_prefix("/api/adsbdb/type/") {
        return handle_type(state, hex).await;
    }
    if let Some(cs) = path.strip_prefix("/api/adsbdb/route/") {
        return handle_route(state, cs).await;
    }
    Err(HubError::not_found("unknown adsbdb endpoint"))
}
```

**核心结构**：
```rust
pub struct EnrichmentCache {
    routes: HashMap<String, CachedRoute>,    // callsign → entry
    aircraft: HashMap<String, CachedAircraft>, // hex → entry
    dirty: AtomicBool,
    inflight: Mutex<HashMap<String, Arc<OnceCell<()>>>>,
}

const TTL_MS: u64 = 24 * 3600 * 1000;
const CACHE_PATH: &str = "/var/lib/intelhub/adsbdb-cache.json";

struct CachedRoute { at: u64, data: Option<RouteData> }  // Option for negative cache
struct CachedAircraft { at: u64, data: Option<AircraftData> }
```

**关键函数**：
```rust
async fn load_once(cache: &EnrichmentCache) {
    match tokio::fs::read_to_string(CACHE_PATH).await {
        Ok(s) => match serde_json::from_str::<PersistedCache>(&s) {
            Ok(p) => { cache.routes = p.routes; cache.aircraft = p.aircraft; }
            Err(e) => warn!("adsbdb cache parse failed: {}", e),
        },
        Err(_) => {}, // 首次运行视为空 cache
    }
    // dirty 标记 + setInterval 刷盘
    let cache_weak = cache.weak_clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(15));
        interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
        interval.tick().await; // 跳过 immediate
        loop {
            interval.tick().await;
            if cache_weak.dirty.swap(false, Ordering::Relaxed) {
                if let Err(e) = cache_weak.persist().await {
                    warn!("adsbdb cache persist failed: {}", e);
                }
            }
        }
    });
}

async fn handle_type(state: AppState, hex: &str) -> Result<Response> {
    let hex = hex.to_lowercase();
    if !HEX6.is_match(&hex) {
        return Ok(json_error(400, "invalid hex"));
    }
    
    // 1. Cache hit (fresh)
    if let Some(entry) = state.enrichment.aircraft.get(&hex) {
        if is_fresh(entry.at) {
            return Ok(json_response(match &entry.data {
                Some(d) => AircraftFound { found: true, ...d },
                None => AircraftFound { found: false },
            }));
        }
    }
    
    // 2. In-flight coalesce
    let key = format!("type:{}", hex);
    let _guard = state.enrichment.inflight.entry(key.clone()).or_insert_with(...);
    
    // 3. Upstream fetch
    let url = format!("https://api.adsbdb.com/v0/aircraft/{}", hex);
    let res = state.http.get(&url).timeout(8000).send().await?;
    
    if res.status() == 404 {
        // Negative cache
        state.enrichment.aircraft.insert(hex, CachedAircraft { at: now_ms(), data: None });
        return Ok(json_response(AircraftFound { found: false }));
    }
    if !res.status().is_success() {
        return Ok(json_error(503, "adsbdb upstream unavailable"));
    }
    
    let body: serde_json::Value = res.json().await?;
    let data = parse_aircraft(&body);  // 见 3.2 字段映射
    state.enrichment.aircraft.insert(hex, CachedAircraft { at: now_ms(), data });
    state.enrichment.dirty.store(true, Ordering::Relaxed);
    Ok(json_response(AircraftFound { found: true, ...data }))
}
```

**持久化**：
```rust
#[derive(Serialize, Deserialize)]
struct PersistedCache {
    routes: HashMap<String, PersistedRoute>,
    aircraft: HashMap<String, PersistedAircraft>,
}

impl EnrichmentCache {
    async fn persist(&self) -> Result<()> {
        let p = PersistedCache {
            routes: self.routes.iter().map(|(k, v)| (k.clone(), v.into())).collect(),
            aircraft: self.aircraft.iter().map(|(k, v)| (k.clone(), v.into())).collect(),
        };
        if let Some(parent) = Path::new(CACHE_PATH).parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let tmp = format!("{}.tmp", CACHE_PATH);
        tokio::fs::write(&tmp, serde_json::to_vec_pretty(&p)?).await?;
        tokio::fs::rename(&tmp, CACHE_PATH).await?;  // atomic
        Ok(())
    }
}
```

**测试**（`hub-core/src/gev_enrichment.rs::tests`）：
1. `cache_load_persist_roundtrip` — 写入 → 读回 → 一致
2. `cache_negative_404` — 404 缓存 None → 第二次仍返 found:false
3. `cache_fresh_ttl` — 24h 内走 cache，>24h 重新 upstream
4. `inflight_coalesce` — 100 并发同 key → 1 个 upstream 请求
5. `upstream_5xx_returns_503` — 上游 503 → adapter 503 + 不缓存
6. `parse_route_minimal` — `response.flightroute.{airline,origin,destination}` 提取
7. `parse_aircraft_minimal` — `response.aircraft.{icao_type,manufacturer,type,registration}` 提取
8. `dirty_flush_atomic` — 写盘用 tmp+rename

### 4.B `gev_tracks.rs` — Track backfill 代理

**文件位置**：`hub-core/src/gev_tracks.rs`（new file）  
**注册**：`hub-core/src/lib.rs::modules()` 添加 `register_module("gev_tracks", Box::new(gev_tracks::module()))`

**路由表**：
```rust
async fn route(req: Request, state: AppState) -> Result<Response> {
    let path = req.path();
    if path.starts_with("/api/opensky-track") {
        return handle_opensky_track(req, state).await;
    }
    if path.starts_with("/api/adsblol/trace") {
        return handle_adsblol_trace(req, state).await;
    }
    Err(HubError::not_found("unknown tracks endpoint"))
}
```

**OpenSky OAuth + adaptive TTL**：
```rust
struct OpenSkyClient {
    http: reqwest::Client,
    token: Arc<RwLock<Option<String>>>,
    token_expiry: Arc<RwLock<u64>>,
    inflight_refresh: Arc<Mutex<Option<Arc<OnceCell<()>>>>>,
    adaptive_ttl: AtomicU64,  // ms
    cooldown_until: AtomicU64, // epoch ms
    cache: Arc<Mutex<HashMap<String, CachedTrack>>>,
}

const OPENSKY_CACHE_MS: u64 = 9000;
const TRACK_PROXY_CACHE_MS: u64 = 60000;
const TRACK_PROXY_CACHE_MAX: usize = 200;
const RESPONSE_CAP_BYTES: usize = 5 * 1024 * 1024;

impl OpenSkyClient {
    async fn get_token(&self) -> Result<Option<String>> {
        let now = now_ms();
        {
            let token = self.token.read().await;
            let expiry = self.token_expiry.read().await;
            if token.is_some() && now < *expiry - 60_000 {
                return Ok(token.clone());
            }
        }
        // Coalesce concurrent refresh
        // ... (类似 adsbdb inflight pattern)
        
        let client_id = std::env::var("OPENSKY_CLIENT_ID").ok();
        let client_secret = std::env::var("OPENSKY_CLIENT_SECRET").ok();
        let (Some(cid), Some(sec)) = (client_id, client_secret) else {
            return Ok(None);
        };
        
        let res = self.http.post("https://auth.opensky-network.org/auth/realms/opensky-network/protocol/openid-connect/token")
            .form(&[("grant_type", "client_credentials"),
                    ("client_id", &cid),
                    ("client_secret", &sec)])
            .send().await?;
        let body: serde_json::Value = res.json().await?;
        let token = body["access_token"].as_str().map(String::from);
        let expires_in = body["expires_in"].as_u64().unwrap_or(1800);
        if let Some(ref t) = token {
            *self.token.write().await = Some(t.clone());
            *self.token_expiry.write().await = now + expires_in * 1000;
        }
        Ok(token)
    }
    
    fn adaptive_ttl(remaining: Option<u64>) -> u64 {
        let r = remaining.unwrap_or(u64::MAX);
        if r > 2400 { OPENSKY_CACHE_MS }
        else if r > 1200 { 30_000 }
        else if r > 400 { 90_000 }
        else { 300_000 }
    }
}
```

**关键决策**：
- **Env 缺失时返 503**（不 silent skip）：vendor 期望 503 → silent fallback；silent skip 会让用户以为上游在工作
- **200 stale response with header `X-OpenSky-Stale: 1`**：vendor silent fallback 但 user 看到 200 数据（只是 stale 标记让操作员能识别）
- **`time=0` parameter**（OpenSky `/tracks/all?icao24=X&time=0`）：拿最近历史（vendor `tracking.js:467-560` splice 时机 `oldestFixEpochSec` 决定拿多旧）

**adsb.lol trace** 比 OpenSky 简单：
- 直接 GET `https://adsb.lol/data/traces/{hex末尾2位}/trace_full_{hex}.json`
- 内存 LRU 60s TTL, 200 entries cap
- 无 OAuth

**错误响应统一**：
```rust
async fn handle_opensky_track(req: Request, state: AppState) -> Result<Response> {
    let query: HashMap<String, String> = parse_query(&req);
    let icao24 = query.get("icao24").map(|s| s.to_lowercase()).unwrap_or_default();
    if !HEX6.is_match(&icao24) {
        return Ok(json_error(400, "icao24 must be 6-char hex"));
    }
    
    // 1. Cache hit
    if let Some(entry) = state.tracks.cache.lock().await.get(&icao24) {
        if now_ms() - entry.at < TRACK_PROXY_CACHE_MS {
            return Ok(track_response(200, &entry.body, "HIT"));
        }
    }
    
    // 2. Cooldown check
    let now = now_ms();
    if now < state.tracks.opensky.cooldown_until.load(Ordering::Relaxed) {
        if let Some(entry) = state.tracks.cache.lock().await.get(&icao24) {
            return Ok(track_response(200, &entry.body, "STALE"));
        }
        return Ok(json_error(503, "opensky upstream cooling down"));
    }
    
    // 3. Upstream fetch
    let token = state.tracks.opensky.get_token().await?;
    if token.is_none() {
        return Ok(json_error(503, "OPENSKY_CLIENT_ID/SECRET not configured"));
    }
    
    let url = format!("https://opensky-network.org/api/tracks/all?icao24={}&time=0", icao24);
    let res = state.tracks.opensky.http.get(&url)
        .bearer_auth(token.unwrap())
        .timeout(Duration::from_secs(12))
        .send().await?;
    
    // Read X-Rate-Limit-Remaining header for adaptive TTL
    let remaining = res.headers().get("X-Rate-Limit-Remaining")
        .and_then(|v| v.to_str().ok()).and_then(|s| s.parse::<u64>().ok());
    let new_ttl = OpenSkyClient::adaptive_ttl(remaining);
    state.tracks.opensky.adaptive_ttl.store(new_ttl, Ordering::Relaxed);
    
    if res.status() == 429 {
        let retry_after = res.headers().get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(parse_retry_after_secs)
            .unwrap_or(30_000);
        let cooldown = retry_after.clamp(5_000, 30 * 60 * 1000);
        state.tracks.opensky.cooldown_until.store(now_ms() + cooldown, Ordering::Relaxed);
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
    
    // Cache + return
    let body_str = String::from_utf8_lossy(&body_bytes).to_string();
    state.tracks.cache.lock().await.insert(icao24.clone(), CachedTrack { at: now_ms(), body: body_str.clone() });
    if state.tracks.cache.lock().await.len() > TRACK_PROXY_CACHE_MAX {
        // LRU eviction: remove oldest
        if let Some(oldest) = state.tracks.cache.lock().await.iter().min_by_key(|(_, v)| v.at).map(|(k, _)| k.clone()) {
            state.tracks.cache.lock().await.remove(&oldest);
        }
    }
    Ok(track_response(200, &body_str, "MISS"))
}

fn parse_retry_after_secs(v: &str) -> Option<u64> {
    if let Ok(secs) = v.parse::<u64>() {
        return Some(secs * 1000);
    }
    if let Ok(epoch) = v.parse::<u64>() {
        return Some(epoch.saturating_sub(now_ms() / 1000) * 1000);
    }
    None
}
```

**测试**（`hub-core/src/gev_tracks.rs::tests`）：
1. `oauth_token_cache_within_expiry` — token 续期逻辑
2. `oauth_inflight_coalesce` — 100 并发 refresh → 1 个 upstream 请求
3. `adaptive_ttl_4_tiers` — `>2400/1200/400/else` 4 档
4. `cooldown_429_honors_retry_after` — 429 + Retry-After:30 → cooldown 30s
5. `cooldown_429_clamps` — Retry-After:0 → 5s（最小），Retry-After:7200 → 30min（最大）
6. `serve_stale_during_cooldown` — cooldown 期间返 200 + 上次 cache
7. `cache_lru_eviction` — >200 entries → 删最老
8. `response_cap_5mb` — >5MB → 502
9. `normalizes_track_path` — OpenSky `[time,lat,lon,alt,track,ground]` → 对象数组
10. `env_missing_returns_503` — `OPENSKY_CLIENT_ID` 未设 → 503

### 4.C 镜像化更新

- `hub-core/src/lib.rs::modules()` 列表中加 `gev_enrichment` 和 `gev_tracks`
- `hub-core/Cargo.toml` 无新依赖（reqwest + serde_json + tokio 都已有）
- `core/secrets.env` 模板 `OPENSKY_CLIENT_ID=` `OPENSKY_CLIENT_SECRET=` 占位（与 `core/agent-keys.txt` 同种 user-decision）

## 5. Console 适配层扩展

### 5.1 `console/src/gev-adapters/aircraft-source.ts` (NEW, ~120 LoC)

**不修改** `aircraft-map.ts`（保持 P1 契约 + 现有 14 个 source-contracts 测试不变）。

**新文件**：
```ts
// GEV P12 — aircraft source object (mirror of vendor standalone.js)
//
// Vendor URL contracts (HARDCODED in gev-engine/src/sources/live/standalone.js):
//   Line 86: '/api/opensky-track?icao24=' + encodeURIComponent(reference)
//   Line 99: `/api/adsbdb/${query.kind}/${encodeURIComponent(query.id)}`
// Any path refactor WILL BREAK vendor silently. The path-literal assertions in
// console/src/gev-boot/__tests__/source-contracts.test.ts pin these strings.

import type { ApiFetch } from "./http";
import type { SnapshotEnvelope, GevAircraftRecord } from "./types";
import { toGevRecord, toEnvelope, type AdsbEnvelope } from "./aircraft-map";

export interface TrackRecord {
  observedAtMs: number;
  latitude: number;
  longitude: number;
  baroAltitudeM: number | null;
  courseDeg: number | null;
  onGround: boolean;
}

export type EnrichmentPayload =
  | { found: true; typeCode?: string; typeName?: string; registration?: string }
  | { found: true; airline?: string; origin?: AirportInfo; destination?: AirportInfo }
  | { found: false };

export interface AirportInfo {
  code: string;
  name: string;
  lat: number | null;
  lon: number | null;
}

export interface IntelHubAircraftSource {
  label: string;
  getSnapshot(query: SnapshotQuery, opts: { signal?: AbortSignal }): Promise<SnapshotEnvelope<GevAircraftRecord>>;
  getTrack(reference: string, opts: { signal?: AbortSignal }): Promise<{ records: TrackRecord[]; complete: false }>;
  getEnrichment(query: { kind: "type" | "route"; id: string }, opts: { signal?: AbortSignal }): Promise<EnrichmentPayload>;
}

export interface SnapshotQuery {
  latitude?: number;
  longitude?: number;
}

const HEX6_RE = /^[0-9a-f]{6}$/;
const CALLSIGN_RE = /^[A-Z]{2,8}$/;

export function createIntelHubAircraftSource({ apiFetch }: { apiFetch: ApiFetch }): IntelHubAircraftSource {
  // Constructor contract (P3 lesson): adapter methods are silently broken if
  // apiFetch is missing or non-functional. Assert now so test mocks cannot hide.
  if (typeof apiFetch !== "function") {
    throw new TypeError("createIntelHubAircraftSource: apiFetch must be a function");
  }

  return {
    label: "hub.intelhub",

    async getSnapshot(query, { signal } = {}) {
      const params = new URLSearchParams();
      if (Number.isFinite(query?.latitude) && Number.isFinite(query?.longitude)) {
        params.set("lat", String(query.latitude));
        params.set("lon", String(query.longitude));
      }
      const url = `/api/v1/globe/aircraft${params.toString() ? "?" + params : ""}`;
      const res = await apiFetch(url, { signal });
      if (!res.ok) {
        throw new Error(`aircraft snapshot HTTP ${res.status}`);
      }
      const env = await res.json() as AdsbEnvelope;
      const records = (env.aircraft ?? []).map(p => toGevRecord(p, env.ts ? Date.parse(env.ts) : null));
      return toEnvelope(records, env, "hub.intelhub", Date.now());
    },

    async getTrack(reference, { signal } = {}) {
      const hex = reference.toLowerCase();
      if (!HEX6_RE.test(hex)) {
        // Silent fallback for non-hex (vendor passes icao24 always, but defensive)
        return { records: [], complete: false as const };
      }
      // 8s timeout mirrors vendor (tracking.js:484 AbortSignal.timeout(8000))
      const timeoutSignal = AbortSignal.timeout(8000);
      const composedSignal = signal
        ? AbortSignal.any([signal, timeoutSignal])
        : timeoutSignal;
      try {
        const res = await apiFetch(
          `/api/opensky-track?icao24=${encodeURIComponent(hex)}`,
          { signal: composedSignal },
        );
        if (!res.ok) {
          return { records: [], complete: false as const };
        }
        const body = await res.json();
        return { records: normalizeTrackPath(body), complete: false as const };
      } catch {
        // Silent fallback (vendor tracking.js:486-492 catch { return; })
        return { records: [], complete: false as const };
      }
    },

    async getEnrichment(query, { signal } = {}) {
      if (query.kind !== "type" && query.kind !== "route") {
        throw new Error(`unsupported enrichment kind: ${(query as { kind: string }).kind}`);
      }
      const id = query.kind === "route"
        ? query.id.toUpperCase()
        : query.id.toLowerCase();
      if (query.kind === "type" && !HEX6_RE.test(id)) {
        throw new Error("type enrichment id must be 6-char hex");
      }
      if (query.kind === "route" && !CALLSIGN_RE.test(id)) {
        throw new Error("route enrichment id must be 2-8 char callsign");
      }
      const res = await apiFetch(
        `/api/adsbdb/${query.kind}/${encodeURIComponent(id)}`,
        { signal },
      );
      if (!res.ok) {
        // 4xx (404 etc) → throw, vendor silently skips
        // 5xx → throw, vendor records cooldown
        throw new Error(`adsbdb enrichment HTTP ${res.status}`);
      }
      return await res.json() as EnrichmentPayload;
    },
  };
}

/** Normalize OpenSky /tracks/all response (array-of-arrays) to TrackRecord[].
 *  Vendor's normalizeAircraftTrack does the same in JS — we mirror in TS. */
function normalizeTrackPath(payload: unknown): TrackRecord[] {
  // OpenSky returns { path: [[time, lat, lon, baro_alt, true_track, on_ground], ...] }
  // adsb.lol trace_full_* returns similar shape
  const path = (payload as { path?: unknown[] })?.path;
  if (!Array.isArray(path)) return [];
  const records: TrackRecord[] = [];
  for (const waypoint of path) {
    if (!Array.isArray(waypoint)) continue;
    const [time, lat, lon, baroAlt, track, ground] = waypoint;
    if (!Number.isFinite(time) || !Number.isFinite(lat) || !Number.isFinite(lon)) continue;
    records.push({
      observedAtMs: (time as number) * 1000,
      latitude: lat as number,
      longitude: lon as number,
      baroAltitudeM: Number.isFinite(baroAlt) ? (baroAlt as number) : null,
      courseDeg: Number.isFinite(track) ? (track as number) : null,
      onGround: ground === true,
    });
  }
  return records;
}
```

### 5.2 `console/src/gev-adapters/index.ts` 公开 (改 ~10 行)

```ts
export {
  createIntelHubAircraftSource,
  type IntelHubAircraftSource,
  type SnapshotQuery,
  type TrackRecord,
  type EnrichmentPayload,
} from "./aircraft-source";
```

### 5.3 `console/src/gev-boot/application.ts` 注入 (~10 行改)

```ts
// Existing imports
+ import { createIntelHubAircraftSource } from "../gev-adapters";

  createControls: ({ scene, signal, defer }: any) => {
    const sources = createIntelHubLayerSources({ apiFetch: opts.apiFetch });
+   // P12: inject aircraft source into the catalog (replace stub)
+   const intelAircraft = createIntelHubAircraftSource({ apiFetch: opts.apiFetch });
+   // Wire into the flights source slot — vendor reads `feed._source`
+   sources.flights = intelAircraft;  // (depends on what sources.flights is in createIntelHubLayerSources)
    
    const catalog = createApplicationCatalog({ ... });
    ...
  }
```

(实际 wiring 取决于 `createIntelHubLayerSources` 内部结构 — T1 task 时确认 P1 stub 替换点)

### 5.4 测试

**`console/src/gev-adapters/__tests__/aircraft-source.test.ts`** (NEW, ~180 LoC)：
```ts
describe("createIntelHubAircraftSource", () => {
  let mockFetch: ReturnType<typeof vi.fn>;
  let source: IntelHubAircraftSource;
  
  beforeEach(() => {
    mockFetch = vi.fn();
    source = createIntelHubAircraftSource({ apiFetch: mockFetch as any });
  });

  describe("constructor contract", () => {
    it("throws if apiFetch is missing", () => {
      expect(() => createIntelHubAircraftSource({ apiFetch: null as any })).toThrow(TypeError);
    });
    it("throws if apiFetch is not a function", () => {
      expect(() => createIntelHubAircraftSource({ apiFetch: 42 as any })).toThrow(TypeError);
    });
    it("label is hub.intelhub", () => {
      expect(source.label).toBe("hub.intelhub");
    });
  });

  describe("getTrack", () => {
    it("sends /api/opensky-track with lowercase hex", async () => {
      mockFetch.mockResolvedValue({ ok: true, json: () => Promise.resolve({ path: [[1726845215, 40.69, -74.17, 10500, 91, false]] }) });
      const result = await source.getTrack("4CA9B1");
      expect(mockFetch).toHaveBeenCalledWith("/api/opensky-track?icao24=4ca9b1", expect.any(Object));
      expect(result.records).toHaveLength(1);
      expect(result.records[0]).toMatchObject({ observedAtMs: 1726845215000, latitude: 40.69 });
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
      // Use fake timers to verify AbortSignal.timeout(8000) is composed
      vi.useFakeTimers();
      mockFetch.mockImplementation(() => new Promise(r => setTimeout(r, 10000)));
      const promise = source.getTrack("4ca9b1");
      vi.advanceTimersByTime(8000);
      // 8s timeout fires → fetch rejects → catch returns empty
      await expect(promise).resolves.toEqual({ records: [], complete: false });
      vi.useRealTimers();
    });
    
    it("returns empty for non-hex reference (defensive)", async () => {
      const result = await source.getTrack("not-hex");
      expect(result.records).toEqual([]);
      expect(mockFetch).not.toHaveBeenCalled();
    });
  });

  describe("getEnrichment", () => {
    it("type path: lowercase hex, passes through found:true", async () => {
      mockFetch.mockResolvedValue({ ok: true, json: () => Promise.resolve({ found: true, typeCode: "B738", typeName: "Boeing 737-800" }) });
      const r = await source.getEnrichment({ kind: "type", id: "4CA9B1" });
      expect(mockFetch).toHaveBeenCalledWith("/api/adsbdb/type/4ca9b1", expect.any(Object));
      expect(r).toEqual({ found: true, typeCode: "B738", typeName: "Boeing 737-800" });
    });
    
    it("route path: uppercase callsign, passes through found:true", async () => {
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
      await expect(source.getEnrichment({ kind: "type" as any, id: "4ca9b1" })).resolves.toBeDefined(); // sanity
      await expect(source.getEnrichment({ kind: "weather" as any, id: "x" })).rejects.toThrow(/unsupported/);
    });
  });
});
```

**`console/src/gev-boot/__tests__/source-contracts.test.ts` 增量** (改 ~30 行)：

```ts
+ // P12: aircraft source must expose getTrack + getEnrichment
+ import { createIntelHubAircraftSource } from "../../gev-adapters/aircraft-source";
+
+ describe("intelhub aircraft source contract", () => {
+   it("exposes getTrack method", () => {
+     const src = createIntelHubAircraftSource({ apiFetch: vi.fn() });
+     expect(typeof src.getTrack).toBe("function");
+   });
+   it("exposes getEnrichment method", () => {
+     const src = createIntelHubAircraftSource({ apiFetch: vi.fn() });
+     expect(typeof src.getEnrichment).toBe("function");
+   });
+   it("hardcoded vendor URL strings preserved", () => {
+     // Read the file itself — pin the vendor path literals so a refactor
+     // cannot silently break vendor compatibility (vendor standalone.js:86,99).
+     const fs = require("fs");
+     const content = fs.readFileSync(
+       path.resolve(__dirname, "../../gev-adapters/aircraft-source.ts"),
+       "utf8",
+     );
+     expect(content).toContain("/api/opensky-track?icao24=");
+     expect(content).toMatch(/\/api\/adsbdb\/\$\{?query\.kind\}?\//);
+   });
+ });
```

## 6. Cockpit UX Aggressive 抛光

### 6.E1 键盘快捷键 — `console/src/gev-visual/cockpit/shortcuts.ts` (NEW, ~120 LoC)

```ts
import { useEffect } from "react";
import type { CockpitStore } from "./cockpit-store";
import type { VisionMountHandle } from "./vision-mount";
import type { BriefingHandle } from "./briefing-mount";
import { VISION_MODES, type VisionMode } from "./vision-mount";

export interface CockpitShortcutsOptions {
  store: CockpitStore;
  vision: VisionMountHandle | null;
  briefing: BriefingHandle | null;
  /** Called when Shift+C toggles cockpit hidden state */
  onToggleHidden?: () => void;
  /** Called when Tab is pressed (briefing tab navigation) */
  onNextTab?: () => void;
  /** Called when Shift+Tab is pressed */
  onPrevTab?: () => void;
}

export function useCockpitShortcuts(opts: CockpitShortcutsOptions): void {
  useEffect(() => {
    if (!opts.store) return;
    
    const handler = (e: KeyboardEvent) => {
      // Only act when cockpit is active
      if (!opts.store.getState().active) return;
      
      // Conflict guards
      if (e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement) return;
      if (e.target instanceof HTMLElement && e.target.isContentEditable) return;
      if (e.ctrlKey || e.metaKey || e.altKey) return;
      if (e.repeat) return;
      
      switch (e.key) {
        case "ArrowLeft":
          opts.briefing?.prev();
          e.preventDefault();
          break;
        case "ArrowRight":
          opts.briefing?.next();
          e.preventDefault();
          break;
        case "Escape":
          opts.store.exit();
          e.preventDefault();
          break;
        case " ":
          if (opts.store.getState().briefingPaused) {
            opts.store.resumeBriefing();
          } else {
            opts.store.pauseBriefing();
          }
          e.preventDefault();
          break;
        case "Tab":
          if (e.shiftKey) opts.onPrevTab?.();
          else opts.onNextTab?.();
          e.preventDefault();
          break;
        case "C":
          if (e.shiftKey) {
            opts.onToggleHidden?.();
            e.preventDefault();
          }
          break;
        default: {
          // Number keys 1-5 → vision mode
          const n = parseInt(e.key, 10);
          if (Number.isInteger(n) && n >= 1 && n <= VISION_MODES.length) {
            const mode = VISION_MODES[n - 1] as VisionMode;
            opts.vision?.setMode(mode);
            opts.store.setVisionMode(mode);
            try {
              localStorage.setItem("intelhub.cockpit.visionMode", mode);
            } catch { /* private mode */ }
            e.preventDefault();
          }
        }
      }
    };
    
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [opts.store, opts.vision, opts.briefing, opts.onToggleHidden, opts.onNextTab, opts.onPrevTab]);
}
```

**挂载位置**：`HudCockpitFrame.tsx` 顶层 useCockpitShortcuts

**i18n**：`console/src/i18n/zh.json` + `en.json` 添加：
```json
{
  "cockpit.shortcut.hint": "← → 简报 · Esc 退出 · 1-5 视觉 · Space 暂停 · Tab 切换 · Shift+C 隐藏"
}
```

**底部提示带**：`HudCockpitFrame` 底部新增 1 行小字 `data-testid="hud-cockpit-shortcut-hint"`，仅 cockpit active 显示

**测试**（`shortcuts.test.ts`，~120 LoC，10 cases）：
1. ArrowLeft / ArrowRight → briefing.prev / next 调用
2. Escape → store.exit 调用
3. 1-5 → vision.setMode + store.setVisionMode 调用
4. Space → pause/resume 切换
5. Tab → onNextTab 调用；Shift+Tab → onPrevTab
6. Shift+C → onToggleHidden 调用
7. Inactive 时不响应任何键
8. 表单元素 focus 时不拦截（input/textarea/contentEditable）
9. 组合键（Ctrl/Meta/Alt）不拦截
10. 长按不重复触发（e.repeat=true 跳过）

### 6.E2 相机进出过渡 — `console/src/gev-visual/cockpit/camera-transition.ts` (NEW, ~80 LoC)

```ts
import * as Cesium from "cesium";

export interface CockpitCameraTransition {
  /** Smoothly fly camera to tracked aircraft overhead. Returns when flight completes or aborts. */
  flyToTracked(target: { longitude: number; latitude: number; altitude: number }, opts?: { duration?: number }): Promise<void>;
  /** Smoothly fly back to the pre-cockpit camera state. */
  flyBackToBaseline(opts?: { duration?: number }): Promise<void>;
  /** Manually capture the current camera state as the new baseline. */
  captureBaseline(): void;
  destroy(): void;
}

export interface CockpitCameraTransitionDeps {
  viewer: { scene: { canvas?: unknown }; camera: { flyTo: (opts: any) => Promise<boolean> | boolean; position: any; direction: any; up: any; heading: number; pitch: number; roll: number } };
}

export function mountCockpitCameraTransition(deps: CockpitCameraTransitionDeps): CockpitCameraTransition {
  if (!deps?.viewer?.camera?.flyTo) {
    throw new TypeError("mountCockpitCameraTransition: viewer.camera.flyTo missing");
  }

  let baseline: { position: any; heading: number; pitch: number; roll: number } | null = null;
  let activeFlyPromise: Promise<void> | null = null;
  let destroyed = false;

  async function fly(opts: any): Promise<void> {
    if (destroyed) return;
    // Cancel active fly (Cesium camera.flyTo returns a Promise; cancel via new request)
    try {
      const p = deps.viewer.camera.flyTo({
        ...opts,
        easingFunction: Cesium.EasingFunction.QUADRATIC_IN_OUT,
      });
      if (p instanceof Promise) await p;
    } catch {
      // AbortError silently OK
    }
  }

  return {
    flyToTracked(target, opts = {}) {
      baseline = {
        position: deps.viewer.camera.position.clone(),
        heading: deps.viewer.camera.heading,
        pitch: deps.viewer.camera.pitch,
        roll: deps.viewer.camera.roll,
      };
      // 500m above, 0.5° latitude south offset for tilted view, pitch -20°
      const dest = Cesium.Cartesian3.fromDegrees(
        target.longitude,
        target.latitude - 0.5,
        target.altitude + 1500,
      );
      const orientation = {
        heading: 0,
        pitch: Cesium.Math.toRadians(-20),
        roll: 0,
      };
      activeFlyPromise = fly({ destination: dest, orientation, duration: opts.duration ?? 0.7 });
      return activeFlyPromise;
    },

    flyBackToBaseline(opts = {}) {
      if (!baseline) return Promise.resolve();
      const dest = baseline.position;
      const orientation = {
        heading: baseline.heading,
        pitch: baseline.pitch,
        roll: baseline.roll,
      };
      activeFlyPromise = fly({ destination: dest, orientation, duration: opts.duration ?? 0.7 });
      return activeFlyPromise;
    },

    captureBaseline() {
      baseline = {
        position: deps.viewer.camera.position.clone(),
        heading: deps.viewer.camera.heading,
        pitch: deps.viewer.camera.pitch,
        roll: deps.viewer.camera.roll,
      };
    },

    destroy() {
      destroyed = true;
      baseline = null;
      activeFlyPromise = null;
    },
  };
}
```

**时序协调**（在 `HudCockpitFrame` 中）：
```ts
useEffect(() => {
  if (state.active && state.trackedId && transition) {
    // 1. Start vendor tracking (already done via follow-controller.follow)
    // 2. Wait one frame for vendor to apply tracked camera frame
    // 3. Our flyTo is a SHORT DELTA (not full takeover)
    requestAnimationFrame(async () => {
      const info = getTrackedInfo?.();
      if (info?.longitude != null && info?.latitude != null && info?.altitudeM != null) {
        await transition.flyToTracked({
          longitude: info.longitude,
          latitude: info.latitude,
          altitude: info.altitudeM,
        });
      }
    });
  }
  if (!state.active && transition) {
    // flyBack BEFORE store.exit() so vendor doesn't grab camera mid-flight
    transition.flyBackToBaseline();
  }
}, [state.active, state.trackedId, transition]);
```

**测试**（`camera-transition.test.ts`，~80 LoC）：
1. Constructor contract: missing flyTo throws
2. flyToTracked calls camera.flyTo with correct destination/orientation/duration
3. flyBackToBaseline returns to captured baseline
4. captureBaseline updates baseline
5. destroy clears baseline + nulls promise
6. Concurrent fly calls: new fly cancels old (Cesium camera.flyTo is replaceable)

### 6.E3 Briefing 节奏微调（改 `HudCockpitBriefingPanel.tsx`，~30 行）

**已有**：hover pause/resume（onMouseEnter/Leave）+ 6s auto-rotate（`COCKPIT_BRIEF_ROTATE_MS`）+ tab 切换

**新增**：
```ts
// Manual navigation grace: 5s 内不自动 resume
const manualUntilRef = useRef(0);

// In the auto-rotate effect:
useEffect(() => {
  if (!briefing || paused || briefing.total() === 0) return;
  const timer = setInterval(() => {
    if (Date.now() < manualUntilRef.current) return;  // grace period
    briefing.next();
    setIndex(briefing.index());
  }, COCKPIT_BRIEF_ROTATE_MS);
  return () => clearInterval(timer);
}, [briefing, paused, data]);

// In onNext/onPrev handlers:
const onNext = () => {
  briefing?.next();
  setIndex(briefing?.index() ?? 0);
  manualUntilRef.current = Date.now() + 5000;  // grace 5s
};
const onPrev = () => {
  briefing?.prev();
  setIndex(briefing?.index() ?? 0);
  manualUntilRef.current = Date.now() + 5000;
};
```

**Fade 动画**（CSS-only）：
```css
.hud-cockpit-summary-bullet {
  transition: opacity 200ms ease-in-out, transform 200ms ease-in-out;
}
.hud-cockpit-summary-bullet.fading {
  opacity: 0;
  transform: translateY(-4px);
}
```

**进度条**（auto-rotate 剩余时间）：
```tsx
<div className="hud-cockpit-progress">
  <div className="hud-cockpit-progress-bar" style={progressStyle} />
</div>
```

**测试**：3 cases（manual nav sets grace / during grace auto-rotate skipped / grace expires after 5s）

### 6.E4 Panel 拖拽 + Cockpit-active 锁定（改 `panel-drag.ts`，~80 行）

```ts
// Existing P10 panel-drag interface
export interface PanelDragOptions {
  panelId: string;
  container: HTMLElement;
  handle: HTMLElement;
  /** P12 NEW: when true, drag is no-op */
  disabled?: boolean;
  onPositionChange?: (pos: { x: number; y: number }) => void;
}

export function mountPanelDrag(opts: PanelDragOptions): PanelDragHandle {
  let isDragging = false;
  
  function startDrag(e: PointerEvent) {
    if (opts.disabled) {
      // Defensive: release any in-flight drag if disabled turns true mid-drag
      if (isDragging) endDrag();
      return;
    }
    isDragging = true;
    // ... existing drag start
  }
  
  function endDrag() {
    isDragging = false;
    // ... existing end
  }
  
  // Existing pointermove/pointerup listeners
  
  return {
    destroy() { ... },
    // NEW
    setDisabled(d: boolean) {
      opts.disabled = d;
      if (d && isDragging) endDrag();
    },
  };
}
```

**HUD 集成**：`HudFrame.tsx` 在 cockpit active 时 `panelDrag.setDisabled(true)`，exit 时 `setDisabled(false)`。

**边缘吸附**（已有 verify）+ 拖拽惯性 + 越界 clamp：保留现有 UX（已工作）

**测试**（5 cases）：
1. disabled=true 时不响应 pointerdown
2. drag 中 disabled=true → 立即 release
3. setDisabled(false) 恢复拖拽
4. destroy 清理所有 listeners
5. 边界 clamp 越界坐标 clamp 到 viewport

### 6.E5 Viewport 锁定 — `console/src/gev-visual/cockpit/viewport-lock.ts` (NEW, ~50 LoC)

```ts
import * as Cesium from "cesium";

export interface CockpitViewportLock {
  /** Lock all input to disable accidental map interaction */
  lock(): void;
  /** Restore input and cursor */
  unlock(): void;
  /** Currently locked? */
  isLocked(): boolean;
  destroy(): void;
}

export function mountCockpitViewportLock(viewer: { scene: { screenSpaceCameraController: { enableInputs: boolean } }; cesiumWidget: { canvas: HTMLElement } }): CockpitViewportLock {
  if (!viewer?.scene?.screenSpaceCameraController) {
    throw new TypeError("mountCockpitViewportLock: viewer.scene.screenSpaceCameraController missing");
  }
  if (!viewer?.cesiumWidget?.canvas) {
    throw new TypeError("mountCockpitViewportLock: viewer.cesiumWidget.canvas missing");
  }

  let locked = false;
  let screenSpaceEventHandler: Cesium.ScreenSpaceEventHandler | null = null;

  function blockAllInputs() {
    const canvas = viewer.cesiumWidget.canvas;
    canvas.style.cursor = "none";
    screenSpaceEventHandler = new Cesium.ScreenSpaceEventHandler(canvas);
    const noop = () => {};
    screenSpaceEventHandler.setInputAction(noop, Cesium.ScreenSpaceEventType.LEFT_CLICK);
    screenSpaceEventHandler.setInputAction(noop, Cesium.ScreenSpaceEventType.LEFT_DOUBLE_CLICK);
    screenSpaceEventHandler.setInputAction(noop, Cesium.ScreenSpaceEventType.RIGHT_CLICK);
    // Note: WHEEL/MIDDLE_DRAG already disabled by enableInputs=false below
  }

  function unblockAllInputs() {
    if (screenSpaceEventHandler) {
      screenSpaceEventHandler.destroy();
      screenSpaceEventHandler = null;
    }
    viewer.cesiumWidget.canvas.style.cursor = "";
  }

  return {
    lock() {
      if (locked) return;
      viewer.scene.screenSpaceCameraController.enableInputs = false;
      blockAllInputs();
      locked = true;
    },
    unlock() {
      if (!locked) return;
      viewer.scene.screenSpaceCameraController.enableInputs = true;
      unblockAllInputs();
      locked = false;
    },
    isLocked: () => locked,
    destroy() {
      if (locked) this.unlock();
    },
  };
}
```

**挂载**：`HudCockpitFrame` 进入 cockpit → `viewportLock.lock()`；退出 → `unlock()`

**Touch 行为**：mobile `matchMedia('(pointer: coarse)')` 时不锁 single-finger（让 briefing tap 仍工作），只禁 drag rotate / pinch zoom（已通过 `enableInputs=false`）

**测试**（5 cases）：
1. lock: enableInputs=false + cursor="none"
2. unlock: enableInputs=true + cursor=""
3. double-lock no-op
4. double-unlock no-op
5. destroy auto-unlock + cleanup ScreenSpaceEventHandler

## 7. 验收基线扩展

### 7.1 sp6 baseline（geographic + 实时数据层）

**当前**：39+5sh/0f（共 44 项）  
**目标**：41+5sh/0f（共 46 项，+2 新增）

**新增**（`scripts/accept-sp6.py`）：
```python
def check_42_adsbdb_route(self):
    """GET /api/adsbdb/route/UAL123 → 200 + body shape"""
    res = self.fetch("/api/adsbdb/route/UAL123")
    assert res.status == 200, f"expected 200, got {res.status}"
    body = res.json()
    assert "found" in body, "missing 'found' key"
    if body["found"]:
        for k in ("airline", "origin", "destination"):
            assert k in body, f"missing {k}"
        for port_name in ("origin", "destination"):
            port = body[port_name]
            for k in ("code", "name", "lat", "lon"):
                assert k in port, f"{port_name} missing {k}"
                if k in ("lat", "lon"):
                    assert isinstance(port[k], (int, float)), f"{port_name}.{k} not numeric"
    # 注意：如果 adsbdb 没有 UAL123 的数据，返 {found: false} 也合法
    # 字段缺失才是 fail

def check_43_opensky_track(self):
    """GET /api/opensky-track?icao24=4ca9b1 → 200 + records array shape"""
    TEST_HEX = "4ca9b1"  # 静态 hex，GEV/intelhub 都验证过 active
    res = self.fetch(f"/api/opensky-track?icao24={TEST_HEX}")
    # 空 records 也合法（hex 可能不在 OpenSky track DB）
    assert res.status == 200, f"expected 200, got {res.status}"
    body = res.json()
    assert "records" in body and isinstance(body["records"], list), \
        f"missing or non-list 'records'"
    if len(body["records"]) > 0:
        # Spot-check first 3 records
        for rec in body["records"][:3]:
            for k in ("observedAtMs", "latitude", "longitude"):
                assert k in rec, f"record missing {k}"
            assert isinstance(rec["observedAtMs"], int) and rec["observedAtMs"] > 0
            assert -90 <= rec["latitude"] <= 90
            assert -180 <= rec["longitude"] <= 180
```

**运行方式**：`python3 scripts/accept-sp6.py "$KEY"`，从 Mac 跑，screenshot 结果。

### 7.2 sp8 baseline（前端 UI / adapter）

**当前**：48+2sh/0f（共 50 项）  
**目标**：53+2sh/0f（共 55 项，+5 新增）

**新增**（`scripts/accept-sp8.py`）：
```python
def check_49_aircraft_source_getTrack(self):
    """Console adapter 暴露 getTrack（dist bundle 含方法名）"""
    bundle = self.vm.read_console_dist_assets()
    assert "getTrack" in bundle, "aircraft adapter getTrack not in dist"
    assert "/api/opensky-track" in bundle, "vendor URL path literal not in dist"

def check_50_aircraft_source_getEnrichment(self):
    """Console adapter 暴露 getEnrichment"""
    bundle = self.vm.read_console_dist_assets()
    assert "getEnrichment" in bundle, "aircraft adapter getEnrichment not in dist"
    assert "/api/adsbdb" in bundle, "adsbdb URL path literal not in dist"

def check_51_cockpit_keyboard_shortcut(self):
    """cockpit active 时键盘 ← → → briefing.next/prev 调用"""
    # 走 probe-gev 风格（puppeteer 或 Playwright）
    result = self.vm.probe_cockpit_keyboard()
    assert result["left_arrow_calls_prev"] is True
    assert result["right_arrow_calls_next"] is True
    assert result["escape_calls_exit"] is True

def check_52_cockpit_camera_transition(self):
    """enter cockpit 调 viewer.camera.flyTo, duration 0.6-0.8s"""
    result = self.vm.probe_cockpit_camera()
    assert result["flyTo_called"] is True
    assert 0.6 <= result["duration"] <= 0.8

def check_53_cockpit_viewport_lock(self):
    """enter cockpit: scene.screenSpaceCameraController.enableInputs = false"""
    result = self.vm.probe_cockpit_viewport()
    assert result["enableInputs_false"] is True
    assert result["cursor_none"] is True
```

**运行方式**：`python3 scripts/accept-sp8.py "$KEY"`，probe 脚本：`scripts/probe-cockpit-p12.mjs` 在 VM 上跑（Playwright + console dist bundle）。

### 7.3 probe-gev.mjs 扩展

**新增 P12_PROBES**（`console/probe-gev.mjs` 末追加）：
```js
const P12_PROBES = [
  ["p12-aircraft-source-getTrack",       () => !!window.__gevAircraftSource?.getTrack],
  ["p12-aircraft-source-getEnrichment",  () => !!window.__gevAircraftSource?.getEnrichment],
  ["p12-cockpit-active",                 () => !!document.querySelector('[data-testid="hud-cockpit-frame"]')],
  ["p12-cockpit-exit-btn",               () => !!document.querySelector('[data-testid="hud-cockpit-exit"]')],
  ["p12-cockpit-vision-keys",            () => document.querySelectorAll('[data-testid^="hud-cockpit-vision-"]').length === 5],
  ["p12-cockpit-shortcut-hint",          () => /← →/.test(document.body.innerText)],
  ["p12-cockpit-viewport-lock",          () => window.__cockpitStore?.getState?.().viewportLocked === true],
  ["p12-tracks-endpoint-reachable",      async () => {
    const res = await fetch("/api/opensky-track?icao24=4ca9b1");
    return res.status === 200;
  }],
];
```

### 7.4 sp3 / sp2a / sp2b / sp4 / sp5 / sp7 / sp9 不变

无新功能对 MCP / agent / schema / providers 等影响。

### 7.5 315→410 验收序列

按 AGENTS.md：
1. worktree `feat/p12-flight-layer-parity` → 4 phase 串行实施
2. 315 → rsync → build-hub.sh → build-console.sh → restart hub-core → wait 5min → sp6 41+5sh/0f + sp8 53+2sh/0f 全绿
3. main → merge --no-ff → 410 → build-hub.sh → build-console.sh → restart hub-core → wait 5min → 生产 sp6 + sp8 全绿
4. `git push origin main`

## 8. 风险登记与对策

| ID | 风险 | 等级 | 对策 |
|---|---|---|---|
| R1 | OpenSky OAuth 上游契约漂移 | 中 | token parse 防御性 + 测试断言必填字段 + env 缺失 fail-fast 503 |
| R2 | adsbdb 速率限制（首次 cold cache burst） | 低 | in-flight coalesce + 永久负缓存 + dirty 串行刷盘 |
| R3 | Vendor URL 路径写死（refactor 静默破坏） | **关键** | source-contracts.test.ts 加路径字面量断言 |
| R4 | Cesium camera.flyTo + vendor tracking 冲突 | **关键** | enter 时机：vendor 1 帧后再 flyTo（仅短距离 delta）；exit 时机：flyBack resolve 后再 store.exit |
| R5 | Cockpit-active 时 Cesium selection 事件 | 中 | viewport-lock.ts 在 lock() 时 setInputAction(noop, LEFT_CLICK/DOUBLE_CLICK/RIGHT_CLICK) |
| R6 | 快速 enter/exit race（< 0.7s 飞行时间内） | 低 | _activeFlyPromise 维护，新 fly 取消旧的；AbortError silent |
| R7 | panel-drag 锁定后未释放的拖拽 | 低 | disabled=true 时同步 endDrag() 释放 pointer capture |
| R8 | briefing fade + manual nav race | 低 | CSS transition + manual nav 重置 transform/opacity |
| R9 | i18n 遗漏 | 低 | 双语字典 + Linter 检查所有新增 cockpit 文案 |
| R10 | acceptance 静态 vs runtime probe | 中 | sp8 check_51-53 用 Playwright（验证 sp8 现有 baseline 是否支持），如果不支持先标 acceptance-deferred |

### R3 关键路径断言细节

```ts
// console/src/gev-boot/__tests__/source-contracts.test.ts
it("hardcoded vendor URL strings preserved in aircraft adapter", () => {
  const fs = require("fs");
  const path = require("path");
  const content = fs.readFileSync(
    path.resolve(__dirname, "../../gev-adapters/aircraft-source.ts"),
    "utf8",
  );
  // vendor standalone.js:86 hardcoded URL
  expect(content).toContain("/api/opensky-track?icao24=");
  // vendor standalone.js:99 hardcoded URL pattern
  expect(content).toMatch(/api\/adsbdb\/\$\{query\.kind\}/);
});
```

如果 refactor 改了路径，**这个测试立即 fail**，CI 红灯。

### R4 关键时序细节

`HudCockpitFrame.tsx`：
```ts
useEffect(() => {
  if (state.active && state.trackedId && cameraTransition) {
    requestAnimationFrame(async () => {
      const info = getTrackedInfo?.();
      if (info?.longitude != null && info?.latitude != null && info?.altitudeM != null) {
        await cameraTransition.flyToTracked({...});
      }
    });
  } else if (!state.active && cameraTransition) {
    // flyBack BEFORE store.exit() — vendor doesn't grab camera mid-flight
    cameraTransition.flyBackToBaseline();
  }
}, [state.active, state.trackedId, cameraTransition]);
```

注意：`store.exit()` 本身在 keyboard Esc handler 里调用，cameraTransition.flyBackToBaseline 在 useEffect 触发（state.active 变 false 时）。这与 keyboard Esc 解耦。

## 9. Worktree 流程与执行

### 9.1 Worktree 准备

```bash
cd /Volumes/TBU/Workspace/IntelHub
git worktree add ../IntelHub-p12 -b feat/p12-flight-layer-parity
sleep 4  # network disk sync delay
cd ../IntelHub-p12
```

### 9.2 4 Phase 串行实施

| Phase | 工作 | 估算 | sp 检查 |
|---|---|---|---|
| 1. Backend | gev_enrichment.rs (~250 LoC) + gev_tracks.rs (~350 LoC) + tests + lib.rs + secrets.env | ~600 LoC + 18 tests | sp6 现有全绿 |
| 2. Console 适配 | aircraft-source.ts (~120 LoC) + index.ts re-export + gev-boot wiring + tests | ~370 LoC | sp6 41+5sh (+2) |
| 3. Cockpit UX | shortcuts.ts (~120) + camera-transition.ts (~80) + viewport-lock.ts (~50) + briefing 微调 (~30) + panel-drag 改 (~80) + tests (~340) | ~700 LoC | sp8 53+2sh (+5) |
| 4. 验收 | probe-gev P12_PROBES + accept-sp6 check_42/43 + accept-sp8 check_49-53 + i18n + spec cleanup | ~250 LoC | 全绿 |

### 9.3 315 验收

```bash
rsync -az --delete <excludes> ../IntelHub-p12/ Debian-test:/home/zou/IntelHub/
ssh Debian-test 'cd /home/zou/IntelHub && \
  bash scripts/build-hub.sh && bash scripts/build-console.sh && \
  sudo systemctl restart hub-core && sleep 300 && \
  systemctl is-active hub-core'
KEY=$(ssh Debian-test 'grep -o "ihk_[a-f0-9]*" /home/zou/IntelHub/core/agent-keys.txt | head -1')
python3 scripts/accept-sp6.py "$KEY"   # 41+5sh/0f
python3 scripts/accept-sp8.py "$KEY"   # 53+2sh/0f
```

### 9.4 合并 + 410 部署

```bash
git add -A && git commit -m "feat(gev-p12): flight layer parity — backend enrichment + track backfill + cockpit aggressive polish"
cd /Volumes/TBU/Workspace/IntelHub && git merge --no-ff feat/p12-flight-layer-parity
git worktree remove ../IntelHub-p12 && git branch -d feat/p12-flight-layer-parity
# 410 rsync + build + restart + 5min wait + sp6+sp8 验收
git push origin main
```

### 9.5 Doc 同步

- `docs/superpowers/specs/2026-09-21-gev-p12-flight-layer-design.md` ← 本文档
- `docs/agents/issue-tracker.md` 加 P12 closed entry（沿用 P11 precedent）
- AGENTS.md 不变（P12 是常规分期，无新铁律）

## 10. 已知边界

- adsbdb 数据不全（小型 GA tail 没 route / type）→ 返 `{found: false}` 是合法，UI 显示「未知」
- OpenSky `/tracks/all` 对未飞行的 hex 返 404 → adapter silent fallback 到本地累积 trail
- 键盘快捷键在 iframe 中需要 `window.top`（如果未来有 embed）—— 本期不做
- Touch 单指点击 briefing 切换：只在 touch 设备激活（`matchMedia('(pointer: coarse)')`）
- briefing manual nav 5s grace 期间 vendor 仍在 drip enrichment（无影响）
- Cesium 相机 flyTo 在弱 GPU 上 0.7s 可能掉帧（属于 Cesium 自身行为，不修）
- cockpit 退出后 vision mode 持久化到 localStorage `intelhub.cockpit.visionMode`（已有 D2 P9 实现）
- panel drag 锁定只对当前 cockpit session 有效；用户退出 cockpit → 立即解锁
- R10 acceptance 静态 vs runtime：如果 sp8 baseline 不支持 Playwright，check_51-53 标 deferred（P13 处理）
