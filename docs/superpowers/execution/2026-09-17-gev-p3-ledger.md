# GEV P3 执行台账 — 四层融合 + HUD deferred 收尾（2026-09-17）

> Spec：`docs/superpowers/specs/2026-09-17-gev-p3-four-layers-design.md` · Plan：`docs/superpowers/plans/2026-09-17-gev-p3.md`
> 分支：`feat/gev-p3`（31 commits）→ merge `860c537`（--no-ff）→ main。**315 验收全绿 → 410 生产验收全绿 → 已 push**。

## 范围

P2 引擎融合之上补四动态层 + HUD 遗留：vessels（AIS）、cctv（交通摄像头）、traffic（路况）、installations（军事设施），+ HUD deferred 五项（三详情模板/底图标签/鼠标坐标/窄屏断点/menubar a11y）。用户追加：①CCTV 城市覆盖超原项目；②全动态域免费源优先+付费槽位预留（provider registry）；③CCTV 帧走浏览器 GPU 纹理（禁 DOM overlay）。

## 任务执行（T0-T17 全完成）

| 任务 | 内容 | 结果 |
|---|---|---|
| T0 | 研究尖刺 contracts.md：四接口钉死（ais-live 三态信封/overpass+tomtom/CameraSource 19 字段/installations 透传）；上游实测 Ontario 944/TfL 890/511NY 2933 台 keyless、aisstream 握手 101 | ✅ |
| T1 | ais.rs WS collector：tokio-tungstenite rustls（**首个批准新 crate**）、mmsi map+STALE 600s、Redis TTL 300s、track ring buffer；25 测试 | ✅ b578763+ef0aaca |
| T2 | hub /gev/ais-live 三态信封（missing-key/connecting/live 均 200 绝不 503）+ vessels.ts（节→m/s）；7+测试 | ✅ b1b4678+f4b5bf5 |
| T3 | 迁移 0020 + installations.rs（24h、4 象限、60s 礼貌、<24h 重启跳过、全成功才 sweep）；14 测试 | ✅ 67aa9ed |
| T4 | gev_installations.rs（bbox 校验、PG 重组 Overpass 元素、stale>30h、saturated cap+1 探测）17 测试 + installations.ts 复用 vendor 工厂 fetchImpl 改写 | ✅ 123a2fd+6cd99e8 |
| T5+T6 | gev_traffic.rs 三端点（overpass 白名单 POST+300s 缓存、tomtom status、flow 瓦片 120s 缓存）；13 测试；审查修复 reqwest `without_url()` 防 key 泄日志 | ✅ 296c94c+f163eb9 |
| T7 | traffic.ts 包 vendor createTrafficSource 前缀改写；8 测试 | ✅ f99cf35 |
| T8 | 迁移 0021 cctv_cameras（22+ 列）+ 静态灌库 259 台（tallinn 255/shinjuku 3/warendorf 1）；12 测试 | ✅ 81ad213 |
| T9 | CityCameraProvider trait（Pin<Box<dyn Future>> 对象安全零 async_trait）+ tfl/ontario511 + refresh 双 Source（1h 刷新 + 5min 健康轮转）；修 T8 全局 sweep wipe 活 provider 交互 bug + mjpeg→frame_url（vendor isVideoFeedType 只认 mp4/hls/webm）；24 测试 | ✅ 78c0714 |
| T10 | nyc511（keyless Disabled/Blocked 过滤）+ lta（env-gated；LTA DataMall 全路径 404 → LTA_API_BASE 可重定向，shelved）；29 测试 | ✅ bf12e07 |
| T11 | gev_cctv.rs sources/health（19 字段、NULL 省略非 null、derive FromRow）；4 测试 | ✅ f17e577 |
| T12 | frame 代理（10s Redis hash 缓存+15s 超时+8MB cap+CT sanitize）+ mp4 流（Semaphore cap 4→429）+ hls 501 留 P4；SSRF 结构性免疫（客户端只给 id）；7 测试 | ✅ c76f16d |
| T13 | cctv.ts——关键发现：vendor getFrameUrl/getMediaUrl 由模块常量烘焙 URL 不经 fetchImpl，createCctvSource 不可复用 → 四方法自实现；7 测试 | ✅ 48a436d |
| T14 | HUD deferred 五项：三详情模板、live 底图标签（BottomBar 非 TopBar 不碰 P2 布局）、鼠标坐标（rAF 节流）、≤900px 断点、menubar a11y（Escape/方向键挂外层 .hud-rail 修键盘陷阱）；129 测试 | ✅ 89c30ea+dda2e17 |
| T15 | 契约守卫 c1 +3 锚点（20/20 绿）+ provider_priority.rs（免费优先默认、{DOMAIN}_PROVIDER_PRIORITY env、付费槽位 not-licensed）；4 测试 | ✅ e89c56f |
| T16 | sp6 +8 检查（ais 信封门控/installations rows+bbox+400/overpass round-trip/tomtom status/cctv catalog>200+frame 抽查×3）+ probe-gev rail 三路 toggle 演练 | ✅ b73787f |
| T17 | 315 部署验收：**全绿**（sp6 37+5sh/0f · sp8 39+2sh/0f · sp7 16+11sh/0f · sp3 19/0f · probe-gev OK） | ✅ |

