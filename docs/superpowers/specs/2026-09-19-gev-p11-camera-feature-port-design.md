# GEV P11：CCTV 功能完整移植 — 期级设计

> 日期：2026-09-19 · 状态：待用户审批 · 前置：P3 (GEV 4 层适配) + P7 (相机姿态) 已上线，main 上 CCTV 现状 = 4 静态目录 + 4 live provider
> 用户原始诉求：①摄像头看不到影像；②展开的影像屏幕框倾斜，无法正面展示；③数据源太少（仅五大湖区 Ontario 511 有覆盖）
> 决策依据：ask_user 三连 — face-on = 浮动 2D popout 面板（gods-eye-view 风格）；Street View = 启用；分期 = 单 comprehensive plan

## 0. 目标与范围

**目标**：将 `/Volumes/TBU/Github/gods-eye-view` 中 CCTV 整套用户体验完整移植到 IntelHub：
1. **Face-on 2D popout 面板**：用户点击摄像头 → 浮窗正面显示画面（解决"看不到影像 / 倾斜 / 无法正面"三连痛点）
2. **数据源广度**：port gods-eye-view 全 10 live provider + 2 static catalog（10 个 live 中 3 个 IntelHub 已有），覆盖五大湖之外的北美/北欧/澳洲/东欧/英国
3. **3 层帧降级链**：upstream → Google Street View → synthetic SVG（永不空帧）

**非目标**：
- 摄像头云台控制（PTZ）/viewshed coverage 模式（P3 文档已划为 P4+，本轮不动）
- 付费源实装（MarineTraffic/HERE 等 — P3 已留槽位）
- 引擎侧改造（vendor 纯净只读 — `console/gev-engine/` 不可触碰）
- 替换 vendor `#cctv-frame` DOM 元素（保留；新 React popout 作为附加 2D 表面，副作用：即便 vendor 隐藏/破坏仍可见）

## 1. 现状基线（来自 P3 落地 + 本轮 exploration）

| 维度 | IntelHub 现状 | gods-eye-view 现状 | 差距 |
|---|---|---|---|
| Live providers | tfl + ontario511 + nyc511 + lta(shelved) = 4 | 10 live (Austin, Caltrans, TxDOT, Fintraffic, DriveBC, Tarkee, NSW, Calgary, TfL, Ontario511) | +7 新增（不含已存在 3 个；视 LTA 重启另算） |
| Static catalogs | austin([]) + shinjuku(3) + tallinn(255) + warendorf(1) = 259 | 同 4 个 | 无 |
| Frame fallback | upstream → 502 直返 | upstream → Street View → synthetic SVG | **缺 2 层降级** |
| 2D face-on 视图 | 仅 HUD link-only | vendor `#cctv-frame` 16:9 panel + 自建 popout | **完全缺失** |
| 地理覆盖 | Great Lakes (Ontario) + London + Tallinn + Tokyo + Warendorf | + 全美 + 加拿大 + 北欧 + 澳洲 + 加州 + 德州 | 7 大区域缺失 |

引擎渲染核 = vendor `console/gev-engine/src/layers/cctv/`（21 文件，byte-pinned `a65d9d85`，P2 锁定）→ 任何 2D face-on 表面必须在 React 壳新建。

## 2. 跨切设计

### 2.1 Provider Registry（继承 P3）

- `CityCameraProvider` trait 已在 P3 实现（`hub-core/crates/hub-core/src/monitor/sources/cctv/providers/mod.rs`）— 每个新 provider = 一个文件 + 一个 impl
- 注册函数 `providers() -> Vec<&'static dyn CityCameraProvider>` 当前返 4 项；本轮追加 5–7 项
- 全部 keyless 优先，可选 key 仅做提额（TfL `TFL_APP_KEY` / NSW `NSW_USER_AGENT` 之类）
- 失败保旧数据（live sweep 已实现，recycle 现有 `CctvRefresh` Source）

### 2.2 帧降级链（新增，详见 §3.4）

`gev_cctv::gev_cctv_frame` 当前实现 = upstream-only。本轮加 2 层降级：
```
GET /api/v1/gev/cctv/frame/{id}
  ├─ 1. upstream (Redis cache 10s, 当前逻辑保留)
  ├─ 2. Street View Static API (env-gated, `GOOGLE_MAPS_SERVER_API_KEY`)
  └─ 3. synthetic SVG (pure, 永可用)
```

### 2.3 2D Face-on Popout（新增，详见 §3.5）

- 新 React 组件 `console/src/globe-hud/CctvPopoutPanel.tsx`
- 触发：`HudDetailPanel` 内 `CctvBody` 的 "打开实时画面 →" 按钮（替换现有 "实时画面 LIVE" 文字链接）
- 数据：`cctv-adapter.getFrameUrl(camera)` 复用现有 frameUrlFor（10s cache grid 同步）

