# SP6: Native Monitor — hub-core 内置信号采集层（取代 Crucix）

日期：2026-09-10 ｜ 状态：已确认（用户 "全部使用推荐方案" + "确认"）｜ 分支：feat/native-monitor（worktree 隔离）

## 0. 背景与动机

Crucix（上游 `/Volumes/TBU/GitHub/Crucix` @3db7068，2026-05 后未维护）经源码测绘确认只是一个
"29 源并发抓取器 + 内存展板"：单进程 Node、`setInterval` 全局 15min 一刀切、无数据库、无认证、
RSS 地理坐标带随机抖动（同新闻每轮坐标漂移）、AIS 模块为空壳、Space 卫星位置为伪坐标。
它与我方情报系统定位重复，且黑盒容器阻碍 AI 深度融合。

决策（用户逐项批准）：
- Q1=A monitor 作为 **hub-core 内置模块**（非独立容器）
- Q2=C **分期**：一期地理事件源（Radar 对等+），二期经济时序
- Q3=A 告警三件套**规则版**融入 alerts.rs，LLM 分析留给 agent 经 MCP 自主做（守 hub zero-LLM）
- Q4=A 对等验收后**全删** Crucix（熵减）
- Q5=A 不加新 API（数据走现有 console geo 端点，源健康走 components 模型）

## 1. 架构

```
hub-core（单二进制，systemd）
└── monitor/
    ├── mod.rs        Source trait + 注册表（10 源）
    ├── scheduler.rs  每源独立 Tokio 任务；独立节奏；错误隔离+指数退避；
    │                 per-host 限速器（GDELT 5s）；每日 geo_events 保留期清理
    ├── geo.rs        Signal → geo_events 幂等 upsert + 总线事件 + 高严重度告警
    ├── firms.rs acled.rs gdelt.rs noaa.rs usgs.rs
    ├── opensky.rs rss.rs radiation.rs kiwisdr.rs
    └── (tests)       各解析器单测
```

**Source trait**：`name() -> &'static str`、`interval() -> Duration`、
`fetch(&Ctx) -> Result<Vec<Signal>>`。Ctx 携带 reqwest client、secrets（FIRMS key、ACLED 凭据）、
限速器句柄。注册表静态数组；`HUB_MONITOR_SOURCES` 可裁剪启用集。

**独立节奏**（对 Crucix 全局 15min 的直接超越）：
NOAA 5m · USGS 5m · FIRMS 15m · GDELT 15m · OpenSky 15m · RSS 30m · radiation 30m · ACLED 1h · KiwiSDR 1h。
启动即首采（错开 0-30s 抖动防 thundering herd）。

**源健康**：每源成功/失败注册进现有 components 健康模型 → System 页自动展示，无新 API。

## 2. 数据模型与流

```rust
pub struct Signal {
    pub kind: &'static str,      // fire|conflict|flight|radiation|maritime|news|health|economic|other
    pub title: String,           // ≤200 chars
    pub lat: f64, pub lon: f64,  // 校验范围；非法丢弃
    pub severity: &'static str,  // info|routine|priority（critical 仅 alerts）
    pub occurred_at: DateTime<Utc>,
    pub external_id: String,     // 源内稳定 id；缺省 = 内容哈希
    pub payload: serde_json::Value,
}
```

落库：`INSERT ... ON CONFLICT (source, external_id) DO NOTHING`，source = `monitor:<name>`。
成功后总线发 `monitor_sweep_ingested {source, new, duration_ms}`（Radar 监听此事件刷新）。
`severity >= priority` 的 Signal 走 `alerts::raise`（dedupe_key = 语义哈希，见 §4）。

**迁移 0004**：
1. 历史源名原地改名（Radar 历史不断档）：
   `crucix:thermal→monitor:firms`、`crucix:acled→monitor:acled`、`crucix:gdelt→monitor:gdelt`、
   `crucix:noaa→monitor:noaa`、`crucix:news→monitor:rss`、`crucix:nuke→monitor:radiation`、
   `crucix:air→monitor:opensky`，其余 `crucix:* → monitor:legacy`。
