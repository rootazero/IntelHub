# IntelHub — Agent 操作手册

> 给后续会话的直接使用手册。所有命令均在实际部署中验证过（最后更新 2026-09-13，main @09ece20）。

## 主机与路径

| 项 | 值 |
|---|---|
| **生产宿主机** | PVE40（Proxmox），VM 410（4 vCPU，8G RAM，balloon 0）。**生产环境，严禁测试** |
| 生产 VM 地址 | `10.10.10.41`，ssh 别名 **`IntelHub`**（免密已配），用户 `zou` |
| **测试宿主机** | PVE40 节点上 VM 415（4 vCPU，8G RAM，UEFI，OVMF）。从模板 9000 (debian-13-cloud) 全量克隆 |
| 测试 VM 地址 | `10.10.10.45`，ssh 别名 **`IntelHub-test`**（免密已配），用户 `zou` |
| VM 部署目录 | `/home/zou/IntelHub`（rsync 目标 + 构建现场） |
| Mac 主仓库 | `/Volumes/TBU/Workspace/IntelHub`（网络盘，有同步延迟） |
| GitHub | `https://github.com/rootazero/IntelHub`（**PRIVATE**，push over HTTPS） |
| hub 服务 | systemd 原生 `hub-core`（唯一的非 docker 例外），端口 8800 |
| 数据层 | docker：`intelhub-postgres` / `intelhub-redis`（compose/compose.base.yml） |

## 开发流程（铁律）

> **生产 / 测试隔离**：所有改动先在 **`IntelHub-test`** 验证通过，再合并到 main 并部署到 **`IntelHub`**。410 是生产服务，绝对不允许测试用——任何 `ssh IntelHub` 之前必须确认改动已在 415 验收全绿。

1. **worktree 隔离**：`cd /Volumes/TBU/Workspace/IntelHub && git worktree add ../IntelHub-<suffix> -b feat/<name>`（网络盘有同步延迟，紧接着操作前 `sleep 4`；失败的 worktree → `rm -rf ../IntelHub-<suffix> && git branch -D feat/<name> && git worktree prune`）
2. 在 worktree 里改代码
3. **rsync → 测试 VM 构建 → 重启 → 验收**（命令见下）—— **全在 IntelHub-test 上**
4. 415 验收全绿 → `git add -A && git commit` → 主仓库 `git merge --no-ff` → `git worktree remove` + `git branch -d`
5. **再次 rsync → IntelHub 生产部署**（与 415 同样的编译/重启序列）
6. 生产验收（sp2a/sp2b/sp3/sp6/sp7/sp8/sp9 全绿）
7. **`git push origin main`**
8. **绝不**：① 直接在 main 工作区改；② 跳过 415 验收直接动 410；③ 在 410 上跑 `build-*` / `restart hub-core` 当作测试；④ 提交 secrets；⑤ 在 pve40 宿主机上装包/改配置/留垃圾文件（VM 内部 disk 操作仅限 losetup 临时挂载修复 SSH 这种例外场景，事后立刻清理 losetup + 卸载 + rm 临时文件）

## 部署命令（每次必走的完整序列）

### 阶段 1：IntelHub-test 验收（必做）

```bash
# 1. rsync 到测试 VM（exclude 清单固定，照搬）
cd /Volumes/TBU/Workspace/IntelHub-<suffix> && rsync -az --delete \
  --exclude '.git/' --exclude 'backups/' --exclude '.DS_Store' \
  --exclude 'compose/.env' --exclude 'compose/.env.crucix' --exclude 'docs/' --exclude 'build/' \
  --exclude 'config/searxng/' --exclude 'hub-core/target/' \
  --exclude 'console/node_modules/' --exclude 'console/dist/' \
  --exclude 'core/' --exclude 'data/' \
  ./ IntelHub-test:/home/zou/IntelHub/

# 2. 测试 VM 构建 + 重启
ssh -o BatchMode=yes IntelHub-test 'cd /home/zou/IntelHub \
  && bash scripts/build-hub.sh 2>&1 | grep -E "^error|built" | head -8 \
  && bash scripts/build-console.sh 2>&1 | tail -1 \
  && sudo systemctl restart hub-core && sleep 4 && systemctl is-active hub-core'

# 3. 415 验收全绿
KEY=$(ssh -o BatchMode=yes IntelHub-test 'grep "api_key:" /home/zou/IntelHub/core/agent-keys.txt | head -1 | grep -o "ihk_[a-f0-9]*"')
for a in sp8 sp6 sp7 sp3; do
  python3 scripts/accept-$a.py "$KEY" 2>&1 | grep -E '==.*(passed|failed)' | tail -1
done
```

