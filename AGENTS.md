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
