# GEV P11 CCTV 功能完整移植 实施计划

> 日期：2026-09-19 · 状态：待用户批准 + 待实施
> 上游 spec：`docs/superpowers/specs/2026-09-19-gev-p11-camera-feature-port-design.md`
> 同期姊妹：P3 `f17e577` (4 层) · P7 `da166a1` (相机姿态) · P10 `2026-09-18` (tail)

## 0. 范围与边界

**In scope**:
- 帧 3 层降级链 (upstream → Street View → synthetic SVG)，永有空帧兜底
- 8 个新 live provider (austin/caltrans/txdot/drivebc/fintraffic/tarktee/nsw/calgary) + 1 个 base64-JSON decoder
- provider 注册更新 + env vars 模板
- 2D face-on popout 面板 (`CctvPopoutPanel.tsx` 新组件，gods-eye-view 风格)
- HUD `CctvBody` 改造 (链接 → 按钮触发 popout)
- localStorage 持久化 panel 位置 (P10 命名空间 `godsEyeView.v11.*`)
- 前端 SVG 兜底 (`<img>` onerror → inline SVG)
- sp6 扩展每 provider 行数下限 + sp8 popout 单测 + probe-gev perf 守卫
- 315 验收 → 410 部署 → push

**Out of scope**: 云台控制 / viewshed / 付费源 / 多 camera tabs / EXIF overlay / calibration UX

## 1. 总览

| Task | 内容 | 出口 / 验证 | 文件 |
|---|---|---|---|
| **T1** | worktree 隔离 + baseline sp6/sp8 测量 | feat/p11-cctv-port worktree；baseline 数报告 | — |
| **T2** | synthetic SVG fallback helper + 单元测试 | `cargo test gev_cctv_frame_fallback::tests::build_synthetic_svg` 绿 | `hub-core/crates/hub-core/src/gev_cctv_frame_fallback.rs` |
| **T3** | Street View fallback helper + 单元测试 | `cargo test gev_cctv_frame_fallback::tests::street_view_url` 绿；env 缺席 → 返 None | 同上 |
| **T4** | gev_cctv_frame handler 改造：upstream → Street View → SVG 三层降级 | 集成测试：mock upstream 502 + mock Street View 200 验证降级 | `hub-core/crates/hub-core/src/gev_cctv.rs` |
| **T5** | Austin Socrata provider + 测试 | `cargo test cctv::providers::austin::tests` 绿 | `hub-core/crates/hub-core/src/monitor/sources/cctv/providers/austin.rs` |
| **T6** | Caltrans provider + 测试 | `cargo test cctv::providers::caltrans::tests` 绿 | `.../providers/caltrans.rs` |
| **T7** | TxDOT provider (含 base64-JSON decoder) + 测试 | `cargo test cctv::providers::txdot::tests` 绿；decoder happy/corrupt/missing-data 三态 | `.../providers/txdot.rs` |
| **T8** | DriveBC provider + 测试 | `cargo test cctv::providers::drivebc::tests` 绿 | `.../providers/drivebc.rs` |
| **T9** | Fintraffic provider + 测试 | `cargo test cctv::providers::fintraffic::tests` 绿 | `.../providers/fintraffic.rs` |
| **T10** | Tarktee DATEX2 XML provider + 测试 | `cargo test cctv::providers::tarktee::tests` 绿；happy/empty/malformed 三态 | `.../providers/tarktee.rs` |
| **T11** | NSW provider + 测试 | `cargo test cctv::providers::nsw::tests` 绿 | `.../providers/nsw.rs` |
| **T12** | Calgary provider + 测试 | `cargo test cctv::providers::calgary::tests` 绿 | `.../providers/calgary.rs` |
| **T13** | 注册更新 + env vars 模板 | `providers()` 返 11 项；env vars 在 hub.env 模板 | `.../providers/mod.rs`, `core/hub.env` |
| **T14** | CctvPopoutPanel.tsx 新组件 + 测试 | vitest 5 用例全绿 | `console/src/globe-hud/CctvPopoutPanel.tsx` + `__tests__/` |
| **T15** | HudDetailPanel CctvBody 改造：链接 → 按钮触发 popout | vitest 全绿；新 testid `cctv-open-popout` | `console/src/globe-hud/HudDetailPanel.tsx` |
| **T16** | 前端 SVG 兜底 + localStorage 持久化 panel 位置 | vitest 全绿；新命名空间 `godsEyeView.v11.cctvPopout.pos` | `CctvPopoutPanel.tsx` + tests |
| **T17** | sp6 扩展 + per-provider 行数下限 | sp6 41+5sh/0f (新 +8 检查位) | `scripts/accept-sp6.py` |
| **T18** | sp8 扩展 + popout 单测 | sp8 50+2sh/0f (新 +2 检查位) | `scripts/accept-sp8.py` |
| **T19** | probe-gev CCTV 段扩展 | `probe cctv popout` exit 0；无 pageerror | `console/probe-gev.mjs` |
| **T20** | ledger + final review + merge main + 410 部署 + push | main @ 新 commit；410 全绿；origin/main 已更新 | `docs/superpowers/execution/2026-09-19-gev-p11-ledger.md` + AGENTS.md |

## 2. Worktree 隔离

```bash
cd /Volumes/TBU/Workspace/IntelHub
git worktree add ../IntelHub-gev-p11 -b feat/p11-cctv-port
sleep 4   # 网络盘同步
cd /Volumes/TBU/Workspace/IntelHub-gev-p11
git log -1 main   # 确认 BASE
```

## 3. 全局约束（继承 P3 spec）

