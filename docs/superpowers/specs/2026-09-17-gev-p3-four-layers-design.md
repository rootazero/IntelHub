# GEV P3 设计：vessels / CCTV / traffic / installations 四层适配 + HUD deferred 收尾

> 2026-09-17 · 状态：待用户批准 · 前置：P2 已上线（main @ 697f8cf）
> 用户新增需求（本轮）：①CCTV 城市覆盖远超原项目（vendor 仅 4 个静态目录 + TfL/Ontario 两个活接口）；②所有动态数据域 = 免费源优先 + 付费源接口预留（provider registry）；③CCTV 帧/视频走浏览器 GPU 纹理渲染（Cesium 场景内投影，禁 DOM overlay）。

## 范围

| 层 | 内容 | 上游 | key |
|---|---|---|---|
| vessels | hub WS broker → Redis 快照 → REST → adapter | aisstream.io（WS，唯一接口） | `AISSTREAM_API_KEY` env-gated |
| installations | hub Overpass 周期全量入 PG → bbox REST | Overpass `military=*` | 无 |
| traffic | Overpass 道路代理 + TomTom flow 瓦片代理 | overpass-api.de + TomTom | `TOMTOM_API_KEY` env-gated |
| cctv | 城市 provider registry：目录入 PG + frame/media GPU 代理 | 多城市免费 API + 4 静态目录 | 个别城市可选 key |
| HUD deferred | 详情模板 ×3、live styleManager、鼠标坐标、窄屏 @media、menubar a11y | — | — |

## 跨切设计：Provider Registry（免费优先，付费预留）

hub-core 每个动态域一个 provider trait + 注册表，`{domain}_PROVIDER_PRIORITY` env 控制顺序（默认免费在前）。P3 只实现免费源；付费源 = 注册表槽位 + env 占位 + shelved 验收，不写实现。

- flights（已有，补注册表形态）：adsb.lol 免费 → OpenSky 免费 OAuth → ~~ADS-B Exchange/Firehose 付费槽~~
- vessels：aisstream 免费 → ~~MarineTraffic/VesselFinder 付费槽~~
- traffic：Overpass 免费（道路）+ TomTom 免费档 BYOK（flow）→ ~~HERE/付费 TomTom 槽~~
- cctv：城市级 provider trait `CityCameraProvider { catalog() -> Vec<Camera>, frame_url(id) -> Url, health_probe(id) }`；城市即 provider，配置驱动增删

## 分层架构

### 1. vessels（路线 A：hub WS broker，用户已批准 tokio-tungstenite 例外）

- hub-core 新 collector `ais.rs`：**tokio-tungstenite**（P1「零新 crate」首个批准例外）连 `wss://stream.aisstream.io/v0/stream`，订阅全球 ShipPositionReport + StaticDataReport
- 内存累积 map（mmsi → 最新记录，STALE_SECS=600 修剪，复用 adsb apply_tick 模式）→ Redis `hub:globe:vessels` TTL 300s，envelope 形状对齐 aircraft（count/ts/stale/coverage="aisstream"）
- REST `GET /api/v1/gev/vessels` → 快照
- console adapter `vessels.ts`：REST 轮询 → vendor vessels source 接口（`src/app/layers/aisLiveVessels.js` 配方，plan 阶段钉死方法集）
- env-gated：无 `AISSTREAM_API_KEY` → collector 不注册，端点 503，sp6 shelved

### 2. installations（纯免费，无 key）

- hub collector `installations.rs`：24h 周期，Overpass 全球 `["military"]` 查询（分 4 象限防超时，指数退避，失败保旧数据 + 健康格红）
- 迁移 `0020_military_installations.sql`：PG 表（osm_id unique, name, kind, lat, lon, tags jsonb, fetched_at）
- REST `GET /api/v1/gev/installations?south&west&north&east[&exact=1]` → 引擎契约 `{elements, retrievedAt, status, saturated}`（≤10° bbox 校验照抄引擎规则）
- adapter 仅做路径改写 `/api/military-installations` → `/api/v1/gev/installations`

### 3. traffic