## 验收期关键修复（T17 战役）

1. **`useCursorCoordinates` 传 `scene` 而非 `scene.canvas` 给 `new ScreenSpaceEventHandler()`** → 真 Cesium 构造器 `element.addEventListener` 崩 → ErrorBoundary 卸载整个 HUD（probe "Globe failed to render"）。假件过宽容掩盖。**修法：接口要求 canvas + 假件构造器断言参数形状**（lenient-mock 教训，34b4dd1）
2. **redis-rs `Vec<u8>` GET 把 Nil 解码成 `Ok(vec![])`** → 二进制缓存 miss 读成 hit+空 body（tomtom flow + overpass 双点，`Option<Option<Vec<u8>>>` 修复，35b7c51）。String/HashMap 目标类型不受影响
3. **osm.ch 是瑞士区域 extract**：区域外 bbox 返 200+空 elements 无 remark → 假空武装全轮 sweep（曾 wipe 315 的 q2 行）。从镜像链移除 + sweep 地板 5000 行守卫（44e11ba/82b61dc）
4. **Overpass 镜像自适应细分**：象限全镜像失败 → 4 子象限重试（410 生产首轮即靠它落 q0 sub=316 行，a2b6e80/82b61dc）
5. overpass 代理：空 body 不当成功（b62ad2e）+ 永不缓存空 body（3125b09）
6. probe-gev：T14 roving-tabindex 需 focus-click 再 open-click（重试 ≤3 次，af29f10）；sp6 `n_mi` int cast bug

## 生产验收（410，2026-09-17 15:5x UTC）

- sp6 37+5sh/0f（shelved：FIRMS/ACLED/AISSTREAM/TOMTOM key 缺席 + ais-live shelved 行门控）
- sp8 42+2sh/0f · sp7 16+11sh/0f · sp3 19/0f
- probe-gev 410 OK（rail-icons=7、P3 三路 toggle 演练通过；infra checkbox actionability WARN 非阻塞）
- 迁移 0020/0021 应用；cctv 3969 台 6 provider 首轮落库；installations 细分模式落库中（每日轮自愈回补）

## 环境事件（均自愈，非 bug）

- Esri arcgisonline 双出口同时 flap ~15min（scene 阶段 setStack 无超时挂起 → **vendor 缺口记 P4**：provider construction 无超时，需 seam 侧 preflight/timeout）
- Overpass 镜像 403/504 惩罚箱（验收期 hammering 所致，自愈；细分+回退链吸收）
- celestrak starlink 403 间歇（collector geo 组健康，starlink 查询选择性惩罚，自愈）
- 315 被并行会话两次重启 hub-core（干扰事件 #3）；worktree BRICS 文档已保留至 main 未跟踪

## Rulings

- **P3-1**：installations 重试=共 3 次尝试（1+2）；列名 mil_class
- **P3-2**：vessels 路线 A hub WS broker（非浏览器直连）；ais-live 三态信封绝不 503
- **P3-3**：cctv vendor 源不可复用（URL 烘焙）→ 自实现；installations/traffic 复用 vendor 工厂 fetchImpl 路径改写
- **P3-4**：hls 501 留 P4（playlist 段 URL 重写复杂）；511NY feed_type 降级 image 保可用
- **P3-5**：LTA shelved（DataMall 404）→ env-gated + LTA_API_BASE 可重定向

## P4 输入

- hls playlist 代理（段 URL 重写）；LTA 端点定案（有 key 后验 LTA_API_BASE）；cctv 云台/viewshed
- 付费源实现（槽位已在 provider_priority）；vendor upstream sync（HEAD 已移至 0d41b6be）
- **vendor setStack/provider construction 无超时**（Esri flap → 全局 boot 挂起）——seam 侧 preflight 或超时包裹
- infra 域 flyout checkbox Playwright actionability（probe WARN，非功能问题）