### 阶段 2：合并 main → 部署 IntelHub（仅 415 全绿后）

```bash
git add -A && git commit
cd /Volumes/TBU/Workspace/IntelHub && git merge --no-ff feat/<name> && git worktree remove ../IntelHub-<suffix> && git branch -d feat/<name>

# 与 415 同样的 rsync + 构建 + 重启，目标换成 IntelHub
cd /Volumes/TBU/Workspace/IntelHub && rsync -az --delete \
  --exclude '.git/' ...（同 415 exclude 清单）
  ./ IntelHub:/home/zou/IntelHub/

ssh -o BatchMode=yes IntelHub 'cd /home/zou/IntelHub \
  && bash scripts/build-hub.sh ... && bash scripts/build-console.sh ... \
  && sudo systemctl restart hub-core && sleep 4 && systemctl is-active hub-core'

# 生产验收（同样 sp8/sp6/sp7/sp3）
KEY=$(ssh -o BatchMode=yes IntelHub '...')
for a in sp8 sp6 sp7 sp3; do python3 scripts/accept-$a.py "$KEY"; done

git push origin main
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

- **测试 VM 验收**：`KEY=$(ssh -o BatchMode=yes IntelHub-test '...')` + 同样的脚本
- 当前基线：sp2a 19 · sp2b 33 · sp3 19 · sp4 25 · sp5 9 · **sp6 18 · sp7 24 · sp8 18 · sp9 14**。改动某个面时对应脚本必须加检查项并保持全绿。

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

## 测试 VM 基础设施（VM 415 IntelHub-test）

> 所有会话在动 410 之前必须用 415 验过；下面是后续会话直接拿来用的全部信息。

### 创建与配置（首次会话已完成，复用即可）

```bash
# 从模板 9000 (debian-13-cloud) 全量克隆（pve40 节点上）
ssh root@10.10.10.40 'qm clone 9000 415 --name IntelHub-test --full true --storage local-lvm'
ssh root@10.10.10.40 'qm set 415 --cores 4 --memory 8192 --balloon 0 --boot order=scsi0 --bios ovmf \
  --efidisk0 local-lvm:1,efitype=4m,ms-cert=2023k,pre-enrolled-keys=1,size=4M \
  --net0 virtio,bridge=vmbr0,firewall=1 --onboot 1'

# cloud-init：用户 zou + 我的 ed25519 pub key + 静态 IP
ssh root@10.10.10.40 'qm set 415 --ciuser zou --sshkeys <(cat ~/.ssh/intelhub-test/id_ed25519.pub) \
  --ipconfig0 ip=10.10.10.45/24,gw=10.10.10.1'
ssh root@10.10.10.40 'qm cloudinit update 415'
ssh root@10.10.10.40 'qm start 415'

# VM 起来后装 qemu-guest-agent（agent 是 static unit，需手动 enable）
ssh IntelHub-test 'sudo apt-get update -y && sudo DEBIAN_FRONTEND=noninteractive apt-get install -y qemu-guest-agent \
  && sudo systemctl enable --now qemu-guest-agent'
```

### SSH 别名（Mac `~/.ssh/config`）

```
Host IntelHub-test
    HostName 10.10.10.45
    User zou
    IdentityFile ~/.ssh/intelhub-test/id_ed25519
    StrictHostKeyChecking accept-new
    UserKnownHostsFile ~/.ssh/known_hosts IntelHub-test
```

### 测试 VM 专用密钥对（IntelHub-test 专用，不用于 410）

- **路径**：`~/.ssh/intelhub-test/id_ed25519`（Mac 本地）
- **公钥指纹**：`SHA256:1clCZK3NaxqR1lQhZQ/q2qSyzdwi99CKkDafWv9fr80`（comment: `intelhub-test-mac-pi`）
- **私钥内容**（仅 415 测试用，泄露立即 `ssh-keygen -t ed25519 -f ~/.ssh/intelhub-test/id_ed25519 -C intelhub-test-mac-pi-NEW` 重生成 + 替换 415 的 authorized_keys）：

```
-----BEGIN OPENSSH PRIVATE KEY-----
b3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gtZWQyNTUx
OQAAACBQofH6DYUAlrb/A0VEtNdY4xhmfr1SiKodj5ZxH/1R6wAAAJjV0VN81dFTfAAAAAtz
c2gtZWQyNTUxOQAAACBQofH6DYUAlrb/A0VEtNdY4xhmfr1SiKodj5ZxH/1R6wAAAEB/0HQH
JGnYOMwmZsGjmfwnDJC+MRaQYpuhFVo1nhwpA6P8mVoBJ5HEgNZJKekHjEXJ4dMiX7t+jeMM
fFfmcOQHm5RxA8nHDkkEx9Lc/8v7Huc7Kb9XIBkRz1eis6lv4GTkmpxSnjvkajsZLbbMQGyN
OQTHhFxoxm+JwImRf+Qp0eI/SRYzWohYMVttnA1dcsYxkj38JzxhPxIQAaBLBdAVxv1OELdB
5H/hkmJLkqWdOOnShcYI/92YPnz5eSfDpYxuhBsX9wsVfb7HJ1tUL+akAAAADWludGVsaHVi
LXRlc3QtbWFjLXBp
-----END OPENSSH PRIVATE KEY-----
```

- **公钥**（已注入 415 authorized_keys）：
  `ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIFCh8foNhQCWtv8DRUS4113jWGZ+vVKIqh2PlnEf/VHr intelhub-test-mac-pi`

### 已知边界

- VM 415 **BIOS = ovmf（UEFI）**——模板 9000 默认 legacy BIOS 启动会卡（无 OVMF pflash），必须显式 `--bios ovmf` + `--efidisk0`
- cloud-init 默认 **禁用密码登录**（`ssh_pwauth: false`）——cipassword 不会生效，必须靠 SSH key
- qemu-guest-agent 包安装后服务是 **static unit**（`/usr/lib/systemd/system/qemu-guest-agent.service`），必须 `systemctl enable --now` 手动起
- PVE 端 `qm guest cmd 415 ...` 偶发空响应；改用 `pvesh create /nodes/pve40/qemu/415/agent/ping`（pve30 代理）总是稳

### pve40 宿主机操作禁令（血泪）

- **不装包**：不要 `apt-get install` 任何东西（即使想 `socat`/`sshpass` 这类临时工具也不行）
- **不改配置**：不动 `/etc/network/interfaces`、`/etc/ssh/`、`/etc/fstab` 等
- **不留垃圾文件**：所有写到 pve40 `/tmp` 的临时文件（公钥、log、qemu screendump 等）**用完立即 `rm`**
- **不破坏 VM**：losetup/lvm 操作仅限修 SSH 这种阻塞场景，事后必须 `losetup -d` + `umount` + 清零 `/tmp`
- 唯一允许：经 qm/pvesh/qemu-monitor 操作 VM 410/415/9000——这些是 PVE 标准运维

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
- **VM 410 的 .git 是 stale worktree pointer**（2026-09-14 教训）：早期 `git worktree add` 后本机 worktree 被删除，但 VM 上 `/home/zou/IntelHub/.git` 仍是指向 `gitdir: /Volumes/TBU/Workspace/IntelHub/.git/worktrees/<name>` 的文件，Mac 端目录消失后 `git pull` 全部静默失败。修法：删 `.git` 文件 → `git init -b main` → `git remote add origin https://github.com/rootazero/IntelHub.git` → `git fetch origin main` → `git reset --hard origin/main` → `git branch --set-upstream-to=origin/main main`。**注意**：`git reset --hard` 会删 .gitignore 里的未跟踪文件，包括 `core/hub` (binary)、`core/hub.env`、`core/secrets.env`、`core/agent-keys.txt`、`core/console-build.env` 等等（这些是故意不提交的 runtime secrets/build artifacts）。修 git 之后必须重新构建（`bash scripts/build-hub.sh && bash scripts/build-console.sh`）+ 重新生成 env（手动拷 `core/hub.env` 模板或调 install.sh 的 `step_secrets`）+ 重新跑 `core/hub create-agent --name agent|console` 重建 keys。教训：**不要在带 searxng/carto/secrets/agent-keys 的部署上跑 `git reset --hard origin/main`**，要保护 `core/` 目录（cp -r 备份再 reset），或用 `git checkout origin/main -- .` 只更新 tracked files。
- **`scripts/_remote.py` 是 sp* 验收的 ssh-or-local 统一抽象**（2026-09-14 教训）：旧 sp2b/sp4-sp8/sp9 都手写 `subprocess.run(["ssh", "-o", "BatchMode=yes", SSH_HOST, cmd])`，VM 410 没有自己的 ssh 私钥会 "Permission denied (publickey)"，导致 sp9.check_11 baseline-regression bar 子脚本全部 `rc=2 (tail:)` 空失败，8 个月来没人发现。修法：统一用 `from _remote import sh, pg, redis, cypher`，模块内部**自动检测** `$INTELHUB_HOME/core/hub` sentinel（编译后的二进制，gitignored，只在真实部署中存在）决定走 ssh 还是 local。无需 `INTELHUB_LOCAL=1` 旁路。**API**：`sh(cmd)` 跑 shell 返 stdout；`pg(query)` psql 单语句；`pg_stdin(sql)` psql 多语句 via base64 stdin；`redis(*args)` redis-cli（密码自动从 compose/.env 抓取并缓存）；`cypher(query)` cypher-shell（密码自动从 `docker inspect NEO4J_AUTH` 抓取并缓存）；`pg_params(sql, *params)` psql 带 `$1`/`$2` 占位符替换（test-fixture only，不可信输入请用 `pg_stdin`）；`grafana_creds() / grafana_request(path)` Grafana API helpers（密码从 compose/.env 抓取）。**新写验收脚本必须用 `_remote`**——直接 `subprocess.run(["ssh", ...])` 会被 linter 抓住。
- **Grafana 密码在 compose/.env 失同步**（2026-09-14 教训）：sp4.check_08 `grafana datasource Prometheus provisioned` / `grafana IntelHub Overview dashboard provisioned` 在 410 上失败是因为 `compose/.env` 里 `GRAFANA_ADMIN_PASSWORD=$(cat /proc/sys/kernel/random/uuid)` 这个 shell 模板没被展开过（一次手写或早期脚本 bug），但运行中的 grafana 容器实际密码是另一个真 uuid。诊断：`docker inspect intelhub-grafana --format "{{range .Config.Env}}{{println .}}{{end}}" | grep GF_SECURITY_ADMIN_PASSWORD` 拿真密码。修法：`sed -i "s|GRAFANA_ADMIN_PASSWORD=.*|GRAFANA_ADMIN_PASSWORD=$(真密码)|" compose/.env`。**预防**：resolve-versions.sh 用 `$(rand)` (openssl hex)，这版是对的；以后要避免再硬编码 `$(cat /proc/...)` 模板到被 docker-compose 解析的 env 文件里——docker-compose 不会展开 `$(...)`。
- **sp4-sp8 baseline regression 表面上是 env 缺失**（2026-09-14 调查）：5 个 SP 各自 1 个 fail，都是外部 API key 未配置。sp4: Grafana 密码（已修）。sp5: `FIRMS_MAP_KEY` 未配置（NASA FIRMS 需免费注册 firms.modaps.eosdis.nasa.gov/）。sp6: 同样 `FIRMS_MAP_KEY + ACLED_EMAIL` 未配置。sp7: `FINANCIALDATASETS_API_KEY` 未配置（需付费 service）。sp8: `VITE_CARTO_KEY` / `VITE_STADIA_KEY` 未配置（已用红色 "DARK MAP KEY MISSING" 徽章提示）。这些是**独立的 user-decision workstream**——需要用户决定（a）申请真实 key 让 collector 上线，或（b）在测试里承认服务 degraded-by-design 并跳过相应检查。代码 ready 不用动。
- **sp* 验收 3-state 状态（passed / shelved / failed）**（2026-09-14）：sp5/6/7 现在用 `check_shelved(name, reason)` 来处理 missing-API-key 场景——打印 `SHELVE` 行、计入独立的 `shelved` bucket、summary 改为 `== N passed, K shelved, M failed ==`。**退码仅取决于 `failed`**（shelved 不计为 failure）。用于服务 shelved-by-design 场景。例：sp5 现在是 `6 passed, 3 shelved, 0 failed`——3 个 FIRMS/Telegram 检查自动跳过因为没 key。判定逻辑：`if not vm("grep ^KEY= core/secrets.env | cut -d= -f2-").strip(): check_shelved(...)`。shelved 不能替代真 fail：如果上游源实际应有数据但没有（比如 key 已配置但 collector 死了），该 fail 还是 fail。
- **v1 create_claim 补齐 link_claim_evidence emit**（2026-09-14 教训）：graphw.rs::create_claim 写 PG claim_evidence 但**忘了**往 graph_sync_queue 推 `link_claim_evidence` v1 op，导致 Neo4j 镜像永远少 `:SUPPORTS` 边（sp10 check_32 长期 PG=Neo4j+1）。修法：每个 PG insert 后**无条件**调用 `graph_write({type:link_claim_evidence,...})`——Neo4j MERGE 端点+边是 idempotent 的，所以 re-run 同一 claim+doc 不会有重复边，但**新 claim 用旧 doc 也能拿到自己的边**（关键设计点）。关系硬编码 `supports` 因为 mcp.rs::ClaimIntent 和 claim_evidence 表都没有 relation 字段。如果 PG ON CONFLICT 触发（重复）则 graph_write 不需要被 gate——因为 Neo4j 这边用的是 claim_id+document_id 复合 edge key，重复 emit 也不会创建重复边。
- **resolve-versions.sh 自动检测 stale `$(...)` 模板**（2026-09-14 教训）：VM 410 的 compose/.env 手改后留下 `GRAFANA_ADMIN_PASSWORD=$(cat /proc/sys/kernel/random/uuid)` 模板未展开，docker-compose 不展开 `$(...)` env 文件语法，结果运行中的 container 用另一个密码，compose/.env 完全误报。修法：`verify_templates()` 函数 grep `^[A-Z0-9_]+_(PASSWORD|TOKEN|KEY)=.*\$\(` 匹配，用 `sed -i -E 's#^([A-Z0-9_]+_(PASSWORD|TOKEN|KEY))=\$\(.*\)#\1=$(rand)#'` 改写为 `$(rand)` 让下一次 heredoc/dind 展开。重跑幂等。**ERE sed 坑**：`\)` 在 ERE 模式里**不是**字面 close paren——必须用裸 `)`。前缀字符类要包含数字 (`[A-Z0-9_]+`) 因为 `CRAWL4AI_API_TOKEN` 这种名字里有数字。dry-run 模式下 verify_templates 必须只 grep 不 sed（否则破坏 no-side-effect 语义）。

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