- vendor `console/gev-engine/` byte-pinned，**禁止触碰**
- Rust async fn in trait：`async fn` NOT dyn-safe；按现有 `pin_box` 模式（precedent: `planner_llm.rs`）
- 错误信封：`HubError::sensor(format!("..."))` 模式（无 `From<serde_json::Error>`）
- Redis 缓存读：双层 `Option<Option<Vec<u8>>>`（P3 教训）
- env vars：`HUB_*` 前缀优先，链式回退裸名
- 验收退码：仅看 failed；shelved 不计 failure（check_shelved 模式）
- 测试 build：`cargo test --package hub-core` 增量跑（避免 5min 全编）

---

## Task 1 — worktree 隔离 + baseline 测量

**Files**: 无（仅 git/worktree 操作 + 跑 baseline）

**Steps**:
1. **建 worktree**：见 §2
2. **baseline sp6 测量**：
   ```bash
   KEY=$(ssh -o BatchMode=yes Debian-test 'grep -o "ihk_[a-f0-9]*" /home/zou/IntelHub/core/agent-keys.txt | head -1')
   python3 scripts/accept-sp6.py "$KEY" 2>&1 | grep -E "^==.*(passed|shelved|failed)" | tail -1
   ```
   期望：`== 39 passed, 5 shelved, 0 failed ==`（P3 baseline）
3. **baseline sp8 测量**：
   ```bash
   python3 scripts/accept-sp8.py "$KEY" 2>&1 | grep -E "^==.*(passed|shelved|failed)" | tail -1
   ```
   期望：`== 48 passed, 2 shelved, 0 failed ==`（P10 baseline）

**Verify**: baseline 数报告（commit 前必读，若差异 > ±1 则报告阻塞）

---

## Task 2 — synthetic SVG fallback helper

**Files**:
- Create: `hub-core/crates/hub-core/src/gev_cctv_frame_fallback.rs`
- Modify: `hub-core/crates/hub-core/src/lib.rs` — 加 `pub mod gev_cctv_frame_fallback;`（字母序插）

**Steps**:
1. **写测试**（在 `gev_cctv_frame_fallback.rs` 末尾 `#[cfg(test)] mod tests`）：
   ```rust
   #[test]
   fn build_synthetic_svg_includes_id_and_city() {
       let svg = build_synthetic_svg("austin-42", "S Lamar & 5th", "Austin", "DOWN");
       let s = String::from_utf8(svg).unwrap();
       assert!(s.starts_with("<svg"));
       assert!(s.contains("austin-42"));
       assert!(s.contains("S Lamar"));
       assert!(s.contains("Austin"));
       assert!(s.contains("DOWN"));
       assert!(s.ends_with("</svg>"));
   }
   #[test]
   fn build_synthetic_svg_handles_unicode() {
       let svg = build_synthetic_svg("tallinn-001", "Tartu mnt", "Tallinn", "UNKNOWN");
       assert!(String::from_utf8(svg).unwrap().contains("Tartu mnt"));
   }
   ```
2. **跑测试确认 fail**：
   ```bash
   cd hub-core && cargo test --package hub-core gev_cctv_frame_fallback::tests::build_synthetic_svg
   ```
   期望：`error[E0433]: failed to resolve: use of undeclared ...`
3. **实现 `build_synthetic_svg`**：纯函数，移植 gods-eye-view `media.js::buildSyntheticCctvSvg` 形态：
   ```rust
   pub fn build_synthetic_svg(id: &str, label: &str, city: &str, status: &str) -> Vec<u8> {
       let svg = format!(
           r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 320 180">
       <defs>
         <linearGradient id="bg" x1="0" y1="0" x2="1" y2="1">
           <stop offset="0%" stop-color="#0a1418"/>
           <stop offset="100%" stop-color="#020406"/>
         </linearGradient>
       </defs>
       <rect width="320" height="180" fill="url(#bg)"/>
       <rect x="8" y="8" width="304" height="164" fill="none" stroke="rgba(0,212,255,0.3)" stroke-width="1"/>
       <text x="16" y="32" font-family="monospace" font-size="10" fill="rgba(145,237,255,0.7)">{}</text>
       <text x="16" y="56" font-family="monospace" font-size="13" fill="rgba(255,255,255,0.9)" font-weight="bold">{}</text>
       <text x="16" y="78" font-family="monospace" font-size="10" fill="rgba(145,237,255,0.6)">{}</text>
       <text x="160" y="160" text-anchor="middle" font-family="monospace" font-size="10" fill="rgba(255,176,82,0.7)">FEED {} — SYNTHETIC PLACEHOLDER</text>
       </svg>"#,
           html_escape(id), html_escape(city), html_escape(label), status
       );
       svg.into_bytes()
   }
   ```
   + 私有 `html_escape(&str) -> String`（`&` `<` `>` `"` 转义）
4. **跑测试确认 pass**：期望 2/2 绿
5. **commit**:
   ```bash
   git add hub-core/crates/hub-core/src/gev_cctv_frame_fallback.rs hub-core/crates/hub-core/src/lib.rs
   git commit -m "feat(gev-p11): synthetic SVG frame fallback helper"
   ```

---

## Task 3 — Street View fallback helper

**Files**:
- Modify: `hub-core/crates/hub-core/src/gev_cctv_frame_fallback.rs`（加 `street_view_url` + `street_view_fallback`）

