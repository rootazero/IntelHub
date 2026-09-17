# GEV P2 执行台账（2026-09-17，main @ cc1143d，已 push）

> gods-eye-view 引擎融合 P2：vendor 引擎 + IntelHub 适配器 + 自研 React HUD，替换 P1 前端。
> 分支 feat/gev-engine（worktree IntelHub-gev，已删），22 commits（56f226a..cc1143d），18/18 任务完成。

## 交付物

- `console/gev-engine/`：gods-eye-view 1121 文件字节级纯净 vendor（pin `a65d9d85`，UPSTREAM.json + sync-gev-engine.sh + check-gev-engine-sync.sh）
- `console/src/gev-adapters/`：15 层 source 工厂（4 真实：flights/military/earthquakes/satellites + 10 stub + transit stub）
- `console/src/gev-boot/`：自研 bootstrap（createApplication 通用生命周期 + LayerLifecycle data 接线）+ 契约守卫测试（17 项，上游 churn 保险丝）
- `console/src/globe-hud/`：HudFrame 四边框架 + LayerRail（7 域飞出）+ DetailPanel（flight/satellite/quake 模板）+ TopBar/BottomBar
- hub-core：`/api/v1/gev/earthquakes` + `/api/v1/gev/celestrak/{group}`（六组 PG + starlink 代理 Redis 6h 缓存）+ celestrak GROUPS 对齐六核心组 + 迁移 0019 + OpenSky OAuth env-gated 全量矢量（独立 key + REST 合并 fresher-wins）
- `console/probe-gev.mjs`（v2 probe）；P1 前端退役（Globe.tsx/src/globe/*/probe-globe.mjs 删，后端 /api/v1/globe/* 保留）

## Rulings（controller 裁决全集）

1. 引擎 import 禁 @ts-expect-error → d.ts 单条 `declare module "gev-engine/*"`
2. celestrak 六组后 sp6 阈值 T15 实测调整
3. OpenSky 全量矢量独立 Redis key `hub:globe:aircraft:opensky`（TTL 120s）+ REST 读时按 hex 合并（fresher wins，无 age 时 adsb 优先）
4. T13 不改注册表——opensky.rs env-gated 增量
5. T17 只做 315，T18 用户门禁
6. 引擎 import 走 `gev-engine/` alias（vite/tsconfig/vitest 三处）
7. contactTimeMs=observedAtMs 维持（300s coast 对 210s 轮转正确）
8. toEnvelope stale 加 ageMs>120s 派生（对齐引擎语义）
9. T8 承担 LayerLifecycle data 接线（图层未注册则 inert；配方在 vendor src/app/data.js）
10. probe loader「消隐」= 文本非 Error 开头 + 非全屏遮罩（HUD 复用 loading-screen 为状态字）
11. sp6 卫星 per-category floor 按真实星座规模校准（10/100/25/20/20/400；GPS 仅 31 颗）
12. sp6 aircraft count>100→>50（tick 延迟 60-75s 时 13-tick 周期 ~15min > 600s 保有期，谷 79-86 峰 429；freshness 承担活性）

## 验收

- 315：sp6 30+3sh/0f · sp8 39+2sh/0f · sp7 16+11sh/0f · sp3 19/0f · probe OK（hud/canvas/rail-icons=7/aircraft=113/satellites=832/pageerrors=0）+ 截图实证四边 HUD
- 410：sp6 30+3sh/0f · sp8 42+2sh/0f · sp7 16+11sh/0f · sp3 19/0f · probe OK（aircraft=156 satellites=832 pageerrors=0）
- 终评（deepseek-v4-pro）：Ready to merge，七项跨任务连贯性全过，无 Critical/Important

## 事故与教训

- **网络盘 rsync 静默丢文件**（两次：315 丢 254、410 丢 257）：rsync 后必须验 vendor 文件数（1121）
- **并行会话干扰**（2026-09-17 410）：另一会话 27 分钟内 4 次重启 hub-core → adsb 累积窗口反复重置（内存 map，重启后 ~10min 才满，count 86 vs 正常 360-429）+ celestrak 4 轮首轮把共享出口打进惩罚箱（starlink 403 惩罚箱 ~40min 自愈）。诊断链：systemctl show ActiveEnterTimestamp + sudo journalctl 找非我重启。**验收异常先查有没有别的会话在动同一台机器**
- **pve40 断电**（部署中途宿主机掉电 ~1h）：VM 重启后 hub-core enabled 但未自动起（docker 数据层未就绪时它 crash-loop 后放弃？实测需手动 systemctl start）——恢复清单：先确认 docker intelhub-* 全 healthy，再 start hub-core，再等采集器首轮
- **adsb 累积是内存态**：hub-core 重启后 aircraft count 需 ~10min 回填（STALE_SECS=600s）；重启后立刻跑验收会看到假性低 count
- hub 绑 10.10.10.41:8800 而非 localhost——VM 内 curl 要用 LAN IP
- 上游已移到 0d41b6be（pin 之后）——P3 首个 sync 窗口处理

## P3 输入（deferred items 汇总）

- T4 I-1：usgs.rs 把 coordinates[2] 拷入 payload 作 depthKm（深度色通道）
- T9：menubar role/Escape/焦点归还/窄屏 @media；T10：vessel/cctv/installation 模板 + earthquakes 层无 contextStore 注册（上游缺口）
- T11：live styleManager 读底图状态（含 photoreal 加载失败回退场景）；鼠标坐标 ScreenSpaceEventHandler
- T12：c2 补 spy 断言 apiFetch 被调
- T14 minor：probe `in` operator 对 primitive body 的 TypeError
- T16：cesium/satellite.js deps 必须保留（gev-engine 经 console/node_modules 解析）——勿顺手清理
- 引擎 data/tools factory 未接（styleManager/#data-toggles chrome）——LayerLifecycle 已接，chrome 留 P3
