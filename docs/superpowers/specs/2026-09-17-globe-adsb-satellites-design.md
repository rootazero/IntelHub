# Globe P1：3D 地球 + ADS-B 逐机航迹 + 卫星轨道 — 设计文档

> 状态：设计已确认（6 节逐节获批），待实施。
> 范围：P1 = Cesium 3D 地球页 + adsb.lol 逐机航迹 + CelesTrak 卫星轨道。
> 后续独立立项：P2 = AIS 实时船舶（aisstream.io 常驻 WebSocket 采集器，IntelHub 第一种长连接采集形态）、P3 = CCTV 公开摄像头（帧代理 + SSRF 防护）。
> 参考项目：`/Volumes/TBU/Github/gods-eye-view`（MIT，Bilawal Sidhu）——**仅作模式参考，代码全部按 IntelHub 体系重写**，不搬运、不 iframe、不引入其 Node middleware 层。

## 0. 背景与决策记录

gods-eye-view（1024 文件 / ~15.6 万行 vanilla JS + Cesium）调研结论：

- 图层渲染层（`src/layers/` 5.7 万行）与 Cesium 深度绑定，128 个文件直接 import cesium，不可拆
- 可拆/可参考：`server/providers/`（21 个上游代理的协议知识）与 `src/data/` 无 DOM 解析器
- 整体搬运 = 养第二个 15.6 万行异构应用（自带 Node 后端 / key 体系 / 构建脚本），已否决
- License 注意：代码 MIT；第三方数据不随 MIT（TeleGeography 海缆 CC BY-NC-SA、OSM 提取 ODbL）——**P1 不使用其任何打包数据**

用户决策记录：

1. 融合目标 = 3D 地球体验 + 数据进 IntelHub 数据层（都要）
2. 底图 = 分层设计，先免费后付费可插拔
3. P1 图层 = ADS-B 逐机航迹 + 卫星轨道（AIS / CCTV 用户也想全要，但同意分期，后续各自立项）
4. ADS-B 位置完全不入 PG（放弃历史航迹回溯，只做实时）
5. 默认卫星分组 ~500 对象（starlink 移出默认组）
6. ADS-B 快照覆盖 = 热点区 + 全球军机（不做全球民航全量）
7. 裸 Cesium API + React.lazy（不引 react-cesium 封装库）
8. P1 交互只做图层开关 + 点击查看（时间轴回放/航迹拖尾/运镜留 P2）

## 1. 架构总览 + Gap 分析映射

### 1.1 Gap 分析映射表

| gods-eye-view 实现模式 | 参考位置 | IntelHub 映射 | 复用 vs 新增 |
|---|---|---|---|
| ADS-B 轮询代理（adsb.lol，免 key） | `server/providers/` + `src/data/adsbLolFallback.js` | 新 `adsb.rs`，克隆 opensky.rs 模式（`Source` trait: `fetch → Vec<Signal>`） | 复用 Source trait / scheduler / 25s HTTP client / limiter / health cell / sweephist 环 |
| 逐机实时位置推送 | `src/layers/aircraft/` | **不进 geo_events**（5000 机 × 15s 会冲垮事件表）→ Redis 快照 `hub:globe:aircraft`（TTL 60s）+ REST 透传 | 新增 Redis 快照通道 + `/api/v1/globe/aircraft` |
| 显著航空事件识别 | `src/data/aircraftClass.js`（仅前端着色） | 采集器内判定 → Signal → geo_events → bus/alerts/investigate | 复用现有事件管线（**超越点**：参考项目只显示，IntelHub 持久化 + 告警 + 图谱化） |
| TLE 目录拉取（CelesTrak，免 key） | `server/providers/` celestrak | 新 `celestrak.rs`，OTX 先例：`ctx.state` 直写 PG，返回空 Vec | 复用 ctx.state 先例；新增迁移 `0018_satellites.sql` |
| 浏览器轨道推演 | satellite.js `propagate()` | console 引入 `satellite.js` npm，前端 1s tick 推演，轨道位置永不写库 | 新增前端依赖 |
| Vite middleware 代理层 | `server/providers/local.js` | **不需要**——hub-core 就是代理层（REST + 鉴权 + VM openclash 出口） | 架构对齐，熵减 |
| 底图三级路由 | `src/maps/google3d.js` | `VITE_GOOGLE_MAPS_KEY` → `VITE_CESIUM_ION_KEY` → 免 key Esri 影像 + 椭球地形；密钥走 console-build.env 注入链 | 复用 build-console.sh verify + .build-manifest 模式 |