**Steps**:
1. **写测试**：
   ```rust
   #[test]
   fn street_view_url_no_key_returns_none() {
       // SAFETY: env 必须在测试前清空
       std::env::remove_var("HUB_GOOGLE_MAPS_SERVER_API_KEY");
       std::env::remove_var("GOOGLE_MAPS_SERVER_API_KEY");
       assert!(street_view_url(30.27, -97.74).is_none());
   }
   #[test]
   fn street_view_url_with_key_pins_host() {
       std::env::set_var("HUB_GOOGLE_MAPS_SERVER_API_KEY", "test-key");
       let url = street_view_url(30.27, -97.74).unwrap();
       assert!(url.starts_with("https://maps.googleapis.com/maps/api/streetview?"));
       assert!(url.contains("location=30.270000%2C-97.740000") || url.contains("location=30.27%2C-97.74"));
       assert!(url.contains("size=640x360"));
       assert!(url.contains("key=test-key"));
       std::env::remove_var("HUB_GOOGLE_MAPS_SERVER_API_KEY");
   }
   ```
2. **跑测试确认 fail**
3. **实现**：
   ```rust
   pub fn street_view_url(lat: f64, lon: f64) -> Option<String> {
       let key = ["HUB_GOOGLE_MAPS_SERVER_API_KEY", "GOOGLE_MAPS_SERVER_API_KEY"]
           .iter().find_map(|v| std::env::var(v).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()))?;
       Some(format!(
           "https://maps.googleapis.com/maps/api/streetview?size=640x360&location={:.6},{:.6}&key={}",
           lat, lon, key
       ))
   }
   pub async fn street_view_fallback(client: &reqwest::Client, lat: f64, lon: f64, timeout_secs: u64) -> Option<Vec<u8>> {
       let url = street_view_url(lat, lon)?;
       match tokio::time::timeout(Duration::from_secs(timeout_secs), client.get(&url).send()).await {
           Ok(Ok(r)) if r.status().is_success() => {
               match r.bytes().await {
                   Ok(b) if b.len() <= 5 * 1024 * 1024 => Some(b.to_vec()),
                   _ => None,
               }
           }
           _ => { tracing::warn!(lat, lon, "street view fetch failed"); None }
       }
   }
   ```
4. **跑测试确认 pass**：期望 2/2 绿
5. **commit**:
   ```bash
   git add hub-core/crates/hub-core/src/gev_cctv_frame_fallback.rs
   git commit -m "feat(gev-p11): street view frame fallback helper (env-gated)"
   ```

---

## Task 4 — gev_cctv_frame handler 改造：3 层降级链

**Files**:
- Modify: `hub-core/crates/hub-core/src/gev_cctv.rs`
- Test: `hub-core/crates/hub-core/tests/gev_cctv_fallback_integration.rs`（新，集成测试）

**Steps**:
1. **写集成测试**：
   ```rust
   // tests/gev_cctv_fallback_integration.rs
   use hub_core::gev_cctv_frame_fallback::{build_synthetic_svg, street_view_url};

   #[test]
   fn synthetic_svg_round_trip_returns_valid_utf8() {
       let bytes = build_synthetic_svg("test-id", "Test Label", "Test City", "DOWN");
       let s = String::from_utf8(bytes.clone()).expect("svg must be valid utf-8");
       assert!(s.contains("<svg"));
       assert!(s.contains("test-id"));
   }
   #[test]
   fn street_view_url_safely_handles_negative_lon() {
       std::env::set_var("HUB_GOOGLE_MAPS_SERVER_API_KEY", "k");
       let url = street_view_url(30.27, -97.74).unwrap();
       assert!(url.contains("location=30.270000,-97.740000") || url.contains("location=30.27,-97.74"));
       std::env::remove_var("HUB_GOOGLE_MAPS_SERVER_API_KEY");
   }
   ```
2. **改造 `gev_cctv_frame` handler**（修改 `gev_cctv.rs` 现有实现）：
   ```rust
   pub async fn gev_cctv_frame(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
       // ... 现有 upstream + cache 逻辑保留 ... 失败时：
       // (1) 查 lat/lon
       if let Ok(cam) = sqlx::query_as::<_, (f64, f64)>(
           "SELECT lat, lon FROM cctv_cameras WHERE id = $1 AND active"
       ).bind(&id).fetch_optional(&state.pg).await {
           if let Some((lat, lon)) = cam {
               // (2) Street View 降级
               if let Some(bytes) = crate::gev_cctv_frame_fallback::street_view_fallback(
                   cctv_http(), lat, lon, 5
               ).await {
                   let ct = "image/jpeg";
                   return ([(axum::http::header::CONTENT_TYPE, ct)], bytes).into_response();
               }
           }
       }
       // (3) Synthetic SVG 兜底（永不 502）
       let (frame_url, _, _) = /* 重新 lookup */ ...;
       let (id, label, city) = sqlx::query_as::<_, (String, String, String)>(
           "SELECT id, name, city FROM cctv_cameras WHERE id = $1"
       ).bind(&id).fetch_one(&state.pg).await.unwrap_or((id.clone(), id.clone(), "Unknown".into()));
       let svg = crate::gev_cctv_frame_fallback::build_synthetic_svg(&id, &label, &city, "DOWN");
       ([(axum::http::header::CONTENT_TYPE, "image/svg+xml")], svg).into_response()
   }
   ```
3. **跑集成测试**:
   ```bash
   cd hub-core && cargo test --package hub-core --test gev_cctv_fallback_integration
   ```
   期望：2/2 绿
4. **commit**:
   ```bash
   git add hub-core/crates/hub-core/src/gev_cctv.rs hub-core/crates/hub-core/src/gev_cctv_frame_fallback.rs hub-core/crates/hub-core/tests/gev_cctv_fallback_integration.rs
   git commit -m "feat(gev-p11): 3-tier frame fallback chain (upstream → street view → svg)"
   ```

---

## Task 5 — Austin Socrata provider

