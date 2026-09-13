# IntelHub — Agent 操作手册

> 给后续会话的直接使用手册。所有命令均在实际部署中验证过（最后更新 2026-09-12，main @05521ee）。

## 主机与路径

| 项 | 值 |
|---|---|
| 宿主机 | PVE40（Proxmox），VM 410（4 vCPU，balloon 8–16G） |
| VM 地址 | `10.10.10.41`，ssh 别名 **`IntelHub`**（免密已配），用户 `zou` |
| VM 部署目录 | `/home/zou/IntelHub`（rsync 目标 + 构建现场） |
| Mac 主仓库 | `/Volumes/TBU/Workspace/IntelHub`（网络盘，有同步延迟） |
| GitHub | `https://github.com/rootazero/IntelHub`（**PRIVATE**，push over HTTPS） |
| hub 服务 | systemd 原生 `hub-core`（唯一的非 docker 例外），端口 8800 |
| 数据层 | docker：`intelhub-postgres` / `intelhub-redis`（compose/compose.base.yml） |

## 开发流程（铁律）

1. **worktree 隔离**：`cd /Volumes/TBU/Workspace/IntelHub && git worktree add ../IntelHub-<suffix> -b feat/<name>`（网络盘有同步延迟，紧接着操作前 `sleep 4`；失败的 worktree → `rm -rf ../IntelHub-<suffix> && git branch -D feat/<name> && git worktree prune`）
2. 在 worktree 里改代码
3. **rsync → VM 构建 → 重启 → 验收**（命令见下）
4. 验收全绿后：`git add -A && git commit` → 主仓库 `git merge --no-ff` → `git worktree remove` + `git branch -d` → **`git push origin main`**
5. **绝不**：直接在 main 工作区改、跳过验收合并、提交 secrets

## 部署命令（每次必走的完整序列）

```bash
# 1. rsync（exclude 清单固定，照搬）
cd /Volumes/TBU/Workspace/IntelHub-<suffix> && rsync -az --delete \
  --exclude '.git/' --exclude 'backups/' --exclude '.DS_Store' \
  --exclude 'compose/.env' --exclude 'compose/.env.crucix' --exclude 'docs/' --exclude 'build/' \
  --exclude 'config/searxng/' --exclude 'hub-core/target/' \
  --exclude 'console/node_modules/' --exclude 'console/dist/' \
  --exclude 'core/' --exclude 'data/' \
  ./ IntelHub:/home/zou/IntelHub/

# 2. 构建 + 重启（hub 改动跑 build-hub.sh；console 改动跑 build-console.sh；都改都跑）
ssh -o BatchMode=yes IntelHub 'cd /home/zou/IntelHub \
  && bash scripts/build-hub.sh 2>&1 | grep -E "^error|built" | head -8 \
  && bash scripts/build-console.sh 2>&1 | tail -1 \
  && sudo systemctl restart hub-core && sleep 4 && systemctl is-active hub-core'
```

- `build-hub.sh` 在 rust:trixie docker 里编译（named volume 缓存 `intelhub-hub-target`/`intelhub-cargo-registry`），产物到 `core/hub`
- `build-console.sh` 的 VITE_CARTO_KEY 回退链：显式 env → console-build.env → secrets.env 的 CARTO_BASEMAP_KEY
- hub-core 重启会触发全部采集器立即首轮扫描（约 2.5 分钟后数据可见）
- 编译报错细节：`ssh IntelHub 'docker run --rm -v /home/zou/IntelHub/hub-core:/ws -w /ws -v intelhub-hub-target:/ws/target -v intelhub-cargo-registry:/usr/local/cargo/registry rust:trixie cargo build --release --workspace 2>&1 | grep -B4 -A12 "error\[" | head -40'`

## 验收（每个 SP 一个脚本，从 Mac 跑）

