# GEV 引擎融合：gods-eye-view 全量复刻进 IntelHub — 总体设计

> 日期：2026-09-17 · 状态：已批准（用户逐节确认 §1-§6 + HUD 三票）
> 性质：**多阶段工程总纲**。每期（P2-P5）各自走完整 spec→plan→SDD→315→410 循环；本文档是架构与决策的单一事实来源。
> 调研证据：`/Volumes/TBU/Github/gods-eye-view`（上游 `github.com/bilawalsidhu/gods-eye-view`，~156k 行 vanilla JS + Cesium，MIT，极活跃：近 2 周 396 commits，单 owner）

## 0. 目标与已决决策

将 gods-eye-view 的**全部 20+ 可视化图层与交互能力**复刻进 IntelHub，数据**全量入湖**（IntelHub 统一数据层），UI 采用**全屏地球 + 手游式 HUD**（明确不要上游的镜头遮罩范式）。

| 决策 | 结果 |
|---|---|
| 融合路线 | **路线 A**：vendor 引擎（纯净、可同步）+ IntelHub 数据适配器 + 自研 React HUD 壳 |
| 产品定位 | **主入口**——全屏地球即控制台，现有页面成为地球上弹出的模块或二级页 |
| 数据深度 | **全量入湖**——情报资产进 PG/Neo4j/Qdrant；字节流（CCTV 帧/TomTom PBF/电台音频）走 hub 密钥代理 |
| 图层范围 | **全部 20+ 图层，一个不落**（含 GTFS 公交/共享单车/电台/ALPR 长尾） |
| API key | **全部办理**：Google Maps Platform（付费可接受）+ 免费注册（OPENSKY/AISSTREAM/TOMTOM/FIRMS/LL2）+ 已有 Windy |
| HUD 布局（可视化伴侣三票） | ①四边框架式（顶条/左技能栏/右详情/底状态条，左右栏可折叠自动隐藏）②图层栏=域图标+悬停飞出 ③详情=右侧常驻面板（内容随对象换模板） |
| 上游同步 | pin 已知好版本 + 月度同步窗口 + 契约守卫测试保险丝 |

## 1. 总体架构：四层分工

```
React HUD 壳（自研 console/src/globe-hud/）
  全屏 Cesium 画布 + 手游式边缘模块；i18n zh/en；复用 console 设计体系
    │ catalog API / getRowControls() / contextStore（窄契约）
GEV 引擎（vendored console/gev-engine/，上游字节级纯净）
  20+ 图层工厂 + ~15 个服务模块（pickRegistry/trailRenderer/trackedCamera/
  groundSnap/renderGovernor…）+ Cesium 渲染；scopeMask 关闭；shell 模板不引入
    │ SOURCE_METHODS 契约（getSnapshot/getCatalog/getFrameUrl/fetchFlowForBounds…）
IntelHub 适配器（自研 console/src/gev-adapters/）
  每图层实现 source 契约 → 调 hub REST。唯一的引擎接缝，冲突面锁死于此
    │ REST /api/v1/gev/*（鉴权继承 auth_middleware）
hub-core 数据后端（Rust 采集器矩阵 + 代理端点）
  情报资产入 PG/Neo4j/Qdrant；高频移动目标走 Redis 快照通道（P1 模式）；
  字节流走 hub 密钥代理
```

**关键机制**

- **引擎纯净度**：自研 bootstrap（`console/src/gev-boot/`）import 引擎的通用 4 阶段生命周期 `createApplication({createScene, createControls, createData, createTools})`（`src/app/application.js`）+ 图层目录（`src/app/constructCatalog.js`），传入我们的 adapter 工厂。上游 standalone 入口（`src/standalone/`，含 `layerSources.js` 与全部 shell 模板）**整个不引入**——vendor 目录内没有我们的任何字符，同步=整体替换，无需三方合并
- **数据统一**：适配器不直连第三方，全部走 `/api/v1/gev/*`。航班/卫星/地震/山火/AIS 等与现有 66 采集器同构入湖；图层开关/详情/追踪走引擎 catalog 契约
- **scopeMask**（上游默认开启的圆形视野遮罩，`src/scopeMask.js`）：bootstrap 中显式关闭
- **P1 关系**：adsb/celestrak 采集器、Redis 快照通道、`/api/v1/globe/*` 模式、build key 注入链全部复用；P1 自研 `Globe.tsx` 在 P2 退役（后端资产 100% 保留）

## 2. Vendor 与上游同步

- **机制**：纯拷贝 + pin 清单。`console/gev-engine/UPSTREAM.json` 记 `{repo, pinned_sha, synced_at}`；`scripts/sync-gev-engine.sh`：fresh clone → checkout pinned SHA → rsync 进 vendor（exclude `.git`、`server/`、`src/standalone/`、`src/ui/templates/`、打包数据 `src/data/local_data/`）→ 契约守卫 → 构建验证 → 提交。**不用 git subtree**（396 commits/2 周会淹没历史）
- **契约守卫**：`console/src/gev-boot/__tests__/source-contracts.test.ts` 运行时内省每图层 `SOURCE_METHODS`（契约定义见 `src/app/constructCatalog.js:27-46`），断言 adapter 全覆盖 + 引擎服务模块 import 面未变。上游改契约 → 守卫红 → 同步暂停先补适配器
- **节奏**：pin 已知好版本，月度窗口 + 安全修复随时；不追 main
- **许可**：vendor 保留 LICENSE；NOTICE 署名。打包数据（海缆 CC BY-NC-SA、OSM ODbL）不随 vendor 进库，走 §3 入湖通道并附归属

## 3. hub-core 后端矩阵

| 图层 | 后端形态 | 数据落点 | 期 |
|---|---|---|---|
| 航班 | **新建** opensky OAuth 采集器 + 复用 P1 adsb 轮转（降级备份） | Redis 快照 + 显著事件 Signal | P2 |
| 军机 | 复用 P1 adsb 轮转（/v2/mil） | Redis + Signal | P2 |
| 卫星 | 复用 P1 celestrak + TLE 分组扩展 | PG satellites | P2 |
| 地震 | 复用现有 usgs | geo_events | P2 |
| 3D 城市 | 非数据层（Google Photorealistic 3D Tiles 浏览器直连） | — | P2 |
| AIS 船舶 | **新建** aisstream WS 长连采集器（聚合→快照+航迹缓冲） | Redis 快照 TTL + Signal | P3 |
| CCTV | **新建** windy 目录采集器 + 帧代理（缓存）；城市 pack 后期 | PG cctv_sources + 代理帧 | P3 |
| 道路车流 | **新建** TomTom PBF 瓦片代理（hub 持 key+缓存）+ Overpass 路网缓存 | 代理 + PG 路网 | P3 |
| 军事设施 | **新建** Overpass 视口查询（磁盘缓存） | PG infrastructure | P3 |
| 太空任务 | **新建** launch-library 采集器（15min） | PG launches + 临近发射 Signal | P4 |
| 山火 | 复用 firms（key 注册后自动解 shelve） | geo_events | P4 |
| 天气 | **新建** Open-Meteo 代理（免 key） | Redis 快照 | P4 |
| 海缆/数据中心/水坝 | 一次性入湖（上游 GeoJSON → 迁移种子） | PG infrastructure | P4 |
| 新闻 | 复用 gdelt | 现有资产 | P4 |
| 公交 GTFS-RT | **新建**代理采集器 | Redis 快照 | P5 |
| 共享单车 GBFS | **新建**代理 | Redis 快照 | P5 |
| 电台 | **新建** radio-browser 代理；音频浏览器直连 | PG 目录 | P5 |
| ALPR | **新建**代理采集器 | PG + 代理 | P5 |
| geocode/routing/地形 | HUD 工具代理 | 代理 | P5 |

合计：**新建 ~11 采集器/代理 + 复用 6 项 + 1 次静态入湖**。全部遵循 Source trait + Redis 健康格 + 失败可见 + 退避的现有模式。REST 面统一 `/api/v1/gev/<layer>/*`。

## 4. HUD 设计（可视化伴侣三票已定）

- **框架**：四边框架式。顶条（搜索/时间/告警）、左图层技能栏、右详情面板、底部数据源健康+计数条；左右栏可折叠，闲置自动隐藏
- **图层栏**：7 个域图标（空中/太空/地面/海上/网络/基建/环境），悬停/点击飞出该域图层开关列表
- **详情**：右侧常驻面板，内容随对象类型换模板（航班=航迹+关联信号；卫星=轨道参数；CCTV=实时帧）；含「进图谱/回放/告警」动作，与 IntelHub 资产互通
- **风格**：专业暗色默认；引擎 GLSL 风格预设（retro/surveillance/thermal 等，`src/ui/visualPresets.js`）作为 HUD 可选皮肤暴露
- **i18n**：HUD 全自研 zh/en；引擎内部 toast/readout 英文可接受（后期如需汉化，在 bootstrap 包装层做，不进 vendor 目录）

## 5. 分期路线

| 期 | 内容 | 出口标准 |
|---|---|---|
| **P2 引擎基座** | vendor+bootstrap+HUD 骨架+契约守卫+同步脚本+构建打通；波次 1 图层：航班（OpenSky 新采集器）/军机/卫星/地震/3D 城市；Globe.tsx 退役 | 315+410 全绿；5 图层在 HUD 可操作；契约守卫绿 |
| **P3 海上与地面** | AIS/CCTV/车流/军事设施 + 对应后端 | 4 图层上线，验收扩展 |
| **P4 太空·环境·基建** | launches/山火/天气/静态基建入湖/新闻复用 | 图层 14+ |
| **P5 长尾与工具** | GTFS/GBFS/电台/ALPR/geocode/routing/HUD 搜索联动/时间轴回放 | 20+ 图层全量，主入口体验完整 |

P2 是命门：构建集成（模板展开/GLSL/vite 插件）与引擎纯净模式在此验证。每期独立 spec→plan→SDD→315→410→push。

## 6. 风险登记

1. **上游 churn**（396 commits/2周）→ pin + 契约守卫 + 月度窗口
2. **Google 3D Tiles 计费**（浏览器暴露 key，按 tile 计费）→ GCP 预算告警 + ion/OSM 兜底链（P1 已有）
3. **长连稳定性**（aisstream WS / OpenSky 配额）→ 断线重连 + 快照 TTL 降级（P1 模式）
4. **引擎单例约束**（module-level singleton：contextStore/pickRegistry/renderGovernor，一页一实例）→ /globe 路由独占 + StrictMode 双挂载防护（P1 ErrorBoundary 模式）
5. **构建集成复杂度** → P2 第一步 spike 打通；失败降级 = 引擎独立构建 + iframe（数据仍统一走 hub REST，损失一体化体验）
6. **许可** → MIT 署名；CC BY-NC-SA（非商业可用）/ ODbL 归属入 NOTICE
7. **单 owner 上游** → vendor 在手，最坏冻结 pin 自维护

## 7. 验收策略

每期：sp6 增加对应图层检查项（健康格/数据落点/REST 200）+ probe 扩展（HUD 元素存在性 + 图层 entity 计数 + pageerror=0）+ 契约守卫测试。315 全绿 → merge → 410 → 生产验收 → push。基线维护随各期更新 AGENTS.md。（探针文件：P1 为 `probe-globe.mjs`，GEV P2 T16 起为 `console/probe-gev.mjs`——前者已删除。）

## 8. 附：调研证据锚点（gods-eye-view 源码）

- source 契约接缝：`src/app/constructCatalog.js:27-70`、`src/sources/sourceSlot.js`、`src/standalone/layerSources.js`
- 图层工厂同构：`src/layers/<name>/{index,ingestion,records,rendering,controls}.js`（20+ 目录同形）
- 服务税清单：`src/data/pickRegistry.js`、`contextStore.js`、`trackedCamera.js`、`trailRenderer.js`、`src/services/groundSnap.js`、`src/renderGovernor.js` 等 ~15 个
- 通用生命周期：`src/app/application.js`（4 阶段 + AbortSignal 清理）
- scopeMask：`src/scopeMask.js:104`（默认开，可关）
- 3D 城市：`src/maps/google3d.js`（`createGooglePhotorealistic3DTileset` + ion 2275207 兜底）
- 代理服务器形态：Vite dev-server Node 中间件（`server/providers/local.js` 挂 22 个 `/api/*` 代理）——其职责（key 保管/缓存/聚合）在 IntelHub 由 hub-core 采集器+REST 承担
- key 面：`.env`（GOOGLE_MAPS_API_KEY/CESIUM_ION_TOKEN/OPENSKY OAuth/AISSTREAM/TOMTOM/FIRMS/LL2/OPENAI + ~30 CCTV pack 开关）