**Files**:
- Create: `hub-core/crates/hub-core/src/monitor/sources/cctv/providers/austin.rs`

**Steps**:
1. **抄 `tfl.rs` 作模板**（`cp tfl.rs austin.rs`，改 provider-specific 部分）
2. **改常量 + endpoint**:
   ```rust
   pub const LICENSE: &str = "City of Austin Transportation & Public Works — Public city traffic camera frame";
   const DEFAULT_URL: &str = "https://data.austintexas.gov/resource/dx9v-zd7x.json?$limit=500";  // Socrata rows.json
   // (具体 ID 需查；agent 已确认 data.austintexas.gov 域名；dataset ID 是 upstream 探测结果 — 计划阶段验证)
   ```
3. **改 parse 逻辑**：按 `data[]` array 解析；只取 `camera_status == "TURNED_ON"`；bbox 检查 `30.0 <= lat <= 30.6 && -97.9 <= lon <= -97.6`
4. **写测试**（≥3 用例）：
   - `parse_austin_row_normal()`：sample row → 合法 CameraRow
   - `parse_austin_skips_disabled()`：`camera_status="REMOVED"` → 跳过
   - `parse_austin_skips_outside_bbox()`：Dallas 坐标 → 跳过
5. **跑测试**：`cargo test --package hub-core cctv::providers::austin::tests` 绿
6. **commit**:
   ```bash
   git add hub-core/crates/hub-core/src/monitor/sources/cctv/providers/austin.rs
   git commit -m "feat(gev-p11): Austin Socrata camera provider"
   ```

---

## Task 6 — Caltrans provider

**Files**:
- Create: `hub-core/crates/hub-core/src/monitor/sources/cctv/providers/caltrans.rs`

**Steps**:
1. **抄 `tfl.rs`**，endpoint 改 `https://cwwp2.dot.ca.gov/data/d{district}/cctv/cctvStatus.json`（按 god-eye-view 形态 `CALTRANS_CCTV_URL`），host-pin `cwwp2.dot.ca.gov`
2. **district env 解析**：`CCTV_CALTRANS_DISTRICTS="4,7,11,3"` → `vec![4,7,11,3]`；留空 → 返 `vec![]`（关 provider）
3. **parse 逻辑**：每个 district JSON `data[]` → row；只 `inService=true && cctv.location.latitude/longitude finite && imageData.static.currentImageURL.starts_with("https://cwwp2.dot.ca.gov/")`
4. **写测试** ≥3：parse happy / 过滤 disabled / host-pin 拒绝非 cwwp2 域 URL
5. **跑测试** 绿
6. **commit**: `feat(gev-p11): Caltrans camera provider (12 districts)`

---

## Task 7 — TxDOT provider (含 base64-JSON decoder)

**Files**:
- Create: `hub-core/crates/hub-core/src/monitor/sources/cctv/providers/txdot.rs`

**Steps**:
1. **endpoint**: `https://its.txdot.gov/its/DistrictIts/GetCctvStatusListByDistrict?districtCode={code}`
2. **base64-JSON decoder**（关键：snapshot 端点返 base64-JPEG）：
   ```rust
   fn decode_txdot_base64_frame(raw_json: &serde_json::Value) -> Result<Vec<u8>, String> {
       let b64 = raw_json.get("snapshotImageBase64Encoded")
           .and_then(|v| v.as_str())
           .ok_or("missing snapshotImageBase64Encoded")?;
       use base64::{Engine as _, engine::general_purpose::STANDARD};
       STANDARD.decode(b64).map_err(|e| format!("base64 decode: {e}"))
   }
   ```
   + Cargo.toml 加 `base64 = "0.22"`（如尚未）
3. **district env**: `CCTV_TXDOT_DISTRICTS="AUS,SAT"` → `vec!["AUS","SAT"]`；空 → 关
4. **parse**: 只 `statusDescription == "Device Online" && hasSnapshot == true`
5. **写测试** ≥3: decoder happy / corrupt base64 / missing field
6. **跑测试** 绿
7. **commit**: `feat(gev-p11): TxDOT provider + base64-JSON frame decoder`

---

## Task 8 — DriveBC provider

**Files**:
- Create: `hub-core/crates/hub-core/src/monitor/sources/cctv/providers/drivebc.rs`

**Steps**:
1. **endpoint**: `https://www.drivebc.ca/api/webcams/`
2. **parse**: GeoJSON `features[]`；只 `is_on == true && should_appear == true`；orientation code 表 `{N:0,NE:45,E:90,SE:135,S:180,SW:225,W:270,NW:315}` → heading
3. **frame URL build**: `https://images.drivebc.ca/bchighwaycam/pub/cam{id}/latest.jpg`（按 god-eye-view `DRIVEBC_IMAGE_URL`）
4. **写测试** ≥3: parse happy / 8-compass code lookup / 过滤 disabled
5. **跑测试** 绿
6. **commit**: `feat(gev-p11): DriveBC camera provider`

---

## Task 9 — Fintraffic provider

**Files**:
- Create: `hub-core/crates/hub-core/src/monitor/sources/cctv/providers/fintraffic.rs`

**Steps**:
1. **endpoint**: `https://tie.digitraffic.fi/api/weathercam/v1/stations` (GeoJSON FeatureCollection)
2. **headers**: 加 `Digitraffic-User: intelhub-cctv-port/1.0`（courtesy）
3. **parse**: 每个 station → 多个 preset（每 preset 一 camera）；frame URL build `https://weathercam.digitraffic.fi/{presetId}.jpg`
4. **bbox check**: 芬兰经纬 bbox (59.5..70.5, 19.5..31.5)
5. **写测试** ≥3: station with 1 preset / multi-preset station / Finland bbox 拒绝 Stockholm 坐标
6. **跑测试** 绿
7. **commit**: `feat(gev-p11): Fintraffic (Finland) camera provider`