2. 9 个咽喉要道静态坐标 seed（maritime 参考层，source=`monitor:chokepoint`，external_id=名称）。
3. 保留期：scheduler 每日 `DELETE FROM geo_events WHERE ingested_at < now() - interval '30 days'`
   （chokepoint 种子除外）。

## 3. 一期源清单（10 源 + 静态层）

| 源 | key | 端点/方式 | 节奏 | kind | 要点（测绘坑位） |
|---|---|---|---|---|---|
| FIRMS | FIRMS_MAP_KEY | `firms.modaps.eosdis.nasa.gov/api/area/csv/{KEY}/VIIRS_SNPP_NRT/{bbox}/2`，6 热点 bbox | 15m | fire | CSV 朴素解析够用；highIntensity 取 FRP>10MW top15 |
| ACLED | ACLED_EMAIL+PASSWORD | OAuth2 password grant `acleddata.com/oauth/token`（client_id=acled，23h token 缓存）→ `api/acled/read` 7 天窗 limit=2000 | 1h | conflict | **HTTP 200 但 body status!=200 也是错**；403=可能未接受 ToS，错误消息带修复指引；fatalities≥10→priority |
| GDELT | 免 | `api.gdeltproject.org/api/v2/geo/geo` PointData 24h GeoJSON maxpoints=30 | 15m | news | **per-host 5s 限速器**（全局跨源共享同一 host 预算） |
| NOAA | 免 | `api.weather.gov/alerts/active?severity=Extreme,Severe` GeoJSON | 5m | other（灾害） | Polygon/MultiPolygon 手工质心；Accept: application/geo+json |
| USGS | 免（**新增图层**） | `earthquake.usgs.gov/earthquakes/feed/v1.0/summary/2.5_day.geojson` | 5m | other（地震→kind=quake? 用 "other"，payload 带 mag） | 直接 GeoJSON Point，无需质心 |
| OpenSky | 免 | `opensky-network.org/api/states/all?bbox` ×10 热点区 | 15m | flight | 匿名 ~4000 credits/天，429/403 退避；区域级计数+高空机 |
| RSS ×19 | 免 | BBC/NYT/AJA/NPR/DW/F24/Euronews/RFI/Africanews/SBS/IndianExpress/TheHindu/MercoPress… | 30m | news | 真 XML 解析（quick-xml 已具备？否则 rss crate）；**去随机抖动**；country-centroid 精度标注 `geo_precision:"country"`；标题前 40 字符去重 |
| radiation | 免 | EPA RadNet `enviro.epa.gov/enviro/efservice` + Safecast `api.safecast.org/measurements.json` 6 核设施 | 30m | radiation | 阈值 signals（GROSS BETA>5.0 pCi/m³ 等；avgCPM>100 anomaly） |
| KiwiSDR | 免 | `receiverbook.de/map?type=kiwisdr` 内嵌 JS 数组 | 1h | other | 关注区接收机；结构校验（正则失败→报错不静默） |
| chokepoint | — | 静态 9 坐标（迁移 seed） | — | maritime | AIS 空壳不抄，只留参考层 |

**明确不抄**：AIS/WebSocket 空壳、Space 伪坐标、OFAC（safeFetch 500 字符截断使其本就残废）、
Reddit/Patents/OpenSanctions/USAspending/Comtrade/GSCPI/BLS/Treasury（二期再评估）、
LLM 交易点子、9-provider LLM 抽象、163KB dashboard、Discord alerter、Telegram 双向 bot。

## 4. 告警三件套（alerts.rs 增强）

1. **语义哈希 dedupe**：`alerts::semantic_key(title)` — 数字归一化（`\d+`→`#`）+ lowercase + sha256，
   monitor 告警以此作 dedupe_key（同一事件伤亡数 12→15 不重复开单，bump 既有 alert）。
2. **衰减冷却**（dispatcher 投递抑制，不影响开单/bump 语义）：按既有 alert 的 bump 次数，
   第 1/2/3/4+ 次投递前要求距上次投递 ≥ 0/6/12/24h；`alert_deliveries` 已有时间戳可判。
3. **分级映射**：FLASH→critical · PRIORITY→warning · ROUTINE→info（monitor 源产出的
   severity priority Signal 进入 raise()；critical 预留给辐射异常/极重冲突）。

规则版先行；LLM 相关性分析由 agent 经 MCP（alerts 查询工具已存在）自主完成。

## 5. 熵减清单（验收对等后同分支执行）

删：`crucix.rs` worker · compose.ui.yml crucix 服务 · `build-crucix.sh` · `manifests/crucix.yml` ·
`.env.crucix` + `.example` · install.sh `build-crucix` 步骤与 crucix key 提示 · accept-sp4/5 中
crucix 检查项 · config.rs `crucix_url` · console.rs crucix meta · Radar.tsx :3117 外链 +
`crucix_sweep_ingested` 监听（→`monitor_sweep_ingested`）· System.tsx crucix 卡片（→monitor 源健康）。
迁：`FIRMS_MAP_KEY`/`ACLED_EMAIL`/`ACLED_PASSWORD` → VM `core/secrets.env`（install.sh keys 步骤改问 hub secrets）。
FRED/EIA/Cloudflare key 保留在 VM 文件中待二期，不经仓库。

## 6. 配置

hub.env 新增（默认值合理，均可覆盖）：
```
HUB_MONITOR_ENABLED=true
HUB_MONITOR_SOURCES=all          # 或逗号分隔子集
HUB_MONITOR_GEO_RETENTION_DAYS=30
```
secrets.env 新增：`FIRMS_MAP_KEY`、`ACLED_EMAIL`、`ACLED_PASSWORD`。
遥测：`hub_monitor_events_total{source}`、`hub_monitor_last_success_timestamp{source}`、
`hub_monitor_errors_total{source}`；health-check.sh 增 monitor 段。

## 7. 二期预览（本 spec 仅记录）

`signal_observations(series, observed_at, value, payload)` 时序表；FRED/EIA/Treasury/YFinance 采集器；
`signal_query` MCP 工具（agent 宏观分析）；Cloudflare Radar 国家级断网/攻击信号；Delta 检测
（跨 sweep 阈值变化 → 告警）。在一期验收稳定后独立 SP 实施。

## 8. 验证

- **单测**：FIRMS CSV / ACLED JSON（含 200-but-error）/ NOAA 质心 / RSS geotag 去抖 / USGS GeoJSON。
- **accept-sp6.py**（新）：①monitor 源健康全绿（keyless 源）②geo_events 出现 `monitor:` 前缀新行
  ③Radar console 端点返回事件 ④USGS 新图层有点 ⑤crucix 容器/镜像已删 ⑥secrets.env 含迁移 key
  ⑦hub 重启后 monitor 恢复采集 ⑧告警 dedupe 语义保持（SP2B 回归）。
- **回归**：SP2A 16 · SP2B 33 · SP3 19 · SP5 9 全绿；SP4 改写为 monitor 语境（crucix 检查移除后）。
- **worktree**：`git worktree add ../OSINT_Intelligence_Hub-monitor -b feat/native-monitor`；
  验收后合回 main 并删 worktree；回滚 = main 保持 Crucix 现状。

## 9. 风险与缓解

| 风险 | 缓解 |
|---|---|
| 源上游改版静默失效 | 解析结构校验 + 源健康翻 DOWN + 告警（hub 自监控既有路径） |
| OpenSky/GDELT 限速 | per-host 限速器 + 退避；429 标记源 DEGRADED 不连坐 |
| ACLED ToS 变更 | 403 错误消息内嵌修复指引（照抄 Crucix 的提示文本） |
| RSS 解析脆弱 | 用真 XML 解析器；item 缺字段跳过；feed 失败独立计健康 |
| 4 核 VM 资源 | 全源仅 HTTP 轮询，峰值 <50MB RSS；无新容器反而省 1 个 Crucix（1g 限额） |