### 1.2 数据流总览

```
adsb.lol ─15s─▶ adsb.rs ─┬─ 显著事件 ─▶ Signal → geo_events → bus/alerts/investigate（复用管线）
                         └─ 全量快照 ─▶ Redis hub:globe:aircraft（TTL 60s）
CelesTrak ─6h──▶ celestrak.rs ─▶ PG satellites 表（ctx.state 直写，OTX 先例）

console /globe 页 ◀─ GET /api/v1/globe/aircraft（15s 轮询，读 Redis 快照）
                 ◀─ GET /api/v1/globe/satellites（TLE 目录，localStorage 缓存 1h）
                 └─ satellite.js 浏览器内 1s 推演（默认 ~500 对象）
Cesium 底图：Google 3D Tiles → Ion 地形 → Esri 影像 + 椭球（可插拔 key）
```

### 1.3 关键架构决策

1. **双通道 ADS-B**：事件流（Signal→geo_events）与实时位置流（Redis 快照）分离——保护 geo_events 不被轨迹洪水冲垮
2. **零 Vite middleware**：不引入任何 Node 服务进程，hub-core 整体替代参考项目代理层
3. **Cesium 仅作渲染**：Globe 页不直接 fetch 任何上游，全部数据经 hub REST——鉴权、审计、缓存白拿

## 2. 数据模型

### 2.1 PG —— 迁移 `0018_satellites.sql`（唯一新表）

```sql
CREATE TABLE satellites (
  norad_id   INTEGER PRIMARY KEY,
  name       TEXT NOT NULL,
  category   TEXT NOT NULL,            -- stations | visual | weather | gnss | military | ...
  tle_line1  TEXT NOT NULL,
  tle_line2  TEXT NOT NULL,
  epoch      TIMESTAMPTZ NOT NULL,     -- TLE 元历元
  fetched_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX satellites_category_idx ON satellites(category);
```

- 每 6h 按 category 全量替换（单事务 `DELETE WHERE category=$1` + 批量 INSERT），自然清理再入离轨卫星
- 目录分组 `HUB_CELESTRAK_GROUPS` 可配，默认 `stations,visual,weather,gnss,military` ≈ 500 对象

### 2.2 Redis —— 航空快照

```
hub:globe:aircraft   STRING  JSON blob   TTL 300s（单键覆盖写，读端透传）
```

```json
{
  "ts": "2026-09-17T08:00:10Z",
  "count": 4213,
  "coverage": "hotspots+mil+squawk",
  "cycle_secs": 210,
  "last_tick": "mil",
  "aircraft": [
    {"hex":"a1b2c3","flight":"UAL123","lat":31.2,"lon":121.4,
     "alt_m":11278,"gs":452,"track":92,"squawk":"2000","mil":false,"age_s":45}
  ]
}
```

300s TTL = 采集器死亡探测器：快照过期即前端图层降级。（2026-09-17 实施期修订：adsb.lol 实测可持续配额 ~2-5 req/min，原 11 端点×15s 设计超配 10×，改为轮转采集——见 §3.1。快照因此为累积式：10 分钟窗口内全部目击，逐机 `age_s` 如实标注数据龄期。）

### 2.3 显著事件 → geo_events（复用现有表，kind=`flight`）

| 触发 | severity | external_id（幂等去重，小时桶同 opensky.rs 模式） |
|---|---|---|
| squawk 7700（紧急） | `flash` | `adsb:{hex}:7700:{yyyymmddhh}` |
| squawk 7500/7600 | `priority` | `adsb:{hex}:{squawk}:{yyyymmddhh}` |
| 军用机（dbFlags & 1）落在热点区 | `routine` | `adsb:{hex}:mil:{yyyymmddhh}` |

自动流入 Radar 2D 页 / SSE / alerts，零额外代码。

### 2.4 REST API（挂现有 auth 中间件）

```
GET /api/v1/globe/aircraft
  → 200 {ts, count, coverage, cycle_secs, last_tick, aircraft:[...]}   # Redis 透传
  → 200 {stale: true, aircraft: []}                         # 快照过期（不 5xx，前端据此降级）

GET /api/v1/globe/satellites?category=stations,military
  → 200 {count, items:[{norad_id,name,category,tle1,tle2,epoch}]}
  # 无 category 参数返回全部默认分组
```

只读、无 LLM、无成本记录，与 `/api/v1/radar/events` 同级轻量。

### 2.5 celestrak.rs 的 Signal 返回值

`Ok(vec![])`——TLE 目录不是地理事件，health cell + sweephist 已提供存活监控，不伪造锚点坐标（熵减）。

## 3. 采集器设计

### 3.1 `adsb.rs`

- **上游**：adsb.lol v2 REST（免 key）。**实施期实测：可持续配额 ~2-5 req/min**（容量 4-8、回填 1 token/20-30s），因此采用**轮转采集**而非并发 fan-out：
  - `interval() = 15s`，**每 tick 单请求**，工作队列 14 项轮转：`[mil, squawk/7700, squawk/7500, squawk/7600, point(热点×10)]`（全周期 3.5min ≈ 4 req/min，在配额内）
  - point 查询复用 opensky.rs 的 HOTSPOTS 中心点（250nm 上限）；`/v2/squawk/{code}` 提供**全球**紧急代码覆盖（优于原热点内检测）
  - 单请求失败即 Err → scheduler 退避 30s→10min 正好充当 429 冷却；成功才前进轮转位置
- **累积快照**：Source 实例内 `Mutex<HashMap<hex,(AdsbPoint,seen_abs)>>`（registry 长存对象，跨 tick 存活），每成功 tick 合并新批（同 hex 保留最新目击）、剔除 >600s 未见、全量重写 Redis envelope（TTL 300s）
- **tick 流程**：
  ```
  取队列项 → 单请求 → parse_ac_array → 累积合并 + 剔除 → 重写 envelope
    ├─ squawk 7700 → flash Signal（全球）
    ├─ squawk 7500/7600 → priority Signal（全球）
    ├─ mil 且落热点区 → routine Signal
    └─ 其余 → 无 Signal
  → Ok(signals)
  ```
- **P1 边界**：热点区（3.5min 周期）+ 全球军机 + 全球紧急代码，不做全球民航全量，envelope `coverage`/`cycle_secs`/`age_s` 如实标注

### 3.2 `celestrak.rs`

- **上游**：`https://celestrak.org/NORAD/elements/gp.php?GROUP={g}&FORMAT=tle`（免 key，纯文本三元组 `名称\n行1\n行2`）
- **默认分组**：`stations,visual,weather,gnss,military` ≈ 500 对象；`HUB_CELESTRAK_GROUPS` 可追加（starlink 一组 ~8000 颗，默认关闭）
- **节奏**：`interval() = 6h`
- **写库**：单事务按 category 全量替换（DELETE + 批量 INSERT），返回 `Ok(vec![])`

### 3.3 纯函数拆分（TDD 锚点）

- `parse_tle_catalog(text) -> Vec<TleRecord>`（坏行跳过不 panic）
- `merge_aircraft(batches) -> Snapshot`（去重 / 最新优先 / envelope 组装）
- `classify_notable(ac) -> Option<(severity, external_id)>`

## 4. 前端 Globe 页

### 4.1 集成

- `console/src/pages/Globe.tsx` + 路由 `/globe` + 导航项；**`React.lazy` 路由级代码分割**（Cesium ~1MB+ gz 不拖累其他 14 页）
- 新依赖：`cesium@^1.124`、`vite-plugin-cesium`（拷贝 Workers/Assets 到 dist）、`satellite.js@^6`
- 裸 Cesium API：`useRef` 容器 + `useEffect` 创建/销毁 Viewer，React 只拥有容器 div——不引 react-cesium 封装库

### 4.2 底图回退链（`src/globe/basemap.ts`，对齐 basemap.ts 单一事实源模式）

```
VITE_GOOGLE_MAPS_KEY → Google Photorealistic 3D Tiles（createGooglePhotorealistic3DTileset）
VITE_CESIUM_ION_KEY  → Cesium World Terrain + Ion 影像
（都无）              → EllipsoidTerrain + Esri World Imagery（免 key 兜底，永远可用）
```

密钥走 console-build.env 注入链（VM-only，同 VITE_CARTO_KEY 通道）；`build-console.sh` verify + `.build-manifest.json` 扩展两个 key 状态字段。

### 4.3 图层渲染（Primitive 层，绕开 Entity API）

| 图层 | 数据源 | 渲染原语 | 更新 |
|---|---|---|---|
| 航空 | `/api/v1/globe/aircraft` 15s 轮询 | `PointPrimitiveCollection` | 快照替换，mil/常规着色分桶；桶内 15s 线性外推平滑 |
| 卫星 | `/api/v1/globe/satellites` + localStorage 1h | `PointPrimitiveCollection` | satellite.js 1s 批量 SGP4（500 对象 <5ms/帧） |
| 点击 | `ScreenSpaceEventHandler` 拾取 | 右侧情报面板（呼号/高度/地速/航向；卫星名/类别/轨道周期） | — |

### 4.4 HUD 与降级

- 顶部图层开关（航空 / 仅军机 / 卫星分类多选）+ 实时计数
- `stale:true` → 琥珀色「ADS-B STALE」徽章 + 图层淡出（失败源保持可见，不静默隐藏）
- **Viewer 就绪前禁动相机**（对齐 2026-09-13 Leaflet 教训同类防护）
- `ErrorBoundary`（console 首个）：Globe 崩溃只黑本页，不卸载整个 console 根

## 5. 错误处理与降级矩阵

| 故障点 | 行为 | 用户可见状态 |
|---|---|---|
| adsb.lol 单请求失败 / 429 | tick Err → scheduler 退避（= 429 冷却），轮转位置不动下轮重试 | envelope `last_tick` 停更；退避 >300s →「ADS-B STALE」徽章 + 图层淡出 |
| CelesTrak 挂 | 退避；PG 保留上一期目录 | 面板显示 TLE 龄期，epoch > 48h 标灰 |
| Redis 挂 | SETEX 失败（`redis_timed` 2s 超时）→ `tracing::warn!` 可见；tick 保持 Ok 以保护 channel-2 紧急事件流（有意设计）；退避只由上游失败路径触发 | 快照 TTL 300s 过期 → 端点返 `{stale:true}` →「ADS-B STALE」徽章 + 图层淡出 |
| PG 挂 | celestrak 写库 Err → 退避 | 前端 localStorage 缓存 TLE；无缓存显示「目录不可用」 |
| ion/google key 缺失/失效 | 底图回退链自动降级 | 免 key Esri 影像兜底，永不白球 |
| Cesium 资产加载失败 | ErrorBoundary | 页内错误卡片 + 重试，不炸 console |

零静默失败，零新告警通道——health cell / sweephist / Monitor 页全部自动继承。

## 6. 测试、验收与部署

### 6.1 测试（TDD，纯函数先行）

- Rust 单测：`parse_tle_catalog` / `merge_aircraft` / `classify_notable`；集成测试 `tests/globe_collectors.rs`（仿现有 tests 模式）
- console：`probe-globe.mjs`（仿 probe-monitor.mjs headless 模式）——无 pageerror、canvas 存在、航空计数 > 0、STALE 徽章逻辑正确
  > **已退役（GEV P2 T16）**：该探针删除，后继为 `console/probe-gev.mjs`（P1 自建 globe 页已由 GlobeV2 取代）。
- 构建守卫：Globe chunk 独立 lazy 分包，其他页面 chunk 体积不变

### 6.2 验收

- sp6 基线 18 → **22**：① adsb/celestrak health cell 绿；② `hub:globe:aircraft` count > 100；③ satellites 行数 > 400 且 fetched_at 新鲜；④ 两个 `/api/v1/globe/*` 端点 200
- sp8 / sp7 / sp3 保持全绿不退化

### 6.3 部署（AGENTS.md 铁律全流程）

```
worktree feat/globe-p1 → rsync → Debian-test (315) 构建+重启 → sp6/sp8/sp7/sp3 全绿
→ merge --no-ff main → IntelHub (410) 生产部署 → 生产验收 → git push origin main
```

### 6.4 实施期验证项（写入实施计划）

1. **VM 出口可达性**：adsb.lol / celestrak.org 必须从 VM（openclash 出口）探测，Mac 直测结论无效
2. **docker Node 版本**：build-console.sh 镜像与 vite-plugin-cesium 兼容性

### 6.5 成本与回滚

- 默认配置零付费 API（adsb.lol / CelesTrak / Esri 影像全免 key）；Google/Ion key 纯可选
- 回滚：标准 `git revert` + 重建；迁移 0018 只增不改，无需回滚 SQL