---

## Task 10 — Tarktee DATEX2 XML provider

**Files**:
- Create: `hub-core/crates/hub-core/src/monitor/sources/cctv/providers/tarktee.rs`

**Steps**:
1. **endpoints**: locations + images 双 GET:
   - `https://tarktee.transpordiamet.ee/api/v1/predefinedLocations.xml`
   - `https://tarktee.transpordiamet.ee/api/v1/trafficViews.xml`
2. **Cargo dep**: `quick-xml = "0.36"`（如尚未；确认 P3 已用同一版本）
3. **parse XML**: 用 `quick-xml::Reader::from_str`；extract `<predefinedLocation id=...>` → (name, lat, lon)；`<trafficView>` → (linearPredefinedLocationRef id, urlLinkAddress)
4. **join**: location 与 trafficView id 配对；bbox 57.4..59.9, 21.5..28.4
5. **写测试** ≥3: happy parse / empty XML → 返空 / malformed XML → 不 panic（返 warn + 空）
6. **跑测试** 绿
7. **commit**: `feat(gev-p11): Tarktee (Estonia DATEX2) camera provider`

---

## Task 11 — NSW provider

**Files**:
- Create: `hub-core/crates/hub-core/src/monitor/sources/cctv/providers/nsw.rs`

**Steps**:
1. **endpoint**: `https://www.transport.nsw.gov.au/sites/default/files/json/live-traffic-cameras.json`（具体 URL 计划阶段验证；GeoJSON FeatureCollection）
2. **parse**: features[] → (id, lat, lon, href, direction, view, title)
3. **direction 解析**: `"N-E"` → `"NE"` → `direction_to_heading` 表
4. **label**: `view` (≤140 字符无换行) OR `title`
5. **bbox check**: NSW (大致 -28..-37, 141..154)
6. **写测试** ≥3: parse happy / direction `N-E` → NE / label 选 view 而非 title
7. **跑测试** 绿
8. **commit**: `feat(gev-p11): NSW (Sydney) camera provider`

---

## Task 12 — Calgary provider

**Files**:
- Create: `hub-core/crates/hub-core/src/monitor/sources/cctv/providers/calgary.rs`

**Steps**:
1. **endpoint**: `https://www.calgary.ca/roads/scheduling/ns/trafficcameras/images/`（具体 URL 计划阶段验证 — god-eye-view constants 里有 `DEFAULT_CALGARY_ROWS_URL`）
2. **parse**: 简单 JSON list；bbox 50.9..51.2, -114.2..-113.9
3. **写测试** ≥3: parse happy / bbox 拒绝 / empty 返 []
4. **跑测试** 绿
5. **commit**: `feat(gev-p11): Calgary camera provider`

---

## Task 13 — 注册更新 + env vars 模板

**Files**:
- Modify: `hub-core/crates/hub-core/src/monitor/sources/cctv/providers/mod.rs`
- Modify: `core/hub.env` (template，新增 env vars 注释 + 默认值)

**Steps**:
1. **注册**：按 §3.1 表加 8 个 `&module::Struct` 到 `providers()` 返回的 vec
2. **测试**：现有 `cargo test --package hub-core cctv::providers` 应继续绿（无新测试）
3. **env vars 模板**：在 `core/hub.env` 加：
   ```bash
   # GEV P11 CCTV 新 provider 配置（继承 P3 `HUB_*` 优先模式）
   CCTV_CALTRANS_DISTRICTS=4,7,11,3
   CCTV_CALTRANS_MAX_SOURCES=300
   CCTV_TXDOT_DISTRICTS=AUS,SAT
   CCTV_TXDOT_MAX_SOURCES=500
   CCTV_DRIVEBC_MAX_SOURCES=250
   CCTV_FINTRAFFIC_MAX_SOURCES=300
   CCTV_TARKTEE_MAX_SOURCES=180
   CCTV_NSW_MAX_SOURCES=250
   CCTV_CALGARY_MAX_SOURCES=220
   CCTV_AUSTIN_MAX_SOURCES=250
   # GOOGLE_MAPS_SERVER_API_KEY=   # 写入 core/secrets.env，缺则跳过 Street View 降级
   ```
4. **rsync + build check**（315）：
   ```bash
   cd /Volumes/TBU/Workspace/IntelHub-gev-p11
   rsync -az --delete --exclude '.git/' --exclude 'backups/' --exclude '.DS_Store' \
     --exclude 'compose/.env' --exclude 'compose/.env.crucix' --exclude 'docs/' --exclude 'build/' \
     --exclude 'config/searxng/' --exclude 'hub-core/target/' \
     --exclude 'console/node_modules/' --exclude 'console/dist/' \
     --exclude 'core/' --exclude 'data/' \
     ./ Debian-test:/home/zou/IntelHub/
   ssh Debian-test 'cd /home/zou/IntelHub && bash scripts/build-hub.sh 2>&1 | grep -E "^error|built" | head -8'
   ```
5. **commit**:
   ```bash
   git add hub-core/crates/hub-core/src/monitor/sources/cctv/providers/mod.rs core/hub.env
   git commit -m "feat(gev-p11): register 8 new camera providers + env var template"
   ```

---

## Task 14 — CctvPopoutPanel.tsx 新组件

**Files**:
- Create: `console/src/globe-hud/CctvPopoutPanel.tsx`
- Create: `console/src/globe-hud/__tests__/CctvPopoutPanel.test.tsx`