```bash
KEY=$(ssh -o BatchMode=yes IntelHub 'grep "api_key:" /home/zou/IntelHub/core/agent-keys.txt | head -1 | grep -o "ihk_[a-f0-9]*"')
for a in sp8 sp6 sp7 sp3; do
  echo "── $a: $(python3 scripts/accept-$a.py "$KEY" 2>&1 | grep -E '==.*(passed|failed)' | tail -1)"
  python3 scripts/accept-$a.py "$KEY" 2>&1 | grep "^FAIL" | head -3
done
```

当前基线：sp2a 19 · sp2b 33 · sp3 19 · sp4 25 · sp5 9 · **sp6 18 · sp7 24 · sp8 18**。改动某个面时对应脚本必须加检查项并保持全绿。

## 健康检查速查

```bash
# 概览（采集器状态/数量）
curl -s -H "Authorization: Bearer $KEY" "http://10.10.10.41:8800/api/v1/overview" -o /tmp/ov.json

# Redis 健康格原文（含错误文案）
ssh IntelHub 'docker exec intelhub-redis redis-cli --no-auth-warning -a $(grep "^REDIS_PASSWORD=" /home/zou/IntelHub/compose/.env | cut -d= -f2) HGETALL hub:monitor:health'

# Postgres（注意多层引号地狱 → 用 base64 管道）
echo "SELECT ..." | base64 | ssh IntelHub 'base64 -d | docker exec -i intelhub-postgres psql -U intelhub -d intelhub'
```

## Secrets 与网络（不可违反）

- **secrets 只在 VM**：`core/secrets.env`（0600）、`compose/.env`——rsync exclude 已挡，永远不要 `git add` 它们
- console-build.env 含 VITE_CARTO_KEY，同样 VM-only
- **上游 API 探测一律从 VM 发**（采集器真实出口路径，走 openclash 代理），Mac 直测会得出错误结论
- openclash 诊断：`ssh ImmortalWrt`（10.10.10.1）；runtime yaml 在 `/etc/openclash/`；controller 127.0.0.1:9090（secret 在 yaml 里）；切节点 `PUT /proxies/<urlencoded-group> {"name":X}`；日志 `/tmp/openclash.log`

## 已知坑（血泪）

- **edit 工具**：一次调用里多个 edits 指向同一路径时，若 oldText 跨文件混淆会静默失败——改完务必 `grep` 验证；TSX 深度嵌套 JSX 属性会触发 TS1381（在 map 块体里预计算）
- **雷达默认窗口 24h**：旧日期事件不可见，排查先看 `occurred_at`，验收用 `?from=2026-01-01T00:00:00Z`
- **Signal::new 的 kind 要 `&'static str`**：动态字符串走 `textclass::static_kind()` 桥
- **HubError 没有 From<serde_json::Error>**：JSON 解析用 `.map_err(|e| HubError::sensor(...))?`
- **Redis 健康格（每轮写）与 sweephist 环（只记成功轮）是两条基线**——读 delta 时必须 ring-first，别混
- **gdelt 429** 是共享出口 IP 惩罚箱，自愈，别当 bug 修
- 失败源**保持可见**（用户决策）：清晰错误文案 + 自动重试 + 上游批准后自愈（acled/reliefweb/bls/x 现都在此状态）
- **Leaflet 视图未就绪禁动视图**（2026-09-13 教训）：任何对 `map.flyToBounds` / `setView` / `setZoom` 的 `useEffect`，首次 mount 时必须显式跳过——地图初始化的 `fitBounds` 包在 `requestAnimationFrame` 里还来不及跑，初次触发就抛 "Set map center and zoom first" → React 卸载整树 → 整页黑屏。判断方式：拿 headless Chromium 抓 pageerror 现场。修法：`const skipFirst = useRef(true)` + effect 头部 `if (skipFirst.current) { skipFirst.current = false; return; }`
- **React render 崩会变全黑**：React 18+ 无 error boundary 时未捕获的 render 异常会卸载根 → `#root` 空、内容全黑。快速诊断：装 `playwright` + headless chromium，调本地页加载，`page.on("pageerror", e => ...)` 抓控制台错误（参考 `console/probe-monitor.mjs` 模式），比读代码快一个数量级
- **`sources` 表存的是网站来源（origin, base_url），不是 monitor 收集器**——监控源健康在 Redis health cell（`monitor:health:<name>` HSET）。任何 `SELECT source, last_status FROM sources` 都会报错，因为列名是 `origin/base_url/source_id/reputation/first_seen`。用 redis-cli 看监控健康：`redis-cli HGETALL monitor:health:<name>`
- **embed 阈值对 OSINT 偏低**（2026-09-13 教训）：`HUB_EMBED_MIN_WORDS` 默认 300 拒了 98% 文档（PG bucket：461 <30w / 439 30-99w / 5 ≥300w）。现默认 50。改阈值后必须加迁移 reset SKIPPED → PENDING（见 `0007_retry_short_docs.sql`）才能让存量文档重生
- **SearXNG 默认引擎从数据中心 IP 全 ban**（2026-09-13 教训）：duckduckgo CAPTCHA、startpage 限流、bing/google 403 → 24h 内 48 个 DOWN 告警。设 `use_default_settings: false` + 显式禁用黑名单引擎 + 启用 mojeek/brave/wikipedia/arxiv。Compose 卷 `config/searxng:/etc/searxng:rw` 会自动 reload

## Agent/MCP

- REST 鉴权：`Authorization: Bearer ihk_<hex>`（key 在 VM `core/agent-keys.txt`，按 hash 认证）
- MCP 端点 `POST /mcp`；clientInfo 自动吸附到 key 对应 agent 行（version + last_seen_at）
- agent 名册：pi（活跃）、codex（预留）、console
- **MCP discoverability**（2026-09-13 教训）：MCP 客户端未必把 schemars 自动生成的参数名显眼展示，因此 create_claim 期望 `evidence_document_ids`、create_finding 期望 `claim_text`、create_relationship 期望 `rel_type` 经常踩坑。两个零开销发现工具：`intelhub_list_tools()` 返 30 个工具名 + 一行描述；`intelhub_tool_schema(name)` 返任意工具的完整 JSON Schema（含参数名/类型/必填）。用法：遇到 "missing field X" 先调 `tool_schema` 查实际字段名
- **hybrid_search 排名区分度**（2026-09-13 教训）：RRF K=60 时 rank 1/3 差距 ~3%，下游阈值过滤几乎失效。现 K=10 + min-max 归一化到 `[0,1]`（`rrf_norm` 字段），同时保留原始 `rrf_score`。Top-3 间距 15%，5× 提升
- **embed worker 滞后会沉默吃掉 semantic_search 召回**（2026-09-13 教训）：worker 5s/job，新爬文档不能立刻查到。`crawl_url` 新增 `await_embed: bool`（默认 false），true 时阻塞轮询 embedding_status 最长 30s，返 `embedding_status` + `embed_waited_ms`。**不要**用 `await_embed=true` 做批量 ingest（会撑爆 timeout），只用于"先爬立刻查"的同步路径
- **Neo4j 节点 id 用于日志关联**（2026-09-13 教训）：`query_entity` 现在每行返 `id` 字段（Int64，bolt_to_json 转 JSON number）。**注意**：id 在 REINDEX 时会变，关系操作仍用 (kind, name)；id 仅供 grep/日志追踪
- **Cross-encoder rerank stage**（2026-09-13 部署）：`hybrid_search` 和 `semantic_search` 在 RRF/cosine 召回后，把 top-K 候选送到 T8star `/v1/rerank`（默认 `BAAI/bge-reranker-v2-m3` 多语）重新排序。K=10 默认（生产环境实测 50 太慢）。**默认关闭**：`HUB_RERANK_ENABLED=false` —— flip to true 后才能拿到 `rerank_score` 字段。失败优雅降级（`rerank: "skipped: ..."` 字段可见），不会让搜索失败。代价记录到 `cost_records.kind='rerank_tokens'`，~4 char/token 估算（T8star 不返 usage）
- **trace_id 贯穿全链路**（2026-09-13 部署）：每个 MCP 调用生成 UUID `trace_id`（来自现有 `RequestTrace.trace_id`），自动串到 `cost_records.trace_id` + `embedding_jobs.trace_id` + 响应 JSON 顶层 `trace_id` 字段。**新 REST 端点** `GET /api/v1/traces/{trace_id}` 走一次调用全图：embedding_tokens / rerank_tokens / tool_call 审计 / 触发的 embed jobs 一并返回。**用法**：`curl -H "Authorization: Bearer $KEY" http://10.10.10.41:8800/api/v1/traces/<uuid>`。MCP 响应顶层 `trace_id` 字段拿到 UUID 后立即能 walk。背景 worker（embed 队列、monitor 收集器）传 `None` —— 它们没有父请求
- **Multi-hop Q&A via `investigate(question)`**（2026-09-13 部署）：B 阶段 — 规则化 planner 把 OSINT 问题拆成 2-6 步（hybrid_search / entity_lookup / graph_path / claim_lookup），每步独立 sub-trace-id。5 种 archetype：①currency/BRICS/yuan/de-dollarization → hybrid_search + entity_lookup(NDB)；②conflict/war/sanctions → hybrid + OFAC 关键词；③"relationship between X and Y" → graph_path X → Y；④默认 → hybrid_search；⑤有 investigation_id 时末尾加 claim_lookup。执行器容错——一步失败不影响其他。返回 synthesized evidence chain + 每步 trace_id + investigation_id（无则自动创建）。用法：`tools.intelhub_investigate({question: "BRICS de-dollarization 2026 Q4"})`

## Agent skills

工程技能（to-issues、triage、to-prd、qa、diagnose、tdd 等）会读取下列配置：

### Issue tracker

GitHub Issues on https://github.com/rootazero/IntelHub （public repo，使用 `gh` CLI）。详见 `docs/agents/issue-tracker.md`。

### Triage labels

默认五角色：`needs-triage` / `needs-info` / `ready-for-agent` / `ready-for-human` / `wontfix`。详见 `docs/agents/triage-labels.md`。

### Domain docs

单 CONTEXT：`CONTEXT.md` + `docs/adr/` 位于仓库根。详见 `docs/agents/domain.md`。
- **A — Redis-backed query result cache**（2026-09-13 部署）：hybrid_search / semantic_search / keyword_search 在 Redis 里按 (mode, query, limit, url_contains) 哈希缓存响应 blob，TTL 300s。重复调用 590× 加速（7.1s → 12ms）。所有 search 响应顶层加 `cache: "hit"|"miss"|"disabled"|"error"` 字段；cost_records 新增 kind=`cache_hit`（带 agent_id + trace_id 串联到 D-trace）。环境变量 `HUB_QUERY_CACHE_ENABLED`（默认 true）+ `HUB_QUERY_CACHE_TTL_SECS`（默认 300）。模块 `hub-core/src/cache.rs` 暴露 `cache_key()` + `search_with_cache()` helper。模式隔离：mode 字节进入 hash 输入，hybrid/semantic/keyword 三套缓存互不污染。例：`tools.intelhub_hybrid_search({query:"BRICS"})` 第一次 `cache:"miss"`、第二次 `cache:"hit"`
- **B — LLM-driven planner fallback**（2026-09-13 部署）：investigate() 在 rule-based plan 退化到只剩 1 步 hybrid_search 时，调 T8star `/v1/chat/completions` 让模型拆成 2-4 步。native `async fn in trait`（不用 async_trait crate），5s+100ms 超时，解析失败/空/hallucinated tool 都静默回退到 rule 计划。response 顶层 `planner: "rule"|"llm"|"llm_fallback"`，off by default（`HUB_LLM_ENABLED=false`）。模型默认 `gpt-4.1-mini`，预算约 800 tokens/call。cost_records kind=`llm_tokens`，~4 char/token 估算
