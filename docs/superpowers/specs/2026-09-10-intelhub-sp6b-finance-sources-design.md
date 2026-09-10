# IntelHub SP6B：金融/经济数据源扩展（monitor phase B）设计

日期：2026-09-10。状态：已获用户批准（Q1=B+C、Q2=B、Q3=B、Q4=B、Q5=B、Q6=B 全按推荐）。
前置：SP6 native monitor 已交付（main @4b8783c）；知识来源 = `/Volumes/TBU/Workspace/Finance/.claude/skills/atlas` 采集层 survey（40 工具调用的完整报告）。

## 决策记录（用户确认）

- 范畴：**宏观时序 + 市场报价时序 + 情报事件流**；深度基本面**不定时采**，按需 MCP 工具
- 落库：时序 → `signal_observations`；事件类（新闻/insider/财报日历/情绪）→ **evidence 表**（可搜索/可嵌入/入图谱）
- FD（financialdatasets.ai，按请求付费）：**MCP 按需工具 + PG 30 天 TTL 缓存**
- Watchlist：**PG 表 + console 可编辑**，预置默认宇宙
- 告警：最小规则集（VIX 穿越 30、10Y-2Y 符号翻转、标的日变动 ±7%）
- A 股：**本期不纳入**（东财无 SLA，后续再评）；Yahoo **整体放弃**（TLS 指纹伪装问题，FMP/Finnhub 双键链足够）

## 1. 架构

复用 SP6 monitor 框架，新增第二类采集器：

```
monitor/
├── mod.rs            现有 Source trait（geo）+ 新 SeriesCollector trait + 双注册表
├── scheduler.rs      同时驱动 geo sources 与 series collectors（同节奏/退避/限速机制）
├── signals.rs  [新]  Observation → signal_observations 幂等 upsert；最新值查询；告警规则
└── sources/
    ├── fred.rs       [新] 8 宏观序列（1h）
    ├── eia.rs        [新] WTI/Brent 现货（6h）
    ├── treasury.rs   [新] 美债总额 + 平均利率（12h，无 key）
    ├── markets.rs    [新] watchlist 日频 bar，FMP→Finnhub 回退链（6h 幂等回填 40d）
    └── finintel.rs   [新] Finnhub 新闻/insider/财报日历 + StockTwits 情绪 → evidence（1h）
```

## 2. 数据模型（migration 0005_finance.sql）

```sql
CREATE TABLE signal_observations (
  series      text        NOT NULL,   -- 'fred:VIXCLS' / 'quote:AAPL' / 'eia:RWTC' …
  source      text        NOT NULL,   -- 'monitor:fred' …
  observed_at timestamptz NOT NULL,   -- 数据点时间（序列自身日期，非采集时间）
  value       double precision NOT NULL,
  payload     jsonb       NOT NULL DEFAULT '{}',  -- OHLCV 其余字段/YoY 窗口等
  PRIMARY KEY (series, observed_at)               -- 幂等：重采 = upsert
);
CREATE INDEX signal_obs_series_time ON signal_observations (series, observed_at DESC);

CREATE TABLE monitor_watchlist (
  symbol      text PRIMARY KEY,       -- 'AAPL' / 'BTCUSD' / 'SPY'
  asset_class text NOT NULL,          -- us_stock/etf/index/crypto
  label       text NOT NULL DEFAULT '',
  enabled     boolean NOT NULL DEFAULT true,
  created_at  timestamptz NOT NULL DEFAULT now()
);
-- 预置默认宇宙（参考 atlas watchlist，仅美股/ETF/指数/加密）：
-- SPY QQQ DIA IWM(ETF) AAPL MSFT NVDA GOOGL AMZN META TSLA AMD AVGO JPM XOM(us)
-- GLD SLV USO UUP TLT(ETF) BTCUSD ETHUSD(crypto)

CREATE TABLE fd_cache (                 -- financialdatasets TTL 缓存（PG 版复刻 atlas 文件缓存）
  ticker     text PRIMARY KEY,
  fetched_at timestamptz NOT NULL,
  endpoints  jsonb NOT NULL,            -- {endpoint_name: payload} 全 8 端点齐备才存在
  payload    jsonb NOT NULL             -- 蒸馏摘要（返回给 agent 的形态）
);
```

## 3. 采集器要点（atlas 坑全规避）

**fred.rs**：`GET https://api.stlouisfed.org/fred/series/observations?series_id=X&api_key=K&file_type=json&observation_start=<今天-400d>`。
序列：VIXCLS、DGS10、DGS2、T10Y2Y、FEDFUNDS、UNRATE、BAMLH0A0HYM2（HY 利差）、CPIAUCSL。
**CPI 存 YoY%**（窗口 len-13 手算），series 名 `fred:CPIAUCSL_YOY` 带单位语义（atlas F1 教训：字段名必须自解释）。`.` 缺测值跳过。

**eia.rs**（key 已在 VM secrets）：v2 `GET https://api.eia.gov/v2/petroleum/pri/spt/data/?api_key=K&frequency=daily&data[0]=value&facets[series][]=RWTC&sort[0][column]=period&sort[0][direction]=desc&length=10`；RBRTE 同理。

**treasury.rs**（无 key）：fiscaldata `v2/accounting/od/debt_to_penny?sort=-record_date&page[size]=5` + `avg_interest_rates` 同模式。

**markets.rs**：对 watchlist enabled 标的：
1. FMP `stable/historical-price-eod/full?symbol=S&from=<今-40d>&to=<今>` → 逐 bar upsert `quote:S`（value=close，payload 带 OHLCV+change_pct）
2. FMP 失败/premium 字符串（**校验 list shape，不信 200**）→ Finnhub `/quote?symbol=S&token=K` 兜底（`c>0` 过滤幽灵符号；无历史只落当点，payload 标 `history_depth:1` 如实记录）
3. 符号过滤：A 股纯数字/`.HK .SS .SZ .T` 等后缀跳过（该 tier 不提供）；`BTC-USD→BTCUSD`
每源只补缺口（回退链语义）。每调用周期 6h：30 标的 × 4 调用/日 ≈ 120 次 FMP，在免费 250/日内。

**finintel.rs** → evidence（源 `finnhub`/`stocktwits`，dedupe_key 幂等，走现有 evidence 管线含嵌入）：
- Finnhub `/calendar/earnings?from=今&to=今+30d` 全市场一次调用，本地过滤 watchlist
- 每标的 `/company-news`（7d 窗取 5）+ `/stock/insider-transactions`（前 10）
- StockTwits `api/2/streams/symbol/{S}.json`（无 key，200 req/hr 内）：net ratio = (bull−bear)/max(1,bull+bear)，>|0.30| 标 regime；**事件落 evidence，ratio 同时落 `sentiment:S` 时序**
- 全 fail-soft：单源挂不影响其他（atlas 教训：如实记录而非造假填充）

## 4. 告警规则（signals.rs，dedupe_key `monitor:fin:*`，吃 SP6 衰减冷却）

| 规则 | 级别 | 触发 |
|---|---|---|
| VIX 穿越 30（最新 >30 且库中前值 ≤30） | warning | 比较最新观测与库中上一观测 |
| T10Y2Y 符号翻转 | warning | 同上 |
| watchlist 标的日变动 ≥7% | info（≥10% warning） | bar upsert 时检查 |

穿越语义避免每周期重复报警（+ 冷却双保险）。

## 5. MCP 工具 + REST

| 工具 | 层级 | 说明 |
|---|---|---|
| `signal_query` | T1 | `{series_pattern, from?, to?, limit?}` → 观测点列（agent 宏观/市场分析入口） |
| `financials_fetch` | L2 | `{ticker}` → FD 8 端点蒸馏摘要；PG fd_cache TTL 30d + required_endpoints 完备校验；**全成功才落缓存**；缓存命中零计费 |
| `watchlist_manage` | L2 | `{action: add/remove/toggle/list, symbol?, asset_class?, label?}` |

REST（console 用，console agent key）：`GET /api/v1/signals/latest`、`GET/POST/DELETE /api/v1/signals/watchlist`。

## 6. Console：Signals 页

watchlist 管理表（增/删/启停 toggle）+ series 最新值表（按源分组）。i18n en/zh。复用现有页面模式，不引新依赖。

## 7. 密钥与配置

`core/secrets.env`（VM-only）新增：`FRED_API_KEY` `FMP_API_KEY` `FINNHUB_API_KEY` `FINANCIALDATASETS_API_KEY`（EIA_API_KEY 已存在）。
config.rs 对应字段；install.sh keys 步骤补 4 个提示项（可跳过降级）。

## 8. 验收（accept-sp7.py 新脚本 + 全量回归）

1. 0005 迁移生效（3 表存在、watchlist 预置 ≥15 行）
2. fred/eia/treasury 序列有点（signal_observations 各源 ≥1 series）
3. markets 序列覆盖 watchlist ≥80% enabled 标的；payload 带 source 审计标记
4. finintel evidence 入库（source IN finnhub/stocktwits，dedupe 幂等——重跑不增行）
5. `signal_query` MCP 返回观测点；`watchlist_manage` add→list→toggle→remove 闭环
6. `financials_fetch` 首调 MISS→落缓存，二调 HIT（fetched_at 不变）；坏 ticker 优雅 None
7. 告警规则单测（穿越/翻转/±7%）+ 采集器解析单测全绿
8. 回归：sp6 14、sp4 25、sp5 9、sp2a 16、sp2b 33、sp3 19

## 9. 熵减与边界

- 复用 SP6 scheduler/限速/健康上报机制，零并行框架
- 不做：Yahoo、A 股、broker/交易执行、定时 FD 全量采、LLM 分析（hub zero-LLM 不变）
- 无新 crate 依赖（reqwest/serde_json 现有）
- atlas 的 daily-JSON 快照/carry-forward 机制不移植（hub 是持续采集非每日批；幂等 upsert 天然防"旧价新日期"）

## 10. 执行协议

worktree `feat/monitor-finance`；VM 部署先行验收；通过后 merge --no-ff 合回 main、清理 worktree。