### 2.4 Attribution（继承 P3）

- 每个新 provider 必带 `LICENSE` 常量（OCaml-style 顶端 const），写入 PG `cctv_cameras.license_note`
- HUD `CctvBody` 末尾追加"数据归属"行（license 字段已有，0 成本渲染）

## 3. 分层架构

### 3.1 新增 Provider（live, keyless）

每文件 = 一 provider，按 `tfl.rs` / `ontario511.rs` 现有 pattern 抄：

| Provider | 上游 | 协议 | 行数估计 | 风险 |
|---|---|---|---|---|
| `austin.rs` | `cctv.austinmobility.io` + `data.austintexas.gov` Socrata rows.json | JSON | ~80 | 低，austinmobility bbox 是 |
| `caltrans.rs` | `cwwp2.dot.ca.gov` (12 districts, keyless) | JSON | ~80 | 低，host-pin 直 |
| `txdot.rs` | `its.txdot.gov` per district (keyless) | JSON，**base64-JSON 帧** | ~120 | 中，frame decoder 需新写（snapshot 端点返 base64 JPEG，需 `media.js::fetchTxdotSnapshot` 移植为 hub-side decoder） |
| `drivebc.rs` | `drivebc.ca/api/webcams/` | GeoJSON | ~90 | 低 |
| `fintraffic.rs` | `weathercam.digitraffic.fi` + `Digitraffic-User` 头 | GeoJSON station + presets | ~90 | 低 |
| `tarktee.rs` | `tarktee.transpordiamet.ee` (DATEX2 XML) | XML 双端点 | ~150 | 中，XML 解析用 `quick-xml`（P3 已用） |
| `nsw.rs` | `webcams.transport.nsw.gov.au` GeoJSON | GeoJSON + view-text | ~90 | 低 |
| `calgary.rs` | `trafficcam.calgary.ca` | JSON | ~70 | 低 |

8 个 provider 总增量 ~800 行 Rust + tests，每个 ≥ 3 单测（parse/fail-open/host-pin）。

### 3.2 注册变更

`hub-core/crates/hub-core/src/monitor/sources/cctv/providers/mod.rs::providers()`：
```rust
let mut out: Vec<&'static dyn CityCameraProvider> = vec![
    &tfl::Tfl, &ontario511::Ontario511, &nyc511::Ny511,
    &caltrans::Caltrans, &txdot::TxDot, &drivebc::DriveBc,
    &fintraffic::Fintraffic, &tarktee::Tarktee,
    &nsw::Nsw, &calgary::Calgary, &austin::Austin,  // <-- 新
];
if lta::api_key().is_some() { out.push(&lta::Lta); }
```

### 3.3 Env Vars（每个新 provider 一组 `CCTV_<PROVIDER>_*`）

```bash
# hub-core 自动识别（沿用现有 env_or 模式 + 链式回退）
CCTV_CALTRANS_DISTRICTS=4,7,11,3     # 1..12, 留空 = 关
CCTV_CALTRANS_MAX_SOURCES=300
CCTV_TXDOT_DISTRICTS=AUS,SAT          # 25 个区，留空 = 关
CCTV_TXDOT_MAX_SOURCES=500
CCTV_DRIVEBC_MAX_SOURCES=250
CCTV_FINTRAFFIC_MAX_SOURCES=300
CCTV_TARKTEE_MAX_SOURCES=180
CCTV_NSW_MAX_SOURCES=250
CCTV_CALGARY_MAX_SOURCES=220
CCTV_AUSTIN_MAX_SOURCES=250
GOOGLE_MAPS_SERVER_API_KEY=<user-provides>   # 缺则跳过 Street View 降级层
```

### 3.4 帧降级链实现（gev_cctv.rs 改造）

新增模块 `hub-core/crates/hub-core/src/gev_cctv_frame_fallback.rs`：
- `street_view_fallback(id, lat, lon) -> Option<Vec<u8>>`：调 Google Street View Static API `https://maps.googleapis.com/maps/api/streetview?size=640x360&location=<lat>,<lon>&key=<env>`，timeout 5s，5MB cap；env 缺席 → 永返 None
- `synthetic_svg_fallback(id, label, city, status) -> Vec<u8>`：pure SVG `buildSyntheticCctvSvg`（移植 gods-eye-view `media.js`），320×180 viewBox，camera id / city / "DOWN"/"UNKNOWN" 状态 + 渐变背景，永不 502

`gev_cctv_frame` handler 改造：
```rust
// 现有 upstream 路径（含 Redis cache）保留
match fetch_upstream_cached(id).await {
    Ok(bytes) => return ok(bytes),
    Err(e) => warn!("upstream failed: {e}"),
}
// 降级链
if let Some(bytes) = street_view_fallback(id, lat, lon).await {
    cache_set(id, bytes);
    return ok(bytes);
}
ok(synthetic_svg_fallback(id, label, city, "DOWN"))
```

注：降级层不写 Redis cache（避免污染 upstream 命名空间；3 层降级语义仅"upstream 真值"才被缓存）。

### 3.5 Face-on Popout Panel（新 React 组件）

`console/src/globe-hud/CctvPopoutPanel.tsx`：
- props: `{ camera: CctvCamera, onClose: () => void }`
- 布局：fixed overlay, center-screen, 640×360 (16:9), `aspect-ratio: 16/9; object-fit: contain`，可拖拽（用现有 `panel-drag.ts` 模块，P10 已上），ESC 关闭
- 内容：
  - `<img>` 主体：`src={cctvSource.getFrameUrl(camera)}`（10s refresh via `ts` query grid，沿用现有）
  - mp4 摄像：用 `<video autoplay loop muted src={cctvSource.getMediaUrl(camera)} />`（P3 已实现 media proxy）
  - 加载中 shimmer（CSS 复用 gods-eye-view `@keyframes cctv-frame-shimmer`）
  - 顶部 chip：`{camera.name} · {camera.city}` + `×` 关闭按钮
  - 底部 chip：`{provider} · {license}`（attribution 行，CC BY 类）
  - 错误态：`<img>` onerror 触发 → 自动切到 synthetic SVG（前端层降级，弥补 frame proxy 5xx 的视觉空窗）
- state：localStorage 持久化 panel 位置（沿用 P10 vendor 命名空间 `godsEyeView.v11.cctvPopout.pos`），新加的命名空间后续可统一整理

`HudDetailPanel.tsx::CctvBody` 改造：
- 替换现有 "实时画面 LIVE" `<a>` 链接为：
  ```tsx
  {camera.frame_url && (
    <button data-testid="cctv-open-popout" onClick={() => setPopoutCamera(camera)}>
      打开实时画面 →
    </button>
  )}
  ```
- 顶层 `useState<CctvCamera | null>(popoutCamera)` 持有 popout 状态，渲染 `<CctvPopoutPanel />` 当 popoutCamera 非 null

### 3.6 引擎 vendor `#cctv-frame` 处理

保留不动（vendor 不可触碰）。但 P3 的 `cctv.ts` adapter 注释中曾强调"frame URL 是 module-level 常量，bypassing fetchImpl" — 我们新增的 React popout 直接调 `cctvSource.getFrameUrl(camera)`，与 engine 路径同源同缓存策略，**不会**双重请求。

## 4. 数据模型

- `cctv_cameras` 表结构 **不变**（P3 migration 0021 已覆盖 24 列）
- 新增 env vars（§3.3）：写入 `core/hub.env` 模板 + `core/secrets.env`（Street View key 仅后者）
- 无新 migration

## 5. API 表面

- `GET /api/v1/gev/cctv/sources` — 不变，自动包含新 provider 的 rows
- `GET /api/v1/gev/cctv/health` — 不变
- `GET /api/v1/gev/cctv/frame/{id}` — 行为扩展（upstream → Street View → SVG 降级），对外契约不变（仍返 image/*）
- `GET /api/v1/gev/cctv/media/{id}` — 不变
- 前端新组件 `CctvPopoutPanel` + 状态钩子（不暴露 REST）

## 6. UI/UX 流

```
用户操作          │ 系统反应
──────────────────┼─────────────────────────────────────────
选中 CCTV kind    │ HudDetailPanel 渲染 CctvBody
点击 "打开实时画面"│ setPopoutCamera(camera) → 渲染 CctvPopoutPanel
                  │ frameUrlFor(camera) → GET /api/v1/gev/cctv/frame/{id}?ts=...
                  │ hub: upstream → Street View → SVG 三层降级
                  │ <img> onload → 显示；onerror → 内嵌 SVG 兜底
拖拽面板         │ localStorage 持久化 (P10 panel-drag 模块复用)
ESC / × 关闭      │ popoutCamera = null
                  │ vendor #cctv-frame 仍存在（独立 DOM 元素，不受影响）
```

## 7. 验收扩展

### 7.1 sp6（GEV 平面）

- **cameras per provider** rows > 0：caltrans ≥ 100 / txdot ≥ 50 / drivebc ≥ 50 / fintraffic ≥ 50 / tarktee ≥ 50 / nsw ≥ 50 / calgary ≥ 50 / austin ≥ 50
- **frame 代理三层降级**：upstream 200 / Street View 200（mock key 测试或 shelved）/ SVG 200
- **CctvPopoutPanel 单测**：`__tests__/CctvPopoutPanel.test.tsx` — 5 用例（open/close/drag-position-persist/img-onerror-→-svg-fallback/mp4-video-element）

### 7.2 sp8（前端）

- React Test Renderer 验证 popout 在 `popoutCamera` set 时挂载、unset 时卸载
- localStorage 持久化往返（与 P10 `panel-drag.test.ts` 平行）
- synthetic SVG fallback 内嵌字符串匹配（含 camera id + city）

### 7.3 probe-gev

- enable cctv layer 后点击事件不触发 pageerror
- popout 挂载时 Cesium viewer 仍 60fps（性能纪律：img src 是 10s cache grid，不触发每帧重 fetch）

## 8. 风险与对策

| 风险 | 对策 |
|---|---|
| 新 provider 端点不可达（数据中心 egress 屏蔽某些城市网关） | 已知模式：`sources.js` 每个 loader 自带 try/catch + warn → `[]`；hub 端 `CctvRefresh` 已支持"失败保旧数据"，health_cell 标红 |
| TxDOT base64-JSON 帧解码失败 | 单测覆盖 happy path + corrupt base64 + missing `data` 字段；hub 端 decode 失败 → warn → 视为上游不可达，触发降级链 |
| Tarkee DATEX2 XML 解析漂移 | quick-xml 已用；解析走宽容（missing 字段 → 跳过该 camera），单测覆盖正常/空/坏 XML 三态 |
| Street View key 配额被烧 | hub 端不缓存 Street View 响应（避免放大流量）；key 缺则降级层整体跳过 |
| 14 provider × N cameras 入库导致 PG `cctv_cameras` 暴增 | 沿用 P3 `MAX_SOURCES` per-provider env（默认 250-500）+ total ceiling 不变（4000）|
| React popout 性能抖动 | img 走 10s cache grid + vendor frame path；不新增定时器；不订阅 Cesium 事件 |
| 引擎 vendor `#cctv-frame` 与 React popout 双显示冲突 | vendor panel 是 `#cctv-panel` 元素，由 `cctv-enable-btn` 触发；新 React popout 是独立 HUD 表面，**两者并存不冲突**（P3 设计即如此） |
| localStorage 命名空间与 vendor 重名 | 新 key `godsEyeView.v11.cctvPopout.*`，与 P10 vendor `v10.*` 不重 |

## 9. 明确不做（P12+）

- 摄像头云台控制、viewshed coverage 模式（vendor 引擎能力在，HUD 入口推迟）
- 付费 provider（MarineTraffic/HERE 等 — P3 槽位保留）
- 多摄像头 tabs popout（gods-eye-view 多 tab；本轮单 camera focus 即可，复杂度低）
- 帧 EXIF/时间戳 overlay（vendor 原生 UI，本轮复用现有）
- 摄像头 calibration UX（vendor 已支持，HUD 入口 P12+）

## 10. 决策日志

- 用户决策 1：face-on 2D 表面 = **浮动 popout 面板（gods-eye-view 风格）**——非 HUD 内嵌、非双表面
- 用户决策 2：Street View 降级层 = **启用**（env-gated；用户后续补 `GOOGLE_MAPS_SERVER_API_KEY` 到 `core/secrets.env`）
- 用户决策 3：分期 = **单 comprehensive plan**（不分 P9-A/P9-B/P9-C 多 PR）
- 设计约束：vendor `console/gev-engine/` byte-pinned，**禁止触碰**（P2 约束继承）
- 设计约束：frame URL 经 hub proxy（SSRF 结构性安全 — P3 已建立）

## 11. 参考

- 上游参考：`/Volumes/TBU/Github/gods-eye-view`
  - `server/providers/cctv.js` — main plugin，3 层 fallback
  - `server/providers/cctv/sources.js` — 14 provider loader
  - `server/providers/cctv/media.js` — synthetic SVG + base64 decoder
  - `src/ui/styles/cctv.css` — popout 样式（L219-L304）
  - `src/ui/cctvFrames.js` — frame 队列管理（preloader 模式）
- 既有 IntelHub CCTV：`hub-core/crates/hub-core/src/monitor/sources/cctv/` + `gev_cctv.rs`
- P3 设计：`docs/superpowers/specs/2026-09-17-gev-p3-four-layers-design.md`
- P7 设计：`docs/superpowers/specs/2026-09-18-gev-p7-camera-search-design.md`（注意 P7 是 Cesium viewer 姿态，非 CCTV 数据；本轮不重叠）
- P10 设计：`docs/superpowers/specs/2026-09-18-gev-p10-tail-design.md`（panel-drag 模块复用）