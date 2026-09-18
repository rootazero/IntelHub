# GEV P7：相机姿态 + 地点搜索飞行 — 期级设计

> 日期：2026-09-18 · 状态：待用户审批
> 上游总纲：`docs/superpowers/specs/2026-09-18-gev-visual-port-design.md` §3 P7 行
> 调研证据：P7 渲染核探索报告（2026-09-18，agent 全量盘点 cameraOrientationControls / location* / trackedCamera / context-bridge / hub 代理面）

## 0. 目标与范围

两件事：**(1)** 选中航班/卫星/军机后一键相机跟随 + 斜视 35°/俯视 -89° 两档姿态切换 + 回北；**(2)** HudTopBar 占位搜索框实装为地点搜索飞行（geocode → flyTo）。

**硬约束**（继承总纲）：全景渲染，严禁 scopeMask/圆形遮罩/周边压黑（已验证相机核与 scopeMask 零耦合）；vendor 纯净只读；渲染核直引 + React 壳。

**明确不做**：vessels/cctv/quake/installation 跟随（vessels 层无 `trackById`，其余无追踪概念）；驾驶舱（P9）；情报全文搜索（占位搜索框的"情报"语义留给后期，本期只做地点）；`LocationNavigation` 类（shell 编排器，不可直引）。

## 1. 相机姿态

### 渲染核（直引）

`console/gev-engine/src/ui/cameraOrientationControls.js`（import 闭包仅 cesium + 纯函数 scenePick）：

- `OBLIQUE_PITCH = -35°` / `STRAIGHT_DOWN_PITCH = -89°`（注意非 -90；-88.5° 以下 heading 读数退化用相机自身）
- `toggleCameraTilt(viewer) → { tilted, pitch } | false` —— 两档切换
- `resetCameraNorth(viewer) → boolean` —— 回北
- `createCameraOrientationAnimator(viewer, { now?, duration=650 })` —— preUpdate 逐帧 CUBIC_IN_OUT 缓动，trackedEntity 变化/指针交互自动 cancel
- `readCameraTargetFrame(viewer)` / `setCameraTargetFrame(viewer, frame)` —— 姿态读写（含 trackedEntity 时走 `lookAtTransform` 保留 EntityView 参考系）

不直引 `bindCameraOrientationControls`（DOM 接线层）；其 `runNavigation` 互斥闸门在 React 壳用直通 `(_noun, fn) => fn()`（P7 相机指令源只有三个：搜索飞行/姿态切换/追踪进出，均用户手势触发，无并发互斥需求）。

### 追踪进出（经图层 API，不直碰 viewer.trackedEntity）

- flights：`dataManager.layers.get('flights').module.trackById(icao24)` / `stopTracking()` / `getTrackedInfo()`
- satellites：`trackById(Number(noradId))` / 同构
- military：`trackById` / 同构（layer id 以 domains.ts 注册名为准，执行时验证）
- 图层内部自管 `viewer.trackedEntity` + `applyTrackedCameraFrame`（含缩放惯性锁 ≥150m）

### React 壳

- **适配层**新增 `console/src/gev-visual/camera-orientation.ts`：`mountCameraOrientation(viewer) → { toggleTilt(), resetNorth(), isTilted(), destroy() }` 窄契约（包 animator + 纯函数）。
- **详情面板动作区**（HudDetailPanel 各 kind Body 的 `.hud-detail-actions` 内，"进图谱"旁）：
  - flight/satellite/military 选中时显示「跟随」按钮 → `trackById`；已跟随时变「解除跟随」+「斜视/俯视」切换按钮（读 `toggleCameraTilt` 返回值驱动按钮态）
  - HudDetailPanel 从**零 props 改为接收** `{ viewer, dataManager }`（GlobeV2 已有 `sceneHandles` + `railManager` state 现成下穿）
- **回北按钮**放 HudTopBar（仅非追踪态有意义时可用；实现上始终可点，`resetCameraNorth` 内部处理）
- **性能纪律**（引擎教训）：`pickViewTarget` 是 4-12ms 深度缓冲读——倾斜态指示用 `frameIsTilted(readCameraTargetFrame())` 只在用户手势后刷新，指南针/朝向读数用免费的 `camera.heading`，严禁每帧轮询。

## 2. 地点搜索飞行

### hub 后端：新建 `/api/v1/gev/geocode` 代理

- 新文件 `hub-core/crates/hub-core/src/gev_geocode.rs`，**照 `gev_traffic.rs::gev_tomtom_flow` 模式**（api.rs:63-84 注册，auth middleware 继承）：
  - `GET /api/v1/gev/geocode?q=<query>` → 上游 **photon.komoot.io**（免 key，引擎默认链本就用它）→ 归一化响应 `{ lat, lng, name, label, types, viewport: {southwest:{lat,lng}, northeast:{lat,lng}} | null }`
  - Redis 缓存（key `hub:gev:geocode:<sha256(query)>`，TTL 3600s）；**缓存读必须 `Option<Option<Vec<u8>>>` 双层**（GEV P3 redis-rs Nil→vec![] 血泪）
  - 结构化降级错误信封（`gev_err` 模式）+ `x-geocode-cache: hit|miss` 头
  - 请求预算沿用 `REDIS_BUDGET_MS=2000` + 复用 `traffic_http()` 式共享 reqwest client
- photon 失败时**不重试不放肆**——错误信封上抛，前端 toast 可见（失败可见原则）

### 渲染核（直引）

- `console/gev-engine/src/ui/locationSearch.js` 的 `LocationSearch` class：注入式 `search` 回调 + 状态机（idle/searching/found/missing/failed/cancelled）+ subscribe。DOM 耦合仅 `input.classList` + `input.blur()`，React 传真实 input ref。
- `console/gev-engine/src/locations.js` 的 `searchAndFlyTo(viewer, query, options)`：**必须显式传 `options.features = disabledFeatures()`**（request-services.ts:30-47 现成）——默认参数引用 shell 单例 `applicationServices.features` 会 POST 不存在的 `/api/overpass`（404 噪音）。`options.recoverNearView` 同样显式处理（传 null/禁用，annotations 传递依赖只建 slot 无网络副作用，可接受）。
- 自建 `placeSearch` 适配器：`geocode(query, {signal})` → 调 hub `/api/v1/gev/geocode` → 返回引擎规约 `{ place: {...} | null, answered: boolean }`（viewport 用 `{southwest, northeast}` shape，`flyToViewportBounds` 直接吃）。
- 飞行模式引擎自管：country/admin→region-overview、locality→city-overview、精确地物→`flyToLandmark`（BoundingSphere+HeadingPitchRange）。

### React 壳

- HudTopBar 占位搜索框（HudTopBar.tsx:98-108）**启用**：去 `disabled`、placeholder 改「地点搜索 / Location search」、`data-testid` 改名 `hud-search-location`（先 grep `hud-search-p5` 测试引用同步改）、hud.css:471 的 `:disabled` 规则删除（熵减）
- Enter 触发 `LocationSearch.run(query)`；搜索中 input 加 `.searching` 态；结果状态 toast 复用引擎状态机 subscribe（found/missing/failed 文案 zh/en）
- **占位框"情报"语义移除**：placeholder/aria/title 文案同步更新，P5 语义代码痕迹清除（熵减）

## 3. 测试与验收

- 适配层 vitest：camera-orientation（fake viewer 断言 trackedEntity/lookAt 调用形状，lenient-mock 防线）；geocode placeSearch adapter（fake apiFetch 信封/缓存头/降级路径）
- hub 侧：`gev_geocode` 单测照 gev_traffic 现有测试模式（mock 上游 + Redis 双层 Option 回归）
- 契约守卫：source-contracts.test.ts 追加 cameraOrientationControls/locationSearch/locations 的 import-surface 钉扎
- probe-gev.mjs 追加：搜索框输入 "Paris" → 等待相机飞行（`camera.position` 变化断言）→ 选中航班跟随按钮 → `viewer.trackedEntity` 非空断言 → 姿态切换按钮态断言
- sp8 追加检查位：geocode 端点 envelope（200 + label 字段）+ 缓存命中头第二请求 `x-geocode-cache: hit`
- 验收序列：315 全绿 → merge → 410 部署（**本期含 hub-core 改动，build-hub.sh 必跑**）→ 生产验收 → push

## 4. 风险登记

1. **`locations.js` 静态 import shell 单例**（applicationServices）——直引 searchAndFlyTo 必须显式传 features/recoverNearView；契约守卫钉扎该 import 行以便上游同步时察觉
2. **`dataManager.layers.get(id).module` 是非文档化公开属性**（getAll() 只给元数据）——契约守卫加 `layers` Map 存在性 + trackById 签名钉扎
3. **photon.komoot.io 可用性**——数据中心 IP 可能被限流；缓存 TTL 1h 缓解；若 315 验收发现不通，fallback 切 nominatim.openstreetmap.org（需 User-Agent 头，1 req/s 自律）——执行时验证，两选一在 plan 定死
4. **-89° 非 -90°**：文案统一「俯视」不写「垂直 90°」