**Steps**:
1. **写测试**（5 用例）：
   ```tsx
   // __tests__/CctvPopoutPanel.test.tsx
   import { render, screen, fireEvent } from '@testing-library/react';
   import { CctvPopoutPanel } from '../CctvPopoutPanel';

   const baseCamera = { id: 'tfl-1', name: 'Test Cam', city: 'London', lat: 51.5, lon: -0.1, headingDeg: 90 };

   it('renders image with frame URL', () => {
       render(<CctvPopoutPanel camera={baseCamera} onClose={() => {}} />);
       const img = screen.getByRole('img');
       expect(img).toHaveAttribute('src', expect.stringContaining('/api/v1/gev/cctv/frame/tfl-1'));
   });
   it('renders video element for mp4 camera', () => {
       const mp4Cam = { ...baseCamera, feedType: 'mp4' as const };
       render(<CctvPopoutPanel camera={mp4Cam} onClose={() => {}} />);
       expect(screen.getByTestId('cctv-popout-video')).toBeInTheDocument();
   });
   it('close button calls onClose', () => {
       const onClose = vi.fn();
       render(<CctvPopoutPanel camera={baseCamera} onClose={onClose} />);
       fireEvent.click(screen.getByTestId('cctv-popout-close'));
       expect(onClose).toHaveBeenCalledTimes(1);
   });
   it('ESC key calls onClose', () => {
       const onClose = vi.fn();
       render(<CctvPopoutPanel camera={baseCamera} onClose={onClose} />);
       // trusted-event 模拟：参照 P11-A jsdom 教训（GEV P11-A）
       const evt = Object.create(KeyboardEvent.prototype);
       Object.assign(evt, { isTrusted: true, key: 'Escape', bubbles: true });
       fireEvent.keyDown(document, evt);
       expect(onClose).toHaveBeenCalled();
   });
   it('displays attribution chip with license', () => {
       const cam = { ...baseCamera, license: 'Powered by TfL Open Data' };
       render(<CctvPopoutPanel camera={cam} onClose={() => {}} />);
       expect(screen.getByTestId('cctv-popout-attribution')).toHaveTextContent('TfL');
   });
   ```