- `POST /api/v1/gev/overpass`：代理 overpass-api.de，SSRF 固定 host，body 前缀白名单 `[out:json]`，Redis 缓存 300s（query hash）
- `GET /api/v1/gev/tomtom/status` → `{hasKey}`（读 secrets.env）
- `GET /api/v1/gev/tomtom/flow/{z}/{x}/{y}`：代理 TomTom flow 瓦片，Redis 缓存 60s；无 key 503
- **额度保护**：免费档 200K tiles/月 → hub 侧缓存 + 层默认 OFF + probe 不 enable
- adapter 路径改写 `/api/overpass`、`/api/tomtom/*` → `/api/v1/gev/*`

### 4. cctv（最重，城市 registry）

- **catalog 入湖**：迁移 `0021_cctv_cameras.sql`（id, city, name, lat, lon, heading/fov/pitch, feed_type, frame_url, media_url, provider, active）
- 初始 provider 集（全免费）：
  - 静态目录 ×4（vendor `config/cctv_sources.{austin,shinjuku,tallinn,warendorf}.json`，首次启动灌库）
  - TfL JamCams（伦敦，keyless `api.tfl.gov.uk`，可选 `TFL_APP_KEY` 提额）
  - Ontario 511（keyless `511on.ca/api/v2/get/cameras`）
  - NYC（511ny.org API，plan 阶段验证端点可达性，不可达换 NYC DOT）
  - Singapore LTA DataMall（免费 key `LTA_API_KEY` env-gated，缺席则该 provider 不注册）
- hub collector 周期刷新各 provider 目录（1h）+ 健康抽样探测（5min 每 provider N 个）
- REST：`/api/v1/gev/cctv/sources`（catalog）、`/health`、`/frame/{id}`（SSRF 白名单=库内 host，10s 缓存）、`/media/{id}`（视频流代理；GPU 渲染路径见下）
- **GPU 渲染（用户指令）**：帧/视频一律经 vendor cctv 引擎管道（frames.js/projection.js/ground.js → Cesium 纹理/视锥投影，mp4 走 video-element 纹理）——console 侧只做端点接线，**禁止 DOM `<img>/<video>` overlay 旁路**；probe 断言帧 URL 命中代理且纹理挂载（canvas 像素采样）
- adapter 路径改写 4 端点 → `/api/v1/gev/cctv/*`

### 5. HUD deferred（P2 台账清单）

- DetailPanel：vessel / cctv / installation 三模板（cctv 模板内嵌引擎帧纹理预览入口，非 overlay）
- live styleManager 底图状态（TopBar 标签诚实化：含 photoreal 加载失败回退）
- 鼠标坐标 readout（ScreenSpaceEventHandler → BottomBar，P1 hud.css 风格）
- 窄屏 @media（rail/panel 断点折叠）+ LayerRail menubar role / Escape / 焦点归还

## 验收扩展（sp6/sp8/probe-gev）

- sp6 新增：vessels 快照（shelved by AISSTREAM_API_KEY）、installations rows>0 + bbox 端点、overpass 代理 200、tomtom status（shelved by TOMTOM_API_KEY）、cctv sources rows>0 + frame 代理抽查 1 台（200 + image content-type）
- probe-gev：enable vessels/cctv/traffic/installations 各层后断言 layer state=on 且无 pageerror；cctv 帧像素经 canvas 采样断言非全黑
- 契约守卫：四层 source 契约锚点入 `contract-guard.test.ts`

## 风险与对策

| 风险 | 对策 |
|---|---|
| aisstream 单连接全球流 ≈ 数百 msg/s，hub 内存压力 | 只存最新位置（mmsi 去重），不写原始流；STALE 600s |
| Overpass 全球军事查询超时/限流 | 4 象限拆分 + 退避 + 旧数据保持 + 健康格 |
| TomTom 免费额度被浏览器烧穿 | hub 60s 缓存 + 层默认 OFF + status 端点暴露 hasKey |
| CCTV frame 代理成 SSRF 跳板 | frame 目标 host 必须 ∈ PG catalog 白名单，逐请求校验 |
| 视频流代理带宽 | media 限时 30s 断流 + 并发上限 4；超限 429 |
| tokio-tungstenite 首例新 crate | 用户已批准；构建镜像 rust:trixie 无需系统依赖 |

## 明确不做（P4+）

- CCTV 云台控制、viewshed 覆盖率模式（引擎能力在，HUD 入口 P4）
- 付费源实现（MarineTraffic/HERE/ADS-B Exchange 仅槽位）
- GTFS/GBFS/电台/ALPR（P5）
- flights 域 provider registry 重构（现 adsb+opensky 已工作，注册表化顺手做但不改行为）
