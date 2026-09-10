# IntelHub SP7: Monitor Command Deck（指挥台）— 设计规范

日期：2026-09-10。状态：已获批准（用户：设计 §1–§8 全部确认）。前置：SP6 native monitor + SP6B finance plane 均已交付（main @c45a438）。

## 0. 决策记录（已确认）

| # | 决策 | 选择 |
|---|---|---|
| Q1 | 信息架构 | **B** 新增统一 Monitor 工作台页为默认首页；Radar/Signals/Overview 保留 |
| Q2 | 视觉风格 | **B** dark-ops HUD 仅限 Monitor 页（作用域 CSS，不动全局主题） |
| Q3 | 时序图表 | **A** 手写 SVG sparkline/折线图，零依赖；新增 history REST 端点 |
| Q4 | Sweep Delta | **A** 做——新端点 + 面板 + topbar 方向 pill |
| Q5 | 3D 地球 | **A** 不做，保持 Leaflet 平面 |
| Q6 | 范围 | **A** 只动 monitor 相关；非 monitor 页面零改动 |

## 1. 后端变更（hub-core）

### 1.1 `GET /api/v1/signals/history?series=<name>&days=<n>`
- 从 `signal_observations` 读 `{series, points: [{t, v}]}`，按 observed_at 升序。
- 默认 days=40，范围 1..400；点数硬上限 500（超出时按时间均匀降采样，SQL `WHERE` + `LIMIT` 兜底即可——40 天×每日数点远低于上限，降采样仅在超长窗口触发，实现为查后抽样）。
- series 必填，缺省 400。series 无数据 → 200 + 空 points（前端显 "no data"）。

### 1.2 `GET /api/v1/monitor/delta`
- 返回 `[{source, state, new, fetched, prev_new, prev_fetched, direction, ts}]`，含 series 采集器。
- **存储**：scheduler 写 Redis 健康单元（`hub:monitor:health` HSET）时，把旧值的 new/fetched 平移为 prev_new/prev_fetched 再写入新值（读旧 JSON → 拼新 JSON → HSET，单点写入无竞态）。
- direction 计算：`prev_new` 不存在 → `"new_source"`；`new > prev_new` → `"up"`；`new < prev_new` → `"down"`；否则 `"flat"`。
- 端点读整个哈希逐源计算，无新表、无新存储。

### 1.3 复用（零改动）
`/api/v1/overview`（sources[] 逐源健康）、`/api/v1/signals/latest`、`/api/v1/radar/events`、`/api/v1/alerts`、SSE `/api/v1/events`。

## 2. 前端（console）

### 2.1 路由/导航
- 新页 `Monitor.tsx` → 默认路由 `/`；Overview 移至 `/overview`。
- Nav 顺序：Monitor, Overview, Radar, Signals, Investigations, Search, Evidence, Alerts, Agents, Audit, System。i18n en+zh 全量键。

### 2.2 文件结构
```
console/src/pages/Monitor.tsx            布局编排 + 数据加载 + SSE
console/src/components/hud/Panel.tsx     玻璃面板（标题+状态点）
console/src/components/hud/TopStrip.tsx  徽章/pill/VISUALS 开关
console/src/components/hud/SensorGrid.tsx 源健康层列表（点+名+last_new+state）
console/src/components/hud/Gauge.tsx     大数字+sparkline+阈值变色
console/src/components/hud/Sparkline.tsx 手写 SVG 迷你线
console/src/components/hud/SeriesChart.tsx 手写 SVG 折线（轴/网格/hover）
console/src/components/hud/DeltaPanel.tsx ▲/▼/NEW 差分行
console/src/components/hud/EventStream.tsx SSE 实时卡片流
console/src/components/hud/MonitorMap.tsx Leaflet 简版（复用 Radar 瓦片回退+kind 色）
console/src/hud.css                      dark-ops tokens，作用域 .hud-root
```

### 2.3 布局（三栏 HUD）
- **Topbar**：`SOURCES ok/total` 徽章、delta 方向 pill（up=琥珀/down=绿/mixed/new_source=蓝/无数据=灰）、最近 sweep TimeAgo、未确认告警数（点击跳 Alerts）、VISUALS FULL/LITE 开关（localStorage `intelhub.hud.visuals`，默认 FULL）。
- **左栏**：Sensor Grid（14 源）；Risk Gauges：`fred:VIXCLS`、`fred:DGS10`、`fred:T10Y2Y`、`fred:BAMLH0A0HYM2`、`eia:WTI`——各 40 天 sparkline+最新值；点击 gauge → 中央图表切到该序列。
- **中央**：上=MonitorMap（kind 色点+tooltip+详情侧卡）；下=SeriesChart（当前选中序列）+ 宏观市场卡条（`quote:` watchlist 中 index 组最新值+日变动，正负红绿）。
- **右栏**：DeltaPanel；EventStream（SSE 卡片，kind 色签）；最近 5 条未确认告警（含 ack 快捷按钮）。

### 2.4 数据流
初始 `Promise.all` 5 端点；SSE `monitor_sweep_ingested` → 局部刷新 delta/sources/stream；`alert_raised` → 刷新告警栏；30s 兜底轮询。风格令牌：`--bg:#020408`、玻璃面板 `rgba(13,20,32,.55)+blur`、`ui-monospace` 数字栈、accent `#64f0c8/#44ccff`、warn `#ffb84c`、扫描线/网格 overlay（LITE 关闭 + 移动宽度强制 LITE）。

## 3. 错误处理
delta 不足两次 sweep → 面板显 "accumulating"；瓦片失败 → 现有三级回退；SSE 断 → 现有自动重连；任一面板 fetch 失败 → 面板内 ErrorBox 不拖垮整页；history 空 → "no data"。

## 4. 验收（scripts/accept-sp8.py）
1. history 端点 shape + fred 序列有点；2. days 参数生效；3. delta 端点 shape + direction 枚举合法；4. delta 覆盖全部健康单元源；5. `/` 返回 SPA 200；6. MCP 28 工具不变；7. 回归 accept-sp3/sp6/sp7 全绿；8. console VM 构建绿；9. cargo test 全绿。

## 5. 明确不做（YAGNI）
3D 地球；图表库引入；非 monitor 页面改版；LLM regime chip（Hub 零 LLM 原则）；Monitor 页外的 dark-ops 扩散；delta 的跨 sweep 内容级 diff（仅计数级）。