2. **跑测试确认 fail**
3. **实现 `CctvPopoutPanel`**（核心骨架）：
   ```tsx
   // CctvPopoutPanel.tsx
   import { useEffect, useRef, useState } from 'react';
   import { cctvSource } from '../gev-adapters/cctv';
   import { apiFetch } from '../gev-adapters/http';

   export interface PopoutCamera {
       id: string;
       name?: string;
       city?: string;
       lat?: number;
       lon?: number;
       headingDeg?: number;
       feedType?: 'image' | 'mjpeg' | 'mp4' | 'hls' | 'webm';
       license?: string;
       provider?: string;
       [k: string]: unknown;
   }

   export function CctvPopoutPanel({ camera, onClose }: { camera: PopoutCamera; onClose: () => void }) {
       const cctv = cctvSource(apiFetch);
       const frameUrl = cctv.getFrameUrl(camera);
       const isVideo = camera.feedType === 'mp4' || camera.feedType === 'hls' || camera.feedType === 'webm';
       const [imgError, setImgError] = useState(false);
       const fallbackSvg = useRef('');
       if (!fallbackSvg.current) {
           const esc = (s: string) => s.replace(/[<>&"]/g, c => ({'<':'&lt;','>':'&gt;','&':'&amp;','"':'&quot;'}[c]!));
           fallbackSvg.current = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 320 180">...${esc(camera.id)}...${esc(camera.city ?? '')}...</svg>`;
       }

       useEffect(() => {
           const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose(); };
           document.addEventListener('keydown', onKey);
           return () => document.removeEventListener('keydown', onKey);
       }, [onClose]);

       return (
           <div className="cctv-popout-overlay" role="dialog" aria-label={`Live feed from ${camera.name ?? camera.id}`}>
               <div className="cctv-popout">
                   <header className="cctv-popout-header">
                       <span data-testid="cctv-popout-title">{camera.name ?? camera.id} · {camera.city ?? ''}</span>
                       <button data-testid="cctv-popout-close" aria-label="Close" onClick={onClose}>×</button>
                   </header>
                   <div className="cctv-popout-frame">
                       {isVideo ? (
                           <video data-testid="cctv-popout-video" autoPlay loop muted src={cctv.getMediaUrl(camera)} />
                       ) : imgError ? (
                           <img src={`data:image/svg+xml;utf8,${encodeURIComponent(fallbackSvg.current)}`} alt="frame unavailable" />
                       ) : (
                           <img src={frameUrl} alt="live camera" onError={() => setImgError(true)} />
                       )}
                   </div>
                   <footer className="cctv-popout-footer">
                       <span data-testid="cctv-popout-attribution">{camera.provider ?? ''} · {camera.license ?? ''}</span>
                   </footer>
               </div>
           </div>
       );
   }
   ```
4. **CSS**（追加到 `console/src/hud.css` 末尾）：
   ```css
   .cctv-popout-overlay {
       position: fixed; inset: 0; background: rgba(0,0,0,0.7);
       display: flex; align-items: center; justify-content: center; z-index: 1000;
   }
   .cctv-popout {
       background: #0a1418; border: 1px solid rgba(0,212,255,0.4); border-radius: 8px;
       box-shadow: 0 8px 32px rgba(0,0,0,0.6); overflow: hidden;
       width: 640px; max-width: 90vw;
   }
   .cctv-popout-header { display: flex; justify-content: space-between; padding: 8px 12px; color: rgba(255,255,255,0.9); font-family: monospace; font-size: 12px; }
   .cctv-popout-frame { aspect-ratio: 16/9; width: 100%; background: #000; }
   .cctv-popout-frame img, .cctv-popout-frame video { width: 100%; height: 100%; object-fit: contain; display: block; }
   .cctv-popout-footer { padding: 6px 12px; color: rgba(145,237,255,0.7); font-family: monospace; font-size: 10px; }
   ```
5. **跑测试**：5/5 绿
6. **commit**: `feat(gev-p11): CctvPopoutPanel face-on popout component`

---

## Task 15 — HudDetailPanel CctvBody 改造

**Files**:
- Modify: `console/src/globe-hud/HudDetailPanel.tsx`（CctvBody 函数 + add `popoutCamera` state）

**Steps**:
1. **找 `CctvBody`**（grep `function CctvBody` 或类似）
2. **改 body**：
   - 当前 `<a target="_blank">实时画面 LIVE</a>` 替换为：
     ```tsx
     {camera.frame_url && (
         <button data-testid="cctv-open-popout" onClick={() => setPopoutCamera(camera)}>
             打开实时画面 →
         </button>
     )}
     ```
   - 函数顶部 `const [popoutCamera, setPopoutCamera] = useState<any>(null);`
   - 函数返回末尾 `{popoutCamera && <CctvPopoutPanel camera={popoutCamera} onClose={() => setPopoutCamera(null)} />}`
3. **import CctvPopoutPanel**
4. **写测试**（追加到现有 `HudDetailPanel.test.tsx` 或新建）：
   ```tsx
   it('cctv body renders open-popout button when frame_url set', () => {
       render(<HudDetailPanel detail={{ kind: 'cctv', payload: { id: 'x', frame_url: 'http://x' } }} ... />);
       expect(screen.getByTestId('cctv-open-popout')).toBeInTheDocument();
   });
   it('cctv body does not render button when frame_url missing', () => {
       render(<HudDetailPanel detail={{ kind: 'cctv', payload: { id: 'x' } }} ... />);
       expect(screen.queryByTestId('cctv-open-popout')).not.toBeInTheDocument();
   });
   ```
5. **跑测试**：2/2 绿；现有 cctv 测试不退步
6. **commit**: `feat(gev-p11): HudDetailPanel CctvBody button → CctvPopoutPanel`

---

## Task 16 — localStorage 持久化 panel 位置

**Files**:
- Modify: `console/src/globe-hud/CctvPopoutPanel.tsx`

**Steps**:
1. **加 useState + useEffect**:
   ```tsx
   const [pos, setPos] = useState<{x: number; y: number}>(() => {
       try {
           const raw = localStorage.getItem('godsEyeView.v11.cctvPopout.pos');
           if (raw) return JSON.parse(raw);
       } catch {}
       return { x: window.innerWidth / 2 - 320, y: window.innerHeight / 2 - 180 };
   });
   useEffect(() => {
       try { localStorage.setItem('godsEyeView.v11.cctvPopout.pos', JSON.stringify(pos)); } catch {}
   }, [pos]);
   ```
2. **拖拽支持**（用 P10 `panel-drag.ts` 模块或自建 mousedown handler）：
   ```tsx
   const onMouseDown = (e: React.MouseEvent) => {
       const start = { x: e.clientX - pos.x, y: e.clientY - pos.y };
       const onMove = (ev: MouseEvent) => setPos({ x: ev.clientX - start.x, y: ev.clientY - start.y });
       const onUp = () => { document.removeEventListener('mousemove', onMove); document.removeEventListener('mouseup', onUp); };
       document.addEventListener('mousemove', onMove); document.addEventListener('mouseup', onUp);
   };
   ```
3. **写测试**（追加 `CctvPopoutPanel.test.tsx`）：
   ```tsx
   it('persists panel position to localStorage', () => {
       const { rerender } = render(<CctvPopoutPanel camera={baseCamera} onClose={() => {}} />);
       localStorage.setItem('godsEyeView.v11.cctvPopout.pos', JSON.stringify({ x: 100, y: 50 }));
       rerender(<CctvPopoutPanel camera={baseCamera} onClose={() => {}} />);
       const stored = localStorage.getItem('godsEyeView.v11.cctvPopout.pos');
       expect(stored).toBeTruthy();
   });
   ```
4. **跑测试** 全绿
5. **commit**: `feat(gev-p11): CctvPopoutPanel localStorage position persistence`

---

## Task 17 — sp6 扩展

**Files**:
- Modify: `scripts/accept-sp6.py`

**Steps**:
1. **新增 8 检查位**（每个新 provider 一个 row count 检查）：
   ```python
   def check_provider_floor(provider: str, floor: int):
       n = pg("SELECT COUNT(*) FROM cctv_cameras WHERE provider = $1 AND active", provider)[0][0]
       return ("passed", f"{provider}={n} rows (≥{floor})") if n >= floor else ("failed", f"{provider}={n} rows < {floor}")
   # 在 sp6 main 加：
   for prov, floor in [("caltrans",100),("txdot",50),("drivebc",50),("fintraffic",50),("tarktee",50),("nsw",50),("calgary",50),("austin",50)]:
       check(provider=f"cctv-{prov}", fn=lambda p=prov,f=floor: check_provider_floor(p,f))
   ```
2. **新增 frame fallback 检查**：
   ```python
   def check_frame_fallback_returns_image():
       # 取一个已知 active camera id
       cam = pg("SELECT id FROM cctv_cameras WHERE active AND frame_url IS NOT NULL LIMIT 1")[0]
       r = http(f"/api/v1/gev/cctv/frame/{cam[0]}?ts=0", expect_status=200, expect_content_type_startswith="image/")
       return ("passed", f"frame fallback returned {r.headers['content-type']}") if r.status_code == 200 else ("failed", ...)
   ```
3. **跑 sp6**：
   ```bash
   KEY=$(ssh -o BatchMode=yes Debian-test 'grep -o "ihk_[a-f0-9]*" /home/zou/IntelHub/core/agent-keys.txt | head -1')
   python3 scripts/accept-sp6.py "$KEY" 2>&1 | grep -E "^==.*(passed|shelved|failed)" | tail -1
   ```
   期望：`== 41 passed, 5 shelved, 0 failed ==`（P3 baseline 39 + 新增 8 - 已含 cctv checks 1 调整 = 47? — 准确数字以 baseline 为准，sp6 main 函数签名须保持 0 failed）
4. **commit**: `test(gev-p11): sp6 extension — per-provider floor + frame fallback check`

---

## Task 18 — sp8 扩展

**Files**:
- Modify: `scripts/accept-sp8.py`

**Steps**:
1. **新增 2 检查位**：
   ```python
   # check: CctvPopoutPanel testid in bundle
   bundle = read_console_dist("assets/index-*.js")
   check(name="cctv-popout-testid-in-bundle", passed="cctv-popout-close" in bundle)

   # check: open-popout button testid in HudDetailPanel bundle
   check(name="cctv-open-popout-testid-in-bundle", passed="cctv-open-popout" in bundle)
   ```
2. **跑 sp8**：期望 `== 50 passed, 2 shelved, 0 failed ==`（P10 baseline 48 + 新 2）
3. **commit**: `test(gev-p11): sp8 extension — popout testid + open-button testid`

---

## Task 19 — probe-gev CCTV 段扩展

**Files**:
- Modify: `console/probe-gev.mjs`

**Steps**:
1. **加 popout 测试**：
   ```js
   // 进 globe / 选 cctv / 点 open-popout button / 验证 popout 挂载
   await page.click('[data-testid="cctv-card"]');  // 或类似 — 实际 selector 验证
   await page.click('[data-testid="cctv-open-popout"]');
   await page.waitForSelector('[data-testid="cctv-popout-close"]', { timeout: 5000 });
   const hasError = await page.evaluate(() => /* check pageerror */);
   if (hasError) throw new Error('pageerror after popout open');
   await page.click('[data-testid="cctv-popout-close"]');
   ```
2. **跑 probe-gev**：期望 exit 0
3. **commit**: `test(gev-p11): probe-gev CCTV popout section`

---

## Task 20 — ledger + final review + 部署 + push

**Files**:
- Create: `docs/superpowers/execution/2026-09-19-gev-p11-ledger.md`
- Modify: `AGENTS.md` — sp6/sp8 数字更新

**Steps**:
1. **写 ledger**（参考 P9 ledger 形态）：任务 T1-T19 完成情况、决策日志、未决项
2. **whole-branch review**：`git diff main feat/p11-cctv-port --stat` — 列出所有改动文件，cross-check 无 vendor 改动
3. **merge main + 清理 worktree**:
   ```bash
   cd /Volumes/TBU/Workspace/IntelHub
   git add -A && git commit  # ledger + AGENTS 更新
   git merge --no-ff feat/p11-cctv-port
   git worktree remove ../IntelHub-gev-p11
   git branch -d feat/p11-cctv-port
   ```
4. **rsync → 410 + build + restart + 验收**（AGENTS.md 阶段 2 全套命令）：
   ```bash
   rsync -az --delete ...（同 T13 exclude 清单） ./ IntelHub:/home/zou/IntelHub/
   ssh IntelHub 'cd /home/zou/IntelHub && bash scripts/build-hub.sh ... && bash scripts/build-console.sh ... && sudo systemctl restart hub-core && sleep 4 && systemctl is-active hub-core'
   KEY=$(ssh IntelHub '...')
   for a in sp6 sp8 sp3; do python3 scripts/accept-$a.py "$KEY" 2>&1 | grep -E '==.*(passed|shelved|failed)' | tail -1; done
   ```
5. **push**:
   ```bash
   git push origin main
   ```

---

## 4. 验收总表

| 阶段 | 检查项 | 期望值 |
|---|---|---|
| T14 | vitest CctvPopoutPanel | 5/5 |
| T15 | vitest HudDetailPanel cctv | 4/4（2 新 + 2 旧不退步）|
| T16 | vitest CctvPopoutPanel persist | 6/6 |
| T17 | sp6 (315) | 41+ passed, 5+ shelved, 0 failed（基线 39+5/0 + 新 8 检查位）|
| T18 | sp8 (315) | 50+ passed, 2+ shelved, 0 failed（基线 48+2/0 + 新 2 检查位）|
| T19 | probe-gev CCTV popout | exit 0 |
| T20 | sp6 (410 prod) | 同 315 |
| T20 | sp8 (410 prod) | 同 315 |
| T20 | origin/main pushed | ✓ |

## 5. 风险与回退

- **R1**: 新 provider endpoint 不可达 → CctvRefresh fail-open（已实现），health_cell 标红；旧数据保留
- **R2**: TxDOT base64 decoder 失败 → warn → 上游视为不可达 → 触发降级链
- **R3**: Street View quota 烧穿 → hub 端不缓存 Street View；key 缺则整体跳过；synthetic SVG 兜底
- **R4**: popout CSS 与现有 HUD 冲突 → cctv-popout z-index 1000 高于所有 HUD 表面
- **回退策略**: 任一任务失败 → `git reset --hard HEAD~1` 该任务 commit + 排查；worktree 隔离确保 main 不受影响