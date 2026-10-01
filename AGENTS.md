# IntelHub — Agent 操作手册

> 给后续会话的直接使用手册。所有命令均在实际部署中验证过（最后更新 2026-09-30，main @fb96eac）。

## 主机与路径

| 项 | 值 |
|---|---|
| **生产宿主机** | PVE40（Proxmox），VM 410（4 vCPU，8G RAM，balloon 0）。**生产环境，严禁测试** |
| 生产 VM 地址 | `10.10.10.41`，ssh 别名 **`IntelHub`**（免密已配），用户 `zou` |
| **测试宿主机** | **PVE40** 节点上 **VM 415**（4 vCPU，8G RAM，UEFI，OVMF；复用了旧 VM 415 的 ID——旧那个 10.10.10.45 IntelHub-test 已销毁，新机器 10.10.10.45 Debian-test 拿到同一 VMID 槽且复用同一 IP）。**2026-09-30 由 PVE30 上的 VM 315 重部署到 PVE40**：IP/MAC/hostname/SSH 用户+密钥全部不变；ssh host key 因 fresh clone 自然换；与 IntelHub (VM 410) **共享同一 PVE 宿主机**——host-isolation 已失效（详见下方「规则 3」）。**2026-10-01 IP 重映射**：测试 VM IP 从 `.35` 切换到 `.45`（reclaim 旧 IntelHub-test 的 .45 槽——该 IP 自 2026-09-17 销毁起未使用），hostname/MAC/VMID 不变，ssh host key 不变（PVE40 端 vm config + netplan 内同时改）。从模板 9000 (debian-13-cloud) 全量克隆。 |
| 测试 VM 地址 | `10.10.10.45`，ssh 别名 **`Debian-test`**（免密已配），用户 `zou` |
| VM 部署目录 | `/home/zou/IntelHub`（rsync 目标 + 构建现场） |
| Mac 主仓库 | `/Volumes/TBU/Workspace/IntelHub`（网络盘，有同步延迟） |
| GitHub | `https://github.com/rootazero/IntelHub`（**PRIVATE**，push over HTTPS） |
| hub 服务 | systemd 原生 `hub-core`（唯一的非 docker 例外），端口 8800 |
| 数据层 | docker：`intelhub-postgres` / `intelhub-redis`（compose/compose.base.yml） |

## ⚠️ 网络可达性（2026-09-27 校正）

**老假设**（已过时）：Windows 环境 (10.10.10.5) 不能 ssh 到 Debian-test (10.10.10.45) / IntelHub (10.10.10.41)，Mac (10.10.10.4) 也到不了 VM，必须用户亲自跑部署。

**新事实**（2026-09-27 验证）：Windows 环境用 ssh-key + ssh config（`Debian-test` / `IntelHub`）**可以直接**到两台 VM，**Mac 同样可以**（之前是 Mac→VM 的网络限制，与 Windows↔VM 路径无关）。Windows env / Mac 都和用户一样可以执行全流程——`worktree → rsync/scp → build → restart → sp6 → git push`。

**含义**：
- Agent 可以独立完成从代码改动到生产部署 + 验收的完整链条
- 不需要等待用户在终端执行
- 但 agent 必须严格遵守本文件所有铁律（生产/测试隔离、5-min cooldown、.git stale fix 等）

## 开发流程（铁律）

> **生产 / 测试隔离**（2026-09-30 后已弱化）：所有改动先在 **`Debian-test`** 验证通过，再合并到 main 并部署到 **`IntelHub`**。410 是生产服务，绝对不允许测试用——任何 `ssh IntelHub` 之前必须确认改动已在 Debian-test 上验收全绿。
>
> **⚠️ 2026-09-30 起变化**：Debian-test 已迁移到 PVE40，与 IntelHub (VM 410) **共享同一宿主机**——以前"测试机崩了不影响生产"的假设已失效。两台 VM 现在共用 PVE40 的 bridge fdb / openclash NAT / conntrack 表，stampede risk 会同时影响两者（详见下方「规则 3」）。

1. **worktree 隔离**：`cd /Volumes/TBU/Workspace/IntelHub && git worktree add ../IntelHub-<suffix> -b feat/<name>`（网络盘有同步延迟，紧接着操作前 `sleep 4`；失败的 worktree → `rm -rf ../IntelHub-<suffix> && git branch -D feat/<name> && git worktree prune`）
2. 在 worktree 里改代码
3. **代码改动完成后，在 Debian-test（PVE40 上的测试 VM）上完整验收**（命令见下）—— **全在 Debian-test 上**
4. 测试 VM 验收全绿 → `git add -A && git commit` → 主仓库 `git merge --no-ff` → `git worktree remove` + `git branch -d`
5. **再次部署 IntelHub 生产**（远端 ssh 进 410，**在 410 本机跑 `update.sh`**，与 Debian-test 同样的编译/重启序列；详见下方"🔴 部署铁律"规则 2）
6. 生产验收（sp2a/sp2b/sp3/sp6/sp7/sp8/sp9 全绿）
7. **`git push origin main`**
8. **绝不**：① 直接在 main 工作区改；② 跳过 Debian-test 验收直接动 410；③ 在 410 上跑 `build-*` / `restart hub-core` 当作测试；④ 提交 secrets；⑤ 在 pve40 宿主机上装包/改配置/留垃圾文件（VM 内部 disk 操作仅限 losetup 临时挂载修复 SSH 这种例外场景，事后立刻清理 losetup + 卸载 + rm 临时文件）

## 🔴 重启 hub-core 后必须等 5 分钟（2026-09-20 PVE40 崩溃教训）

> **`sudo systemctl restart hub-core` 之后必须等至少 5 分钟才能再做验收 / 重启 / 高流量动作。**

**为什么**：hub-core 重启会 spawn **74 个 monitor 任务同时打外网**（cnn/misp/gdelt/overpass/celestrak/adsb/bluesky/telegram/...），加上 cctv-refresh 内部 11 个 provider 串联 retry（最坏 103s），所有出口走 `openclash` fake-IP（10.10.10.1）。未限速的 reqwest pool + 7-source-per-slot stagger 会在重启瞬间产生 ~70 个并发 TCP 出站请求，击穿 openclash 的 NAT 表 + PVE40 bridge fdb → **PVE40 宿主机断网（`Host is down`）**。P10 ledger + 2026-09-20 实测都记录了这个模式（恢复需要重启 PVE40 硬件）。

**强制等待协议**：
1. `sudo systemctl restart hub-core` 后**至少 `sleep 300`**，才能跑 acceptance
2. 在等待期间，**禁止**再 `systemctl restart hub-core`（背靠背重启会让问题翻倍）
3. 验收前 `sudo journalctl -u hub-core --since "5 minutes ago"` 确认 `cctv-refresh` 一轮跑完（修复后 < 30s，修复前可能 > 60s），且没有 stampede 重启痕迹

**验证 stampede 已经修复**（fix/deploy-stampede 合并后生效）：
```bash
ssh -o BatchMode=yes IntelHub 'sudo journalctl -u hub-core --since "3 minutes ago" --no-pager \
  | grep "cctv-refresh" | tail -3'
# 期望：ms<30000 (修复后 ~5s)。修复前会看到 ms=100000+
```

详见：`docs/superpowers/execution/2026-09-20-deploy-stampede-postmortem.md`

## ⚠️ 2026-09-27 PVE40 stampede 复发（Phase 1 部署后 50+ min 仍在 down）

**时间线**（UTC）：
- 03:48:39 — IntelHub 生产上 `sudo systemctl restart hub-core`（Phase 1 commit a7a2650，含 +8 sources）
- 03:53:39 — sleep 300 完成，cctv-refresh 91s 完成（no stampede，stampede 修复生效）
- 03:57:20 — ssh 第一次 timeout
- 03:57–04:39 — 持续 ssh timeout，PVE40 10.10.10.40 端口 22/8006 都不通，10.10.10.41 (IntelHub VM) 也不通
- 04:39+ — 仍在 down（postmortem 说 38-min host outages，但这次超出）

**Debian-test**（**2026-09-27 时**仍是 PVE30 上的 VM 315）**完全正常**——同样的代码、同样的 key、同样的 build/restart 序列，sp6 36/5/32，Phase 1 全部 PASS。所以**问题不是 hub-core 代码**，而是 PVE40 host 的网络栈被击穿。

> **2026-09-30 历史标注**：本节描述的是 2026-09-27 stampede 复发事件。当时 PVE40 上**只有 VM 410（IntelHub）一个 VM**——Debian-test 在 PVE30 上不受影响，纯属 host-isolation 兜底。2026-09-30 起 Debian-test 已迁到 PVE40，如果今天再发生同样的 stampede，Debian-test 也会一并 down，没有 fallback。

**与上次区别**：上次的 stampede 发生在 74 sources，本次是 82 sources（+8 Phase 1）。pool_max_idle_per_host(8) + scheduler stagger (idx*3)/2 + cctv Semaphore(3) 都已部署 (cd07a7b)。cctv-refresh 91s 完成证明 stampede 修复**对 cctv-refresh 这一层生效**。

**未解释的部分**：
- 为什么 5-min wait 之后 PVE40 host 仍 down？
- 是否 Phase 1 8 个 sources 加起来仍击穿 openclash / bridge fdb？
- 是否需要更严的 rate limit（如 per-source 最小 stagger > 3s，或基于 host 的 reqwest pool 改成 ≤4）？

**恢复需要**：物理重启 PVE40（按 2026-09-20 postmortem 要求）。Agent 无法在网络隔离状态下远程执行。

**后续 action items**（待用户物理恢复后）：
1. 验证 IntelHub 恢复后 hub-core 自动续运行（systemd restart=on-failure）
2. 验证 Phase 1 8 sources 在 IntelHub 上也 PASS sp6
3. 如果再次出现 PVE40 down，考虑 **per-source pool_max_idle_per_host(2)** 或 **scheduler stagger 加到 (idx*5)/3**
4. 调研 openclash 的连接追踪表上限，调整 conntrack

**教训**：
- 即使 stampede 修复已部署，Phase 1 +8 sources（keyless，每个都有自己的 first-sweep URL）可能仍足量击穿 PVE40 的 bridge fdb / openclash NAT
- 后续每次大规模加 source（>5/批次）都应该考虑“分批 restart”（先 restart hub-core，让前 N 个 sources 首轮 sweep 完成，再 add 下一批）
- **接受 retry-restart-on-PVE40-down 场景**，但在代码层提供 `--max-fanout` 参数，给运营留手动 ramp-up 能力

## ⚠️ 2026-09-30 Debian-test 迁回 PVE40 + 初始化快照

**事件**：Debian-test（VM 315）从 PVE30 节点重新部署到 PVE40 节点。IP `10.10.10.45` / MAC / hostname / SSH 用户+密钥全部保留。ssh host key 因 fresh clone 自然换（首连需 `ssh-keygen -R 10.10.10.45`）。**新 VMID = 415**（复用了 PVE40 上旧 VM 415 的 ID 槽——旧那个 10.10.10.45 IntelHub-test 已于 2026-09-17 切换时销毁，新 Debian-test 拿到同一 ID + 同一 IP）。注意：MAC 与旧 VM 415 相同，但 ssh host key 不同——不要把新 VM 415 当成 .45 那台旧机器访问（用 ssh-host-key 区分，不要按 IP）。

**清理后的快照状态**（为后续新 install 提供的已知干净基线）：
- 磁盘 7G used / 88G avail（原 32G 减到 7G，省 25G）
- 零 IntelHub 痕迹：`find / -iname '*intelhub*'` 与 `find / -name 'hub' -type f` 均返回空
- Docker 状态：0 容器、3 个默认网络（bridge/host/none）、0 卷、2 张基础镜像（alpine + curlimages/curl）
- 监听端口：22/53/5355（无 8800）
- 保留：Debian 13 base + zou 用户 + qemu-guest-agent + docker engine 29.8.1（buildx + compose v5.5.1 + containerd）+ cloud-init + sshd/journald 默认配置
- `/etc/docker/daemon.json` 删了（IntelHub-tuned 172.30.0.0/16 pool 没了，docker 回默认 172.17.0.0/16）
- 两个 IntelHub drop-in 也清：`/etc/systemd/journald.conf.d/60-intelhub.conf` + `/etc/ssh/sshd_config.d/60-intelhub.conf`（base sshd_config 仍 PasswordAuthentication no）
- 用户选择：仅清应用层，保留 docker engine/buildx/compose（下次 install.sh 会重装所有东西）
- 重启 sshd + journald service 验证为 active

**host-isolation 后果（最关键）**：Debian-test 与 IntelHub (VM 410) 现在共享 PVE40。后果 1：`pve40 down = 两台 VM 同时 down，没有 fallback`——PVE40 bridge fdb / openclash NAT / conntrack 表被击穿时两台一起没。后果 2：测试机上的 stampede 现在也会把生产一起带崩。后果 3：PVE40 是单点，所有 VM 操作都要更谨慎。

**行动项**：
1. **本次提交后**：本文件已同步更新（头表、rule 3、阶段 1、测试 VM 基础设施、pve40 宿主机禁令）
2. **下次 install.sh**：在快照基础上跑，会自动重装 docker、写 drop-in、配置 service——与原始路径一致
3. ~~**VMID 待补**：用户打快照时记录 PVE UI 上 Debian-test 的新 ID~~（已完成：VMID = **415**，在 PVE UI 上可看到）
4. **memory 已记**：参见 long-term memory 中 `intelhub.testvm-relocation-2026-09-30` + `intelhub.host-isolation-gone-pve40` + `intelhub.testvm-current-state-2026-09-30`

## 🔴 部署铁律（2026-09-27 PVE40 二次崩溃后确立）

### 规则 1：IntelHub（VM 410, 生产）**绝不允许测试**

VM 410 是**生产服务**——任何对 hub-core 的 `restart` / `build` 操作都直接冲击生产。2026-09-27 实测：Phase 1 +8 sources 后 `restart hub-core` 击穿 PVE40 宿主机网络栈 → PVE40 物理断网 → 85+ min 才恢复。

**2026-09-30 注**：当时 PVE40 上只有 VM 410 一个 VM，所以"没有 fallback"指的是"IntelHub 单点没 VM 兜底"。2026-09-30 起 Debian-test 也迁到了 PVE40——现在 PVE40 上有 **VM 410 + Debian-test 两个 VM**，bridge fdb / openclash NAT 表被击穿后**仍然没有 fallback**（只是现在影响范围从 1 个变 2 个：IntelHub 和 Debian-test 一起 down）。

**铁律**：
1. **任何**代码改动先在 **Debian-test（VM 415 on PVE40）** 完整测试（sp2a/sp2b/sp3/sp6/sp7/sp8/sp9 全绿）。
2. **只有全部 sp 验收通过**后才允许在 410 上跑 `update.sh`
3. **永远不要**在 410 上跑 `bash scripts/build-hub.sh` 当成"试一下能不能编译"——这就是"在生产上测试"
4. **永远不要**因为"小改动"跳过 Debian-test 验证直接部署到 410
5. 误操作（410 上手动改文件后 build）的补救：备份 `core/` → 重置 `.git` → 重 fetch → 重置 → 重建

### 规则 2：生产部署走 `update.sh`（在 VM 上跑），**不走** `deploy.sh`（从 Mac rsync）

**禁止**从 Mac / Windows 跑 `bash scripts/deploy.sh`（即 `rsync -az --delete ./ IntelHub:/home/zou/IntelHub/`）到 410。理由：
- `deploy.sh` 是从 Mac 的 `/Volumes/TBU/Workspace/IntelHub`（网络盘）走 LAN rsync 到 410——这条路径与 hub-core restart 时的 74+ monitor 并发出口**叠加**，曾造成 PVE40 bridge fdb 击穿
- `deploy.sh` 推代码 → 410 build + restart 是 stampede 的同样触发场景（deploy + restart 两次叠加）
- `update.sh` 在 410 本机跑，先 `git pull --rebase --autostash`（代码从 GitHub origin HTTPS 拉，**不走 LAN rsync**），再 build + restart——只触发**一次** stampede（restart 本身）

**部署到 410 的唯一正确姿势**：
```bash
# 从 Mac / Windows 远端 ssh 进 410，**在 410 本机**跑 update.sh：
ssh -o BatchMode=yes IntelHub 'cd /home/zou/IntelHub \
  && bash scripts/update.sh 2>&1 | tail -50'
# 然后按 stampede 规则 sleep 300：
ssh -o BatchMode=yes IntelHub 'sleep 300 && systemctl is-active hub-core'
# 再跑 sp 验收：
KEY=$(ssh -o BatchMode=yes IntelHub 'grep -o "ihk_[a-f0-9]*" /home/zou/IntelHub/core/agent-keys.txt | head -1')
python3 scripts/accept-sp6.py "$KEY" "http://10.10.10.41:8800" 2>&1 | grep -E "==.*(passed|failed|shelved)"
```

**`update.sh` 内部做的**（已实现，详见 `scripts/update.sh`）：
- `git pull --rebase --autostash`（代码从 GitHub origin HTTPS 拉，**不走 LAN rsync**）
- `bash scripts/build-hub.sh`
- `bash scripts/build-console.sh`
- `sudo systemctl restart hub-core`
- 轮询 `/api/v1/health` 直到 200
- Track B：docker 组件的 smart-diff 升级

**如果 410 上 `.git` 是 stale worktree 状态**（前次部署留下的），先修复再 `update.sh`：
```bash
ssh -o BatchMode=yes IntelHub 'cd /home/zou/IntelHub \
  && sudo cp -a core /tmp/intelhub-core-backup-$(date +%s) \
  && sudo rm -rf .git && sudo chown -R zou:zou /home/zou/IntelHub \
  && git init -b main && git remote add origin https://github.com/rootazero/IntelHub.git \
  && git fetch origin main && git reset --hard origin/main \
  && sudo cp -a /tmp/intelhub-core-backup-* core && sudo chown -R zou:zou core'
```

### 规则 3：Debian-test（PVE40）允许任何测试——但不再与 IntelHub host-isolation

**2026-09-30 之前**（旧规则）：Debian-test 在 PVE30 上——即使它被 stampede 击穿，影响的也只是 PVE30 上的其他 VM，**不会扩散到 IntelHub / PVE40**。那时候这是兜底层。

**2026-09-30 之后**（v2）：Debian-test 已迁到 PVE40，与 IntelHub (VM 410) **共享同一 PVE 宿主机**。后果 1：`pve40 down = 两台 VM 同时 down，没有 fallback`——PVE40 bridge fdb / openclash NAT 表被击穿时两台一起没。后果 2：测试机上的 stampede 现在也会把生产一起带崩。host-isolation 假设已失效，**生产 / 测试隔离现在只能靠流程纪律，不能靠宿主机隔离**。

所以现在的"允许"列表要收紧：
- **仍然允许**：第一次 build、单元测试、低流量调试、playground、单源 sweep 测试
- **仍要 sleep 300**（避免在同一 PVE 节点上背靠背 restart；2026-09-20/27 的 stampede 在 PVE40 上确证过击穿模式）
- **sp 全绿后才允许进 410**——这条没变
- **新增软限制**：Debian-test 上任何 restart hub-core / 大规模 source fanout 都要先评估"对 IntelHub 的连带影响"，必要时改用 update.sh 走 GitHub HTTPS（避免 LAN rsync 与 restart 叠加，详见规则 2）
- **新增兜底**：PVE40 物理断网时无法远程恢复，必须等硬件层恢复——所以**保持 PVE40 单点稳定的优先级现在比 Debian-test 本身的吞吐更重要**

## 部署命令（每次必走的完整序列）

### 阶段 1：Debian-test（PVE40）验收（必做）

```bash
# 1. rsync 到测试 VM（exclude 清单固定，照搬） — ⚠️ **废弃，改用 update.sh**（见下方新流程）
#    本块保留仅为迁移参考；正式部署不要跑下面这段
cd /Volumes/TBU/Workspace/IntelHub-<suffix> && rsync -az --delete \
  --exclude '.git/' --exclude 'backups/' --exclude '.DS_Store' \
  --exclude 'compose/.env' --exclude 'compose/.env.crucix' --exclude 'docs/' --exclude 'build/' \
  --exclude 'config/searxng/' --exclude 'hub-core/target/' \
  --exclude 'console/node_modules/' --exclude 'console/dist/' \
  --exclude 'core/' --exclude 'data/' \
  ./ Debian-test:/home/zou/IntelHub/

# 2. 测试 VM 构建 + 重启 — ⚠️ **同上废弃**
ssh -o BatchMode=yes Debian-test 'cd /home/zou/IntelHub \
  && bash scripts/build-hub.sh 2>&1 | grep -E "^error|built" | head -8 \
  && bash scripts/build-console.sh 2>&1 | tail -1 \
  && sudo systemctl restart hub-core && sleep 4 && systemctl is-active hub-core'

# 3. 测试 VM 验收全绿
KEY=$(ssh -o BatchMode=yes Debian-test 'grep -o "ihk_[a-f0-9]*" /home/zou/IntelHub/core/agent-keys.txt | head -1')
for a in sp8 sp6 sp7 sp3; do
  python3 scripts/accept-$a.py "$KEY" 2>&1 | grep -E '==.*(passed|failed)' | tail -1
done
```

### 阶段 2：合并 main → 部署 IntelHub（仅测试 VM 全绿后）

```bash
git add -A && git commit
cd /Volumes/TBU/Workspace/IntelHub && git merge --no-ff feat/<name> && git worktree remove ../IntelHub-<suffix> && git branch -d feat/<name>

# ⚠️ 以下 rsync + 重启流程**废弃**。详见上方"规则 2"。仅保留为迁移参考。
cd /Volumes/TBU/Workspace/IntelHub && rsync -az --delete \
  --exclude '.git/' ...（同测试 VM exclude 清单）
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
KEY=$(ssh -o BatchMode=yes IntelHub 'grep -o "ihk_[a-f0-9]*" /home/zou/IntelHub/core/agent-keys.txt | head -1')
for a in sp8 sp6 sp7 sp3; do
  echo "── $a: $(python3 scripts/accept-$a.py "$KEY" 2>&1 | grep -E '==.*(passed|failed)' | tail -1)"
  python3 scripts/accept-$a.py "$KEY" 2>&1 | grep "^FAIL" | head -3
done
```

- **测试 VM 验收**：`KEY=$(ssh -o BatchMode=yes IntelHub-test '...')` + 同样的脚本
- 当前基线：sp2a 19 · sp2b 33 · sp3 19 · sp4 25 · sp5 9 · **sp6 49+5shelved/0f（共 42 项，GEV P2 6 检查位：per-category 卫星地板（10/100/25/20/20/400 按真实星座规模校准）+总>600、flights envelope coverage 容忍 +opensky、earthquakes rows>0、celestrak stations TLE、starlink 代理 TLE、opensky OAuth 缺席 shelved。GEV P3 再 +9 检查位：ais-live 三态信封（无 key shelved）、installations rows>0+bbox+400、overpass 代理 round-trip、tomtom status（无 key shelved flow）、cctv catalog>200+frame 抽查×3、starlink TLE；shelved 不计 failure、退码只看 failed） · sp7 16+11shelved · sp8 54+2shelved+3deferred/0f（P3 45+2sh/0f；P10 +3 检查位：recording body class via setMode regex、`hud-scene-panel` testid in bundle、panel-drag vendor key + adapter wiring。**315 clean baseline tracking** —— 410 生产 baseline 不同但同等稳定） · sp9 14**。改动某个面时对应脚本必须加检查项并保持全绿。

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

## 测试 VM 基础设施（Debian-test on PVE40）

> 所有会话在动 410 之前必须用 Debian-test 验过；下面是后续会话直接拿来用的全部信息。
>
> **2026-09-17 切换说明**：原 VM 415（pve40，10.10.10.45，alias `IntelHub-test`）被替换为 VM 315（pve30，10.10.10.35，alias `Debian-test`）。用户/密码/SSH key/Mac 地址全部保持不变，只是换了一台更近的 Proxmox 节点。所有 `IntelHub-test` 引用改为 `Debian-test`，所有 `10.10.10.45` 改为 `10.10.10.35`，所有 `pve40` 改为 `pve30`。
>
> **⚠️ VMID 复用警告**：2026-09-30 起，新 Debian-test（10.10.10.35）复用了上面说的旧 VM 415 的 PVE ID 槽。所以现在 PVE40 上的 "VM 415" 指的是 Debian-test（.35），不是当年的 IntelHub-test（.45，那个已销毁）。MAC 跟旧 VM 415 相同——别按 MAC 在 Proxmox 里误以为找回了 .45 那台。
>
> **2026-09-30 重部署**：PVE30 上的 VM 315 已重新部署到 PVE40 节点，**新 VMID = 415**（复用了 PVE40 上旧 VM 415 的 ID 槽——旧那个 10.10.10.45 IntelHub-test 已于 2026-09-17 切换时销毁；MAC 与旧 VM 415 相同但 IP/hostname 不同）。IP 10.10.10.35 / hostname / SSH 用户+密钥全部保留——只 ssh host key 因 fresh clone 自然换（首次连接需 `ssh-keygen -R 10.10.10.35`）。**与 IntelHub (VM 410) 现在共享 PVE40 宿主机**——详见上方「规则 3」中 host-isolation 已失效的说明。下方命令示例里的 VMID 现在已统一替换为 `415`，可直接跑；`10.10.10.35` / `Debian-test` ssh alias 不变。
>
> **⚠️ 2026-10-01 IP 重映射（reclaim）**：Debian-test IP 从 `.35` 改回 `.45`（reclaim 旧 IntelHub-test 销毁后空出来的 IP 槽——是 2026-09-17 那次切换中最早 `.45` 那台机器的地址）。hostname/MAC/VMID 不变，ssh host key 不变（不是 fresh clone）。同步修改了 4 处：
>   1. PVE40 端 `qm set 415 --ipconfig0 ip=10.10.10.45/24,gw=10.10.10.1`
>   2. VM 内 `/etc/netplan/50-cloud-init.yaml`（Debian 13 默认网络栈是 netplan+systemd-networkd，**不是** `/etc/network/interfaces`——后者是 ifupdown 旧栈，Debian 12 起已弃用）。`sudo netplan apply` 会瞬断 SSH session（IP 变了），属预期行为。
>   3. Mac `~/.ssh/config`：`HostName` 改为 `.45`；首连可能仍报 "Host key changed"（旧 .45 那台 IntelHub-test 的 ed25519 指纹残留）→ `ssh-keygen -R 10.10.10.45` 一次
>   4. VM 内 `/home/zou/IntelHub/core/hub.env` + `/home/zou/IntelHub/compose/.env`：LAN_IP / HUB_LISTEN_ADDR / SEARXNG_URL / CRAWL4AI_URL / HUB_MCP_ALLOWED_HOSTS 全部从 `.35` 改为 `.45`，然后 `sudo systemctl restart hub-core` + `docker compose --profile optional up -d`（stampede 规则 sleep 300）
>
> **副作用**：`ssh-keygen -R 10.10.10.35` 一次清掉旧 IP 的 host key entry。重新接上 Debian-test 必须用 `ssh Debian-test`（现在走 .45），不要再用 `.35`——它现在 ping 不通。

### 创建与配置（首次会话已完成，复用即可）

```bash
# 从模板 9000 (debian-13-cloud) 全量克隆（PVE40 节点上，VMID = 415）
ssh root@10.10.10.40 'qm clone 9000 415 --name Debian-tester --full true --storage local-lvm'
ssh root@10.10.10.40 'qm set 415 --cores 4 --memory 8192 --balloon 0 --boot order=scsi0 --bios ovmf \
  --efidisk0 local-lvm:1,efitype=4m,ms-cert=2023k,pre-enrolled-keys=1,size=4M \
  --net0 virtio,bridge=vmbr0,firewall=1 --onboot 1'

# cloud-init：用户 zou + 我的 ed25519 pub key + 静态 IP（2026-10-01 起 .45）
ssh root@10.10.10.40 'qm set 415 --ciuser zou --sshkeys <(cat ~/.ssh/Debian-test/id_ed25519.pub) \
  --ipconfig0 ip=10.10.10.45/24,gw=10.10.10.1'
ssh root@10.10.10.40 'qm cloudinit update 415'
ssh root@10.10.10.40 'qm start 415'

# VM 起来后装 qemu-guest-agent（agent 是 static unit，需手动 enable）
ssh Debian-test 'sudo apt-get update -y && sudo DEBIAN_FRONTEND=noninteractive apt-get install -y qemu-guest-agent \
  && sudo systemctl enable --now qemu-guest-agent'
```

### SSH 别名（Mac `~/.ssh/config`）

```
Host Debian-test
    HostName 10.10.10.45
    User zou
    IdentityFile ~/.ssh/Debian-test/id_ed25519
    StrictHostKeyChecking accept-new
    UserKnownHostsFile ~/.ssh/known_hosts Debian-test

# IntelHub-test — legacy alias for the same machine (Debian-test IS the renamed
# IntelHub-test, same VM 415 / MAC / hostname / IP .45). Use this alias when
# reading archived docs/investigations that say `ssh IntelHub-test` (e.g.
# docs/investigation/2026-09-15-e2e-audit/*); semantically identical to
# Debian-test now (was pointing to the old destroyed VM 415 pre-2026-09-17).
Host IntelHub-test
    HostName 10.10.10.45
    User zou
    IdentityFile ~/.ssh/Debian-test/id_ed25519
    StrictHostKeyChecking accept-new
    UserKnownHostsFile ~/.ssh/known_hosts IntelHub-test
```

### 测试 VM 专用密钥对（Debian-test 专用，不用于 410）

> **密钥仅存在于两个地方**，不在 AGENTS.md / 仓库 / 任何文档里写明：
  - Mac 本地：`~/.ssh/Debian-test/id_ed25519`（2026-09-17 重命名时 rekey 过一次；2026-10-01 又 rekey 过一次，因为 2026-09-13 commit 6d9754d 错误地把私钥写进 AGENTS.md 并 push 到了私有仓库）
  - Debian-test VM：`~/.ssh/authorized_keys`（由 `qm set 415 --sshkeys` 注入，或 `ssh-copy-id` 后追加）
>
> **未来再写文档时**——只写 key 路径（`~/.ssh/Debian-test/id_ed25519`）+ 写入流程（`qm set --sshkeys <(cat .pub)` 或 `ssh-copy-id`），绝不抄 base64 body / fingerprint / 公钥。Fingerprint 是公钥的 hash，公开也算 sensitive（缩小攻击面 + 防止 MITM 探测）。
>
> **已执行的清理（2026-10-01）**：`git filter-repo --force --replace-text` 把所有 commit 的 private key 块（markers + base64 body）+ 旧 fingerprint + 旧公钥替换成 `***REDACTED***`；然后 force-push 到 origin。Debian-test authorized_keys 先 add-new（新 key）防 lockout，再 revoke-old。Mac 本地 `~/.ssh/Debian-test/id_ed25519` 整体覆盖为新生成 keypair。

### 已知边界

- PVE40 Debian-test **BIOS = ovmf（UEFI）**——模板 9000 默认 legacy BIOS 启动会卡（无 OVMF pflash），必须显式 `--bios ovmf` + `--efidisk0`
- cloud-init 默认 **禁用密码登录**（`ssh_pwauth: false`）——cipassword 不会生效，必须靠 SSH key
- qemu-guest-agent 包安装后服务是 **static unit**（`/usr/lib/systemd/system/qemu-guest-agent.service`），必须 `systemctl enable --now` 手动起
- PVE 端 `qm guest cmd 415 ...` 偶发空响应；改用 `pvesh create /nodes/pve40/qemu/415/agent/ping`（同节点代理）总是稳

### pve40 宿主机操作禁令（血泪）

- **不装包**：不要 `apt-get install` 任何东西（即使想 `socat`/`sshpass` 这类临时工具也不行）
- **不改配置**：不动 `/etc/network/interfaces`、`/etc/ssh/`、`/etc/fstab` 等
- **不留垃圾文件**：所有写到 pve40 `/tmp` 的临时文件（公钥、log、qemu screendump 等）**用完立即 `rm`**
- **不破坏 VM**：losetup/lvm 操作仅限修 SSH 这种阻塞场景，事后必须 `losetup -d` + `umount` + 清零 `/tmp`
- 唯一允许：经 qm/pvesh/qemu-monitor 操作 VM 410/415/9000（注意：VM 415 = Debian-test on PVE40 @.35；不是旧那个 .45 IntelHub-test，已销毁）——这些是 PVE 标准运维

## 已知坑（血泪）

- **edit 工具**：一次调用里多个 edits 指向同一路径时，若 oldText 跨文件混淆会静默失败——改完务必 `grep` 验证；TSX 深度嵌套 JSX 属性会触发 TS1381（在 map 块体里预计算）
- **雷达默认窗口 24h**：旧日期事件不可见，排查先看 `occurred_at`，验收用 `?from=2026-01-01T00:00:00Z`
- **Signal::new 的 kind 要 `&'static str`**：动态字符串走 `textclass::static_kind()` 桥
- **HubError 没有 From<serde_json::Error>**：JSON 解析用 `.map_err(|e| HubError::sensor(...))?`
- **Redis 健康格（每轮写）与 sweephist 环（只记成功轮）是两条基线**——读 delta 时必须 ring-first，别混
- **gdelt 429** 是共享出口 IP 惩罚箱，自愈，别当 bug 修
- 失败源**保持可见**（用户决策）：清晰错误文案 + 自动重试 + 上游批准后自愈（acled/reliefweb/bls/x 现都在此状态）
- **sp6 secret() 必须查两个 env 文件**（2026-09-27 教训）：hub-core 的 systemd unit 使用两个 `EnvironmentFile=`（`core/secrets.env` + `core/hub.env`），systemd 按顺序 merge，后到的覆盖前到的。运行中进程能看到任一文件里的 key。sp6 的 `secret()` 原本只查 `secrets.env` → HUB_OPENAQ_API_KEY 写在 `hub.env` 时被误判为 SHELVE。修法：`secret()` 先查 `secrets.env`，空则回退 `hub.env`。任何**新增 env-gated source 都应同时检查两文件**
- **sp6 secret() 需依赖文件可读**（2026-09-27 教训）：`core/secrets.env` / `core/hub.env` / `core/agent-keys.txt` / `core/admin-token.txt` / `core/console-build.env` 在 install 时是 `root:root 0600`，sp6 作为 user `zou` 跑会报 `Permission denied`（`grep` 静默失败，sp6 认为 key 不存在 → SHELVE）。即使 systemd unit 是 root 能读，但如果 sp6 本身读不到就报 SHELVE。修法：`sudo chmod 0640 /home/zou/IntelHub/core/{secrets,hub}.env /home/zou/IntelHub/core/{admin-token,agent-keys,console-build}.txt && sudo usermod -a -G root zou`（zero group = root；让 zou 进 root group + 文件 mode 0640 = group-readable）。**新增 secret() 路径前先 verify 文件 mode + group membership**
- **Leaflet 视图未就绪禁动视图**（2026-09-13 教训）：任何对 `map.flyToBounds` / `setView` / `setZoom` 的 `useEffect`，首次 mount 时必须显式跳过——地图初始化的 `fitBounds` 包在 `requestAnimationFrame` 里还来不及跑，初次触发就抛 "Set map center and zoom first" → React 卸载整树 → 整页黑屏。判断方式：拿 headless Chromium 抓 pageerror 现场。修法：`const skipFirst = useRef(true)` + effect 头部 `if (skipFirst.current) { skipFirst.current = false; return; }`
- **React render 崩会变全黑**：React 18+ 无 error boundary 时未捕获的 render 异常会卸载根 → `#root` 空、内容全黑。快速诊断：装 `playwright` + headless chromium，调本地页加载，`page.on("pageerror", e => ...)` 抓控制台错误（参考 `console/probe-monitor.mjs` 模式），比读代码快一个数量级
- **`sources` 表存的是网站来源（origin, base_url），不是 monitor 收集器**——监控源健康在 Redis health cell（`monitor:health:<name>` HSET）。任何 `SELECT source, last_status FROM sources` 都会报错，因为列名是 `origin/base_url/source_id/reputation/first_seen`。用 redis-cli 看监控健康：`redis-cli HGETALL monitor:health:<name>`
- **embed 阈值对 OSINT 偏低**（2026-09-13 教训）：`HUB_EMBED_MIN_WORDS` 默认 300 拒了 98% 文档（PG bucket：461 <30w / 439 30-99w / 5 ≥300w）。现默认 50。改阈值后必须加迁移 reset SKIPPED → PENDING（见 `0007_retry_short_docs.sql`）才能让存量文档重生
- **SearXNG 默认引擎从数据中心 IP 全 ban**（2026-09-13 教训）：duckduckgo CAPTCHA、startpage 限流、bing/google 403 → 24h 内 48 个 DOWN 告警。设 `use_default_settings: false` + 显式禁用黑名单引擎 + 启用 mojeek/brave/wikipedia/arxiv。Compose 卷 `config/searxng:/etc/searxng:rw` 会自动 reload
- **VM 410 的 .git 是 stale worktree pointer**（2026-09-14 教训）：早期 `git worktree add` 后本机 worktree 被删除，但 VM 上 `/home/zou/IntelHub/.git` 仍是指向 `gitdir: /Volumes/TBU/Workspace/IntelHub/.git/worktrees/<name>` 的文件，Mac 端目录消失后 `git pull` 全部静默失败。修法：删 `.git` 文件 → `git init -b main` → `git remote add origin https://github.com/rootazero/IntelHub.git` → `git fetch origin main` → `git reset --hard origin/main` → `git branch --set-upstream-to=origin/main main`。**注意**：`git reset --hard` 会删 .gitignore 里的未跟踪文件，包括 `core/hub` (binary)、`core/hub.env`、`core/secrets.env`、`core/agent-keys.txt`、`core/console-build.env` 等等（这些是故意不提交的 runtime secrets/build artifacts）。修 git 之后必须重新构建（`bash scripts/build-hub.sh && bash scripts/build-console.sh`）+ 重新生成 env（手动拷 `core/hub.env` 模板或调 install.sh 的 `step_secrets`）+ 重新跑 `core/hub create-agent --name agent|console` 重建 keys。教训：**不要在带 searxng/carto/secrets/agent-keys 的部署上跑 `git reset --hard origin/main`**，要保护 `core/` 目录（cp -r 备份再 reset），或用 `git checkout origin/main -- .` 只更新 tracked files。
- **`scripts/_remote.py` 是 sp* 验收的 ssh-or-local 统一抽象**（2026-09-14 教训）：旧 sp2b/sp4-sp8/sp9 都手写 `subprocess.run(["ssh", "-o", "BatchMode=yes", SSH_HOST, cmd])`，VM 410 没有自己的 ssh 私钥会 "Permission denied (publickey)"，导致 sp9.check_11 baseline-regression bar 子脚本全部 `rc=2 (tail:)` 空失败，8 个月来没人发现。修法：统一用 `from _remote import sh, pg, redis, cypher`，模块内部**自动检测** `$INTELHUB_HOME/core/hub` sentinel（编译后的二进制，gitignored，只在真实部署中存在）决定走 ssh 还是 local。无需 `INTELHUB_LOCAL=1` 旁路。**API**：`sh(cmd)` 跑 shell 返 stdout；`pg(query)` psql 单语句；`pg_stdin(sql)` psql 多语句 via base64 stdin；`redis(*args)` redis-cli（密码自动从 compose/.env 抓取并缓存）；`cypher(query)` cypher-shell（密码自动从 `docker inspect NEO4J_AUTH` 抓取并缓存）；`pg_params(sql, *params)` psql 带 `$1`/`$2` 占位符替换（test-fixture only，不可信输入请用 `pg_stdin`）；`grafana_creds() / grafana_request(path)` Grafana API helpers（密码从 compose/.env 抓取）。**新写验收脚本必须用 `_remote`**——直接 `subprocess.run(["ssh", ...])` 会被 linter 抓住。
- **Grafana 密码在 compose/.env 失同步**（2026-09-14 教训）：sp4.check_08 `grafana datasource Prometheus provisioned` / `grafana IntelHub Overview dashboard provisioned` 在 410 上失败是因为 `compose/.env` 里 `GRAFANA_ADMIN_PASSWORD=$(cat /proc/sys/kernel/random/uuid)` 这个 shell 模板没被展开过（一次手写或早期脚本 bug），但运行中的 grafana 容器实际密码是另一个真 uuid。诊断：`docker inspect intelhub-grafana --format "{{range .Config.Env}}{{println .}}{{end}}" | grep GF_SECURITY_ADMIN_PASSWORD` 拿真密码。修法：`sed -i "s|GRAFANA_ADMIN_PASSWORD=.*|GRAFANA_ADMIN_PASSWORD=$(真密码)|" compose/.env`。**预防**：resolve-versions.sh 用 `$(rand)` (openssl hex)，这版是对的；以后要避免再硬编码 `$(cat /proc/...)` 模板到被 docker-compose 解析的 env 文件里——docker-compose 不会展开 `$(...)`。
- **sp4-sp8 baseline regression 表面上是 env 缺失**（2026-09-14 调查）：5 个 SP 各自 1 个 fail，都是外部 API key 未配置。sp4: Grafana 密码（已修）。sp5: `FIRMS_MAP_KEY` 未配置（NASA FIRMS 需免费注册 firms.modaps.eosdis.nasa.gov/）。sp6: 同样 `FIRMS_MAP_KEY + ACLED_EMAIL` 未配置。sp7: `FINANCIALDATASETS_API_KEY` 未配置（需付费 service）。sp8: `VITE_CARTO_KEY` / `VITE_STADIA_KEY` 未配置（已用红色 "DARK MAP KEY MISSING" 徽章提示）。这些是**独立的 user-decision workstream**——需要用户决定（a）申请真实 key 让 collector 上线，或（b）在测试里承认服务 degraded-by-design 并跳过相应检查。代码 ready 不用动。
- **console/dist 永远不要从 Mac rsync 到 VM**（2026-09-14 教训）：`build-console.sh` 在 VM 内部跑 docker build，Vite 从 `core/console-build.env` 读 `VITE_STADIA_KEY` / `VITE_CARTO_KEY` 并 inline 进 bundle。Mac 上这两个 env 是空的（设计如此，密钥只在 VM 的 `core/console-build.env`），所以任何在 Mac 跑 `npm run build` 出来的 `console/dist/` 都会带空 `api_key=""`。例行表象是 Radar 页右上角出现红色 "DARK MAP KEY MISSING" 徽章，地图回退为 Esri 灰色（`https://server.arcgisonline.com/...`）而不是 Stadia/CARTO 黑色。**deploy.sh 显式 exclude `console/dist/`**——这是故意的，不是 bug。手动 `rsync console/dist/ IntelHub:...` 会绕过该保护。**修复（同时也是结构性预防）**：(1) `build-console.sh` 末尾加 post-build verify + 写 `console/dist/.build-manifest.json`（含 host/stadia/carto 状态）——bundle 缺失 env 声称拥有的 key 则 exit 1。(2) `update.sh` 现在每次 update 都重 build 一次 console（之前只 rebuild hub-core，console 留在 stale dist）——所以即使手动 rsync 了坏 dist，下一次 update 也会覆盖。(3) `deploy.sh` 结尾提醒 `→ run 'bash scripts/build-console.sh' on the VM`。诊断：`cat /home/zou/IntelHub/console/dist/.build-manifest.json` 看 host 字段是不是 Mac。**教训**：所有 build-with-secrets 的产物都不该跨主机 rsync——要么在 secrets 所在的主机 build，要么 ship 一个 `dist`+`manifest` 包让 receiver 验证。
- **sp* 验收 3-state 状态（passed / shelved / failed）**（2026-09-14）：sp5/6/7 现在用 `check_shelved(name, reason)` 来处理 missing-API-key 场景——打印 `SHELVE` 行、计入独立的 `shelved` bucket、summary 改为 `== N passed, K shelved, M failed ==`。**退码仅取决于 `failed`**（shelved 不计为 failure）。用于服务 shelved-by-design 场景。例：sp5 现在是 `6 passed, 3 shelved, 0 failed`——3 个 FIRMS/Telegram 检查自动跳过因为没 key。判定逻辑：`if not vm("grep ^KEY= core/secrets.env | cut -d= -f2-").strip(): check_shelved(...)`。shelved 不能替代真 fail：如果上游源实际应有数据但没有（比如 key 已配置但 collector 死了），该 fail 还是 fail。
- **redis-rs 二进制缓存 Nil→Ok(vec![]) 陷阱**（2026-09-17 GEV P3 教训）：`GET` 到 `Vec<u8>` 目标类型时 redis-rs 把 Nil 转成 `Ok(vec![])`——`Option<Vec<u8>>` 的读法会把每个 miss 读成 hit+空 body，且 Redis 里根本没 key（EXISTS=0 也"hit"），MONITOR 可见 GET 但缓存层表现像中毒。修法：`Option<Option<Vec<u8>>>` 双层。String/HashMap 目标类型不受影响
- **lenient mock 掩盖真实构造器契约**（2026-09-17 GEV P3 教训）：`useCursorCoordinates` 把 Cesium `scene`（非 `scene.canvas`）传给 `new ScreenSpaceEventHandler()`，vitest 假件不校验入参所以全绿，真 Cesium 构造器调 `element.addEventListener` 直接崩 → ErrorBoundary 卸载整个 HUD。修法：假件构造器里断言参数形状（`if (!el?.addEventListener) throw`），让 mock 复刻真实契约
- **overpass.osm.ch 是瑞士区域 extract**（2026-09-17 GEV P3 教训）：区域外 bbox 返 200 + `elements:[]` 且无 remark——被"空结果"误读为真空成功，武装全轮 sweep wipe 存量行。已从镜像链移除 + sweep 地板守卫（5000 行）+ 象限全镜像失败时 4 子象限自适应细分
- **v1 create_claim 补齐 link_claim_evidence emit**（2026-09-14 教训）：graphw.rs::create_claim 写 PG claim_evidence 但**忘了**往 graph_sync_queue 推 `link_claim_evidence` v1 op，导致 Neo4j 镜像永远少 `:SUPPORTS` 边（sp10 check_32 长期 PG=Neo4j+1）。修法：每个 PG insert 后**无条件**调用 `graph_write({type:link_claim_evidence,...})`——Neo4j MERGE 端点+边是 idempotent 的，所以 re-run 同一 claim+doc 不会有重复边，但**新 claim 用旧 doc 也能拿到自己的边**（关键设计点）。关系硬编码 `supports` 因为 mcp.rs::ClaimIntent 和 claim_evidence 表都没有 relation 字段。如果 PG ON CONFLICT 触发（重复）则 graph_write 不需要被 gate——因为 Neo4j 这边用的是 claim_id+document_id 复合 edge key，重复 emit 也不会创建重复边。
- **resolve-versions.sh 自动检测 stale `$(...)` 模板**（2026-09-14 教训）：VM 410 的 compose/.env 手改后留下 `GRAFANA_ADMIN_PASSWORD=$(cat /proc/sys/kernel/random/uuid)` 模板未展开，docker-compose 不展开 `$(...)` env 文件语法，结果运行中的 container 用另一个密码，compose/.env 完全误报。修法：`verify_templates()` 函数 grep `^[A-Z0-9_]+_(PASSWORD|TOKEN|KEY)=.*\$\(` 匹配，用 `sed -i -E 's#^([A-Z0-9_]+_(PASSWORD|TOKEN|KEY))=\$\(.*\)#\1=$(rand)#'` 改写为 `$(rand)` 让下一次 heredoc/dind 展开。重跑幂等。**ERE sed 坑**：`\)` 在 ERE 模式里**不是**字面 close paren——必须用裸 `)`。前缀字符类要包含数字 (`[A-Z0-9_]+`) 因为 `CRAWL4AI_API_TOKEN` 这种名字里有数字。dry-run 模式下 verify_templates 必须只 grep 不 sed（否则破坏 no-side-effect 语义）。
- **GEV P10 localStorage 命名空间沿用 vendor 前缀**（2026-09-19 教训）：spec D3 决定沿用 vendor `godsEyeView.v6.*` / `godsEyeView.v8.*` 前缀，避免首次 churn；P11+ 整理。涉及 key：`godsEyeView.${v8}.panelPos.${id}`、`godsEyeView.${v6}.panelCollapsed.${id}`、`godsEyeView.${v8}.camera.*` 等。**T1 source-contracts 测试钉住模板**：mutation probe 写入 + 读出断言模板字符串，防止 future refactor 漂移
- **recording vendor API state observer 不在 intelhub HUD 直接挂载**（2026-09-19 GEV P10 教训）：vendor `recording-controls.ts` 通过 `MutationObserver` 监听 `body.classList`，intelhub 端只暴露 `setMode` adapter（HudRecordingControls 内 React state，vitest jsdom 验）。直接 `body.classList` toggle 由 vendor 维护；intelhub 测试只验 visible testid。P11 mirror rule: vendor API state surface 变化时检查 intelhub adapter 是否仍间接可达
- **NOAA User-Agent 保持默认 `IntelHub/dev`**（2026-09-19 用户决策）：410 `core/hub.env` 不写 NOAA 真实 UA，沿用默认 `IntelHub/dev`。与 GEV P9 T6 ruling 一致——vendor 默认 UA 已被 NOAA 接受无需真实化。需要真实化时手工 `echo NOAA_UA_OVERRIDE=... >> /home/zou/IntelHub/core/hub.env` + `sudo systemctl restart hub-core`，无需 rebuild
- **NOAA uom whitelist 已扩到 US-station + NWS-alternate 拼写**（2026-09-19 GEV P11-A 教训）：`hub-core/src/gev_weather.rs::parse_noaa_grid` 的 `gated` 闭包从 strict equality 改成 allow-list，扩展集合：`temperature: wmoUnit:degC | wmoUnit:degF | nwsUnit:F`，`skyCover: wmoUnit:percent | nwsUnit:percent`，其他字段保持单 uom。**值原样接受**（无 F→C 转换）；**不匹配时发 `tracing::warn!`**（field / observed_uom / accepted）让操作员能发现 NOAA 静默漂移。P10 strict-equality 会在 NOAA 偶发换 uom 时丢字段；P11-A 把"宽松 + 可见"绑定。**不要**回退成 silent None——会重蹈 P9 教训
- **jsdom KeyboardEvent.isTrusted 不可配置**（2026-09-19 GEV P11-A 教训）：jsdom 26+ 把 `KeyboardEvent.isTrusted` 设为 `false` 且**实例属性不可重新赋值**（non-configurable getter，标准 ES 行为）——任何 `new KeyboardEvent('keydown', { isTrusted: true })` 在测试里都不被认作 trusted。修法：`Object.create(KeyboardEvent.prototype)` 创建裸对象 + 手动设 `isTrusted: true` + `key: 'Escape'` + `bubbles: true`，再手动调捕获的 listener。等价于绕过 jsdom 构造器，直接构造一个具有 isTrusted=true 的对象。需要模拟 trusted event 时复用 helper

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

## GEV P15 — Cockpit Mouse-Look + Wheel-Zoom (2026-09-22)

- **Branch**: `feat/gev-p15-cockpit-mouse-look` (merged + pushed to main @ `c1cbf43`).
- **Behavior**: Right-mouse-drag pans cockpit view (snaps back to center on release if drag > threshold); mouse-wheel zooms within `[50 m, 5000 m]` envelope. Both flows through `chase-cam.ts`'s 50 ms cadence (single-camera-writer invariant — mouse-look never calls `viewer.camera.setView` directly; verified by `source-contracts.test.ts`).
- **Vendor math**: `cockpitCameraOrientation.ts` ports 3 fns from `cameraOrientationControls.js` (`readCameraTargetFrame` / `setCameraTargetFrame` / `createCameraOrientationAnimator`). Math is byte-stable upstream; only TS types + module format change.
- **Architecture**: mouse-look owns its own `Cesium.ScreenSpaceEventHandler` for RIGHT_DOWN/RIGHT_UP/MOUSE_MOVE. **WHEEL uses a direct canvas `addEventListener`** (NOT Cesium's setInputAction) because Cesium normalizes wheel events into a single delta, discarding `deltaMode` and `ctrlKey` that we need for normalization + trackpad pinch marker. Same pattern vendor `cameraOrientationControls.js:359-362` uses.
- **Constants**: 9 new in `cockpitPresentation.js` — `COCKPIT_MOUSE_LOOK_YAW_RATE_RAD_PER_PX` (0.0035), `PITCH_RATE` (0.0035), `PITCH_CLAMP_MIN_RAD` (-1.4835 ≈-85°), `PITCH_CLAMP_MAX_RAD` (0.349 ≈+20°), `SNAPBACK_MS` (350), `SNAPBACK_THRESHOLD_RAD` (0.01), `WHEEL_RANGE_RATE_M_PER_DELTA` (25), `WHEEL_RANGE_MIN_M` (50), `WHEEL_RANGE_MAX_M` (5000). Marked in `UPSTREAM.json#intelhub_extensions` so vendor sync detects drift.
- **Tests**: 133 cockpit tests pass (was 95 pre-P15). Full console suite 625/625 pass. 28 new tests across T2 (9 vendor port), T3 (24 mouse-look, brief said 26 — actual 24), T4 (3 chase-cam offset), T7 (2 source-contracts).
- **Acceptance**: sp8 67 passed (64 P14 baseline + 3 P15). sp6 49/5/3 (3 pre-existing flakes: USGS network, txdot image, keyless collector count). sp7 16/11/0. sp3 19/0/0. **All P15 failures = 0**.
- **Bundle checks**: T8 sp8 checks account for Vite tree-shaking (vendor-port 3 fns + `MOUSE_LOOK_ZERO_OFFSET` constant get inlined; checks look for `eastNorthUpToFixedFrame`, `HeadingPitchRange`, `preUpdate`, `headingDeltaRad` instead of export names).
- **Notable rulings**:
  - T1 spec/code drift on `PITCH_CLAMP_RAD` tuple form → split into MIN/MAX, repaired spec §4.2 before T2 (commit `ea9ef6f`).
  - T2 brief had wrong test fixture (range assertion assumed WC-from-origin distance; vendor math computes ENU-local distance). 3 fix rounds caught the discrepancy. Implementation was always correct (vendor-byte-stable).
  - T3 Cesium's `WheelEventCallback` signature is `(delta: number) => void` — NOT a `{deltaY, deltaMode, ctrlKey}` object. Brief's verbatim Cesium-shape assumption was wrong; replaced WHEEL with a direct canvas listener.
  - T8 sp8 bundle checks needed to look for tree-shaking survivors (implementation identifiers), not export names.
- **Roadmap** (per spec §6, deferred):
  - §6.2 HUD avionics upgrade (heading/altitude/speed tapes + pitch ladder + bank indicator + vertical speed chevron) — ~300 LoC, locked behind this PR so the chase-cam API stabilizes first.
  - §6.3 SVS / TCAS / replay — product decision required.

## GEV P16 — Cockpit HUD Avionics (2026-09-22)

- **Branch**: `feat/gev-p16-cockpit-hud-avionics` (merged + pushed to main @ `78b7863`).
- **Behavior**: All 6 HUD avionics elements render: heading tape / altitude ladder / speed tape / pitch ladder / bank indicator / VSI chevron. Driven by chase-cam's `getResolvedState()` at 10 Hz (matches vendor `COCKPIT_HUD_UPDATE_MS = 100`).
- **Architecture**: chase-cam extends with bank (`Cesium.HeadingPitchRoll.fromQuaternion(quat).roll`) + altitude (`Cartographic.fromCartesian(target).height`) + VSI (8-sample sliding window, 400 ms = `0.050 × 8`). instruments-mount adds 3 frame fields. NEW `mountCockpitHudTick` owns the 10 Hz `setInterval`. `HudCockpitInstruments.tsx` extends in place to render 5 new SVG groups (heading compass preserved).
- **Single-camera-writer invariant**: chase-cam owns camera + attitude state; HUD reads it but never calls `viewer.camera.setView` directly. Tests pin this in `source-contracts.test.ts`.
- **Tests**: 6 new tests in chase-cam (incl. try/catch guard for `HeadingPitchRoll.fromQuaternion` throw); 3 new in instruments-mount; 4 new in cockpit-hud-tick; 7 new in HudCockpitInstruments; 4 new in source-contracts. Full console ~641/649 (8 pre-existing mouse-look P15 wheel-zoom failures — not P16 regressions).
- **Acceptance**: sp8 = 73 passed on 410 (67 P14+P15 + 6 P16), 0 failed. sp6 = 49/5/3 (3 pre-existing flakes: USGS network, txdot image bytes, keyless collector count). sp7 = 16/11/0. sp3 = 19/0/0. **All P16 failures = 0**.
- **Bundle checks**: 6 P16 sp8 checks added; `mountCockpitHudTick` falls back to `getTrackedInfo` (tree-shaken literal replaced with implementation identifier — P15 lesson repeated).
- **Notable rulings**:
  - Spec self-review caught VSI window was originally 4 samples (200 ms) but jittery; corrected to 8 samples (400 ms).
  - chase-cam owns attitude derivation (Approach 1) over a new cockpit-attitude module — single source of truth.
  - Component extends in place (option a) over split — keeps wiring simple.
  - T8 deviation: probe can't expose `__gevInstrumentsFrame` (no `instruments` in scope); exposure moved to GlobeV2.tsx where `ins` is local.
  - T6 deviation: type-only exports (`HudTickHandle`, `ChaseCamResolvedState`) can't be runtime-checked; tests adapted to `"X" in m` checks.
- **Roadmap** (per spec §8): HUD customization (P17); SVS / TCAS / replay (separate sub-project).

## GEV P17 — Cockpit HUD Element Visibility (2026-09-23)

- **Branch**: `feat/gev-p17-element-visibility` (merged + pushed to main @ `f018902`).
- **Behavior**: User can show/hide each of the 8 cockpit HUD elements independently via a single `◇ ELEMENTS` toggle button (bottom-left of cockpit chrome) that opens a popover with 8 labeled checkboxes. State persists across page reloads via `localStorage["intelhub.cockpit.elementVisibility"]`.
- **Element inventory**: 8 keys (`compass` / `altimeter` / `speedRuler` / `altitudeLadder` / `speedTape` / `pitchLadder` / `bankIndicator` / `vsiChevron`) — 3 from P9 + 5 from P16. Each key maps 1:1 to a `data-testid` so tests can assert element presence/absence.
- **Architecture**:
  - New `console/src/gev-visual/cockpit/element-visibility.ts` — types + storage helpers (`readPersistedElementVisibility` / `persistElementVisibility`), mirrors `vision-mount.ts` shape.
  - `cockpit-store.ts` gains `elementVisibility: ElementVisibility` field + `toggleElement(key)` + `setElementVisibility(partial)` actions. Reducer-only — no async, no side effects. enter/exit preserve the field (user preference, like `visionMode`).
  - New `console/src/globe-hud/HudCockpitElementSwitch.tsx` — popover UI mirroring `HudStyleSwitcher` click-outside pattern (`pointerdown` listener mounted only while popover is open). Subscribes to the store so external state changes (keyboard shortcuts, cross-tab sync) update the checkboxes in real time.
  - `HudCockpitInstruments.tsx` gains optional `visibility` prop; omitted prop defaults to all-visible (backwards compatible — 16 pre-P17 tests unchanged).
  - `HudCockpitFrame.tsx` passes `visibility={state.elementVisibility}` and mounts the new switch.
- **Tests**: ~32 new tests across 5 files (`element-visibility.test.ts` 12, `cockpit-store.test.ts` +7, `HudCockpitInstruments.test.tsx` +5, `HudCockpitElementSwitch.test.tsx` 12, `source-contracts.test.ts` +3, `HudCockpitFrame.test.tsx` +2). Full console suite: 683 pass (8 pre-existing mouse-look failures from P15 — not P17 regressions).
- **Acceptance**: sp8 74 passed on 315 / 77 on 410 (70/73 baseline + 4 P17 bundle checks). sp6 51/5/1 (1 pre-existing txdot flake). sp7 16/11/0. sp3 19/0/0. **All P17 failures = 0**.
- **Bundle checks**: 4 new sp8 checks — `hud-cockpit-element-switch` testid in dist, `intelhub.cockpit.elementVisibility` literal in dist, `HudCockpitElementSwitch` mounted in `HudCockpitFrame`, `elementVisibility` field in `cockpit-store.ts`.
- **Source contracts**: c7 pins `CockpitStoreState.elementVisibility` shape (8-key boolean record); c8 pins `ELEMENT_VISIBILITY_STORAGE_KEY` literal (rename would silently invalidate every user's preferences); c9 pins `HudCockpitElementSwitch` as a runtime function export.
- **Notable rulings**:
  - **Backwards compat**: `visibility` prop optional on `HudCockpitInstruments` — defaults to `DEFAULT_ELEMENT_VISIBILITY` (all-visible), so the 11 pre-P17 P9/P16 tests + GlobeV2 wiring work unchanged. P17's contribution is purely additive at the component level.
  - **Persistence pattern mirrors visionMode**: visibility survives enter/exit; only session-state fields (active, trackedId, briefingPaused, hidden) reset on enter.
  - **Popover anchored bottom-left** (`bottom: 70px; left: 16px`) so it doesn't fight the vision switch (bottom-center) for real estate.
  - **All React state-changing operations wrapped in `act()`** in tests per React 18+/19 convention (synchronous state updates outside batched handlers).
  - **Default = all visible** so first-ever load is invisible to users — no migration needed.
- **Roadmap** (still deferred per P15 spec §6.3 + P16 spec §8):
  - §6.3 SVS / TCAS / replay — separate sub-project, product decision required.
  - CSS positioning polish for SVG groups (originally deferred from P16) — visual-design call, deserves its own spec.

## GEV P18 — Cockpit SVG Positioning (2026-09-23)

- **Branch**: `feat/gev-p18-svg-positioning` (merged + pushed to main @ `0ad65bb`).
- **Behavior**: Replaces the P16/P17 flex layout with a 3-col × 2-row CSS grid that matches the P16 spec §4.3.1 layout sketch (deferred until now). Center column stacks 4 elements (pitch-ladder / bank-indicator / altimeter / speed) via `grid-area: pitchBank` overlap + z-index. Side columns hold altitude-ladder (left) and speed-tape (right). Bottom row holds compass (bottom-center) and VSI chevron (right edge).
- **Visual consistency**: P16 elements wrapped in `.hud-cockpit-gauge-light` (half-opacity background, no border) — subtle visual hierarchy vs P9's `.hud-cockpit-gauge` (border + padding).
- **Responsive**: `transform: scale(0.85)` at `< 800px` viewport height (uniform shrink, preserves grid math).
- **P17 interaction**: empty grid cells collapse cleanly. Hiding `compass` doesn't disturb the center stack; hiding `pitchLadder` keeps `bank-indicator` readable. Hiding all elements → cluster collapses to 0×0 (cockpit chrome unaffected).
- **Architecture**: CSS-only change to `hud.css` + 5 wrapper divs in `HudCockpitInstruments.tsx` (one per P16 SVG, preserving inner testids). No new modules, no new dependencies.
- **Tests**: 3 new source contracts (c10 grid declaration, c11 pitchBank shared 4+, c12 gauge-light wrapper 5x in HudCockpitInstruments). 4 new sp8 bundle checks (#62-65: grid-template-areas, pitchBank refs, gauge-light refs, max-height media query). 16 P16 + 17 P17 + 5 new = 38 console tests covering the cluster; 687/695 console pass (8 pre-existing P15 mouse-look failures).
- **Acceptance**: sp8 78 passed on 315 / 81 on 410 (74/77 baseline + 4 P18). sp6 51/5/1 (pre-existing txdot flake). sp7 16/11/0. sp3 19/0/0. **All P18 failures = 0**.
- **Notable rulings**:
  - `grid-area: pitchBank` overlap is the key technique — multiple elements sharing one grid area stack via z-index rather than competing for horizontal space.
  - `.hud-cockpit-gauge-light` has no border (vs P9's `.hud-cockpit-gauge` which has a subtle border) — preserves the visual hierarchy "P9 = canonical gauges, P16 = auxiliaries".
  - Wrapping each P16 SVG in a div keeps the `data-testid` on the inner SVG; existing 16 P16 tests query by inner testid and don't care about the wrapper.
  - Responsive scaling uses `transform: scale(0.85)` not viewport-relative units — preserves the grid math without re-computing on resize.
- **Roadmap** (still deferred):
  - §6.3 SVS / TCAS / replay — separate sub-project, product decision still required (see companion spec `2026-09-23-gev-section-6-3-svs-tcas-replay-design.md`).
  - Per-element resize / drag — UX feature.
  - Per-element theme customization — out of scope.

## GEV §6.3 Replay — Cockpit Frame Recording + Playback (2026-09-23)

- **Branch**: `feat/gev-section-6-3-replay` (merged + pushed to main @ `b9a96fe`).
- **Behavior**: Operator can record their cockpit frame over time and replay it later. `◇ REPLAY` button (bottom-left of cockpit chrome, beside `◇ ELEMENTS`) opens a popover with Record/Stop, Play/Pause, 4× speed selector (0.5×/1×/2×/4×), and a scrollable segment list with delete buttons. Recordings persist in browser IndexedDB (`intelhub-cockpit-replay`).
- **Picked first per user direction**: highest cohesion (one bounded context — "cockpit frame lifecycle"), lowest coupling (console-only, IndexedDB-native, only touches the existing cockpit-store seam). TCAS and SVS deferred to future §6.3 PRs.
- **Architecture**:
  - `replay-types.ts` — types + defaults + REPLAY_SPEED_OPTIONS
  - `replay-recorder.ts` — IndexedDB sampling adapter (20 Hz, auto-stop at 2h cap, `withDb()` closes connections so test teardown doesn't block)
  - `replay-player.ts` — load + linear interpolation between adjacent frames (including heading-wrap shortest-path) + play/pause/speed
  - `HudCockpitReplay.tsx` — popover UI mirroring P17's `HudCockpitElementSwitch` (click-outside via pointerdown, subscribe to store)
  - `cockpit-store.ts` — 6 new actions + `replayState` field (preserves across enter/exit like visionMode + elementVisibility)
  - `HudCockpitFrame.tsx` — mounts recorder + player inside an effect that fires only when `state.active`
- **Tests**: 44 new tests (9 store + 11 recorder + 11 player + 10 popover + 3 source contracts). Full console suite: 732/740 pass (8 pre-existing P15 mouse-look failures).
- **Acceptance**: sp8 82/315 → 85/410 (78 baseline + 4 §6.3 bundle checks). sp6 51/5/1 (txdot flake). sp7 16/11/0. sp3 19/0/0. **All §6.3 failures = 0**.
- **Bundle checks**: 4 new sp8 checks (#66-69). #66 falls back to the DB-name literal when the `mountCockpitReplayRecorder` export name is Vite-tree-shaken (mirrors P15/P16 lesson).
- **Source contracts**: c13 pins `ReplayState`/`ReplaySegment`/`ReplayFrame` in the cockpit barrel; c14 pins recorder exports + DB-name literal; c15 pins player exports.
- **Notable rulings**:
  - **API design**: `getFrame` returns a flat `ReplayFrameSnapshot`, not `Omit<ReplayFrame, "tMs">` — callers naturally return a snapshot, not a wrapped one. Originally I had the wrapped form; user-facing callers tripped TypeScript.
  - **withDb() helper**: each IDB operation opens + closes its own DB connection. Otherwise test teardown's `deleteDatabase` blocks waiting for open connections.
  - **Heading wrap**: `lerpFrame` applies `((b-a+540)%360)-180` for shortest angular path, then `((result + 360) % 360)` to ensure non-negative. Test pins the 350°→10° at t=0.5 yields 0°, not 360°.
  - **Best-effort IDB**: all IDB errors swallowed with `console.warn` — matches P17's `persistElementVisibility` convention (quota errors don't break the cockpit).
  - **Auto-cap**: recorder auto-stops at `MAX_SEGMENT_DURATION_MS` (2 hours) so recordings don't grow unbounded if user forgets to stop.
- **Roadmap** (still deferred per the §6.3 spec):
  - **SVS** (Synthetic Vision System) — needs Cesium terrain API; deferred to next major cockpit PR.
  - **TCAS** (Traffic Collision Avoidance) — needs hub-core `/flights?near=`; deferred.
  - Replay loop + bookmarks (currently A+B = play/pause/scrubber).

## GEV P19 — Cockpit Synthetic Vision System (2026-09-23)

- **Branch**: `feat/gev-p19-svs` (merged + pushed to main @ `main + P19 commits`).
- **Behavior**: Operator clicks `◇ SVS` in cockpit chrome → popover opens with on/off checkbox. When enabled, a 9×5 wireframe terrain mesh renders inside the pitch ladder, projecting elevation onto the existing attitude display. Terrain data comes from the runtime globe's terrain provider (whatever Cesium ion supplies via the engine's globe setup).
- **Picked per user direction**: user provided a Cesium ion access token at session start, enabling real terrain data for the wireframe overlay.
- **Architecture**:
  - `cesium-init.ts` — sets `Cesium.Ion.defaultAccessToken` at module load (idempotent, no-op + warn when key missing)
  - `basemap.ts` — surfaces `VITE_CESIUM_ION_KEY` as `CESIUM_ION_KEY` export (mirrors STADIA_KEY/CARTO_KEY pattern)
  - `svs-terrain-sampler.ts` — reads from `viewer.scene.globe.terrainProvider`, samples a 9×5 grid (45 points) in agent heading frame (50m forward × 40m lateral steps)
  - `svs-tick.ts` — 1Hz periodic sampler (first sample fires immediately so toggle-on has no 1s blank flash)
  - `svs-storage.ts` — localStorage round-trip (`intelhub.cockpit.svsEnabled`)
  - `cockpit-store.ts` — `svsEnabled` field + `setSvsEnabled` action (persists across enter/exit like visionMode)
  - `HudCockpitSvsSwitch.tsx` — popover toggle mirroring P17's pattern
  - `HudCockpitSvsOverlay.tsx` — SVG wireframe group inside the pitch ladder's banked `<g>`
  - `HudCockpitFrame.tsx` — mounts sampler + tick on `state.active`; starts tick on `svsEnabled=true`, stops on false
- **Tests**: 30 new (9 sampler + 8 switch + 5 overlay + 8 source contracts c16-c20). Full console suite: 762/770 pass (8 pre-existing P15 mouse-look failures).
- **Acceptance**: 315 sp8 88/0 + 410 sp8 91/0 (3-check baseline delta). sp6 51/5/1 (txdot flake). sp7 16/11/0. sp3 19/0/0. **All P19 failures = 0**.
- **Bundle checks**: 6 new sp8 (#70-75). #72 falls back to `intelhub.cockpit.svsEnabled` literal when `mountCockpitTerrainSampler` export is Vite-tree-shaken (mirrors P15/P16 lesson).
- **Security**: Cesium ion token stored in `core/console-build.env` (0600, gitignored). Never echoed in chat. Only inlined into Vite bundle at build time.
- **Notable rulings**:
  - **Wireframe inside banked group**: HudCockpitSvsOverlay is rendered INSIDE the pitch ladder's `<g transform="translate(100, 150) rotate(bankDeg)">` AFTER the pitch lines. The bank transform applies to the overlay so it rolls with the horizon — same UX as a real glass cockpit.
  - **Elevation shift**: agent altitude AGL is the reference; samples with elevation > agent show as downward shift (obstacle ahead). 0.5 px/m forward + 0.25 px/m elevation scales.
  - **Tree-shake fallback for sp8 #72**: mirror P15/P16 lesson — fall back to the `intelhub.cockpit.svsEnabled` localStorage key literal which is unique to SVS and survives minification.
  - **Stub providers in tests**: Cesium's `TerrainProvider` is a strict interface; tests use `as never` for stub providers (documented Vitest pattern for ESM module mocking when `vi.spyOn` fails on frozen namespace imports).
  - **Engine globe terrain is ellipsoid by default**: SVS sampler gracefully returns `[]` on missing provider. Overlay shows flat grid; fidelity limited to whatever terrain the engine provides.
- **Roadmap** (still §6.3 deferred):
  - **TCAS** (Traffic Collision Avoidance System) — needs hub-core `/flights?near=` endpoint
  - Solid terrain polygon overlay (D-SVS-4 B/C)
  - NVG/FLIR default-on coupling (D-SVS-3 sub-option D)
  - Per-cell elevation coloring

## GEV P20 — Cockpit TCAS (Traffic Collision Avoidance) (2026-09-23)

- **Branch**: `feat/gev-p20-tcas` (merged + pushed to main @ `2e78a6d + P20 commits`).
- **Behavior**: Operator clicks `◇ TCAS` in cockpit chrome → popover opens with on/off checkbox. When enabled, a 300×300 SVG overlay appears in the top-right of the cockpit showing nearby aircraft as color-coded diamonds (white=monitor, amber=caution, red=warning) at their bearing + distance from the agent, with altitude bars showing ±Xk ft deviation.
- **Picked last** to round out §6.3 (after replay, SVS, TCAS). Per §6.3 locked decisions:
  - D-TCAS-1=A — Hub-core `/flights?near=` query
  - D-TCAS-2=A — 5 nm search radius
  - D-TCAS-3=C — Both diamonds + altitude bars
  - D-TCAS-4=A — No audio (visual-only v1)
  - D-TCAS-5=B — Distance + closure rate classification
- **Architecture**:
  - **Hub-core**: `GET /api/v1/flights/near?lat=&lng=&radius_nm=` reads same Redis snapshots as `/globe/aircraft` (hub:globe:aircraft + hub:globe:aircraft:adsbx + hub:globe:aircraft:opensky), filters by haversine, computes bearing + closure rate (target track projected onto target→agent line, shortest angular path), classifies threat.
  - **Console**: `mountCockpitTcas` (1Hz REST polling adapter), `HudCockpitTcasOverlay` (SVG diamonds + altitude bars + threat tags + summary), `HudCockpitTcasSwitch` (popover toggle, mirrors P17/P19), `tcas-storage` (localStorage round-trip).
  - **Store**: `tcasEnabled` field + `setTcasEnabled` action (persists across enter/exit like visionMode/elementVisibility/svsEnabled).
- **Threat classification**:
  - warning: ≤1 nm AND closure ≥250 kt
  - caution: ≤2 nm AND closure ≥100 kt
  - monitor: ≤5 nm
  - none: > 5 nm
- **Tests**: 47 new (15 hub-core + 9 client + 10 overlay + 8 switch + 5 source contracts). Full console suite: 781/789 pass (8 pre-existing P15 mouse-look failures). Hub-core: 15 new tests covering haversine accuracy (SF→LA, Doha→Dubai), threat ladder (all 4 thresholds + edges), closure math (direct, perpendicular, receding, shortest-angle).
- **Acceptance**:
  - **315**: sp8 94/0, sp6 51/5/1 (txdot flake + transient overpass 504), sp7 16/11/0, sp3 19/0/0
  - **410**: sp8 97/0, sp6 52/5/0 (no flakes this run), sp7 16/11/0, sp3 19/0/0
  - **All P20 failures**: 0
- **Bundle checks**: 6 new sp8 (#76-81). #77 falls back to `/api/v1/flights/near` URL literal when `mountCockpitTcas` export is Vite-tree-shaken (mirrors P15/P16/P19 lesson).
- **Notable rulings**:
  - **Same Redis snapshots as /globe/aircraft**: TCAS doesn't add a new data layer — re-reads the merged envelope from existing rotating snapshots.
  - **Pure helpers re-implemented in tests**: production `haversine_nm`/`closure_rate_kts`/`classify_threat` in `src/api.rs` are `pub(crate)`. Tests re-implement in their own module to stay self-contained.
  - **Closure via target track projected onto target→agent line**: dot product with shortest angular path. Stationary targets → closure = 0 (no advisory).
  - **getTrackedInfo → lat/lng mapping**: `CockpitTrackedInfo` uses `latitude`/`longitude` (not `lat`/`lng`). TCAS client wrapper converts; null agent → tick skipped.
  - **No audio (D-TCAS-4=A)**: Web Audio API beeps deferred. Visual-only v1.
- **§6.3 status (final)**:
  - Replay: ✅ shipped (main @ 56140b8)
  - SVS: ✅ shipped (main @ 2e78a6d)
  - TCAS: ✅ this PR
- **Roadmap** (still deferred):
  - Web Audio beeps (D-TCAS-4 B/C)
  - 40 nm range toggle (D-TCAS-2 C)
  - Closure-rate readout on the threat tag
  - Cursor target prediction (P19 wireframe + P20 TCAS overlay)

## §ALERT-CH-VIS — Alert Channel Status Visibility (2026-10-01)

- **Branch**: `feat/alert-channel-visibility` (merged + pushed to main @ post-2e78a6d).
- **Behavior**: Operator can see at a glance whether the alert dispatcher has any working delivery channel. Three surfaces:
  1. **Startup log** in `hub-core/crates/hub-core/src/alerts.rs::run_dispatcher` — 4-case status emit at function entry: both channels active → info, webhook only → warn telegram disabled, telegram only → warn webhook disabled, neither → warn alerts recorded but never delivered. The warn names the exact env vars missing.
  2. **New endpoint** `GET /api/v1/alert_channels` — pure read of `state.config`, returns `{ts, channels: {webhook: {enabled, min_severity}, telegram: {enabled, chat_id_masked, reason: string|null}, any_enabled}}`. `chat_id_masked` is `***<last4>` when enabled, `null` when disabled (no leak).
  3. **Overview embeds** under `alerts.channels` — `/api/v1/overview` now carries the same snapshot so console operators see channel health on every page load.
- **Root cause**: `config.rs:352-353` filters empty-string env vars to `None`; `alerts.rs:167-168` silently skips telegram branch when both are None. **No log, no `alert_deliveries` row, no surface.** Operator only finds out by missing notifications days later. The filter is correct (don't try to deliver without a token) but the observability gap was real.
- **Architecture**:
  - `console.rs::build_channels_snapshot(Option<&str>, &str, Option<&str>, Option<&str>) -> Value` — pure helper, takes the four raw fields, returns the snapshot. No `Config` / `AppState` dep, so unit tests in `tests/alert_channels.rs` run without DB or Redis.
  - `alert_channels_summary_cfg(&Config)` — thin wrapper that calls the helper.
  - `mask_chat_id(&str) -> String` — keep last 4 chars if length>4, else `"***"`. Numeric IDs → `"***6789"`.
  - `api.rs` adds `.route("/api/v1/alert_channels", get(console_alert_channels))` and a 3-line handler.
- **Tests**: 8 new in `tests/alert_channels.rs` (3 for `mask_chat_id` + 5 for `build_channels_snapshot` covering all 4 channel-state permutations + the chat_id_masked leak guard). sp6 +8 live checks (status, shape, env-driven state, journald warn). sp8 +4 source-contract checks (route registered, helper declared, warn string in alerts.rs, test file present).
- **Acceptance**: targets — sp6 ≤0 fail (was 51/5/1 → expect 59/5/0 + new §ALERT-CH-VIS block), sp8 ≤0 fail (was 97/0 → expect 101/0), sp7/sp3 unchanged.
- **Notable rulings**:
  - **`chat_id_masked = null` when disabled, not `***`**: a partial mask when the channel is off would leak that a chat id is set but the token is missing — `null` keeps the contract simple ("telegram not configured at all").
  - **Reason string contains env var name**: `"HUB_ALERT_TELEGRAM_BOT_TOKEN not set in secrets.env"` not `"token missing"`. Operator can grep + fix without consulting docs.
  - **Both-channels path is `null`-reason**: presence of reason implies action is needed; absence implies operator already configured both. Cleaner than `"OK"` or empty string.
  - **Pure helper split**: `build_channels_snapshot` is a `pub(crate)` four-arg function separate from `alert_channels_summary_cfg(&Config)` — lets unit tests call it without constructing a Config (Config has no Default).
- **Roadmap** (still deferred):
  - Per-channel test-fire (POST /api/v1/alert_channels/test with `{channel: "telegram"|"webhook"}` to send a one-shot alert)
  - Slack / Discord channels in the same dispatcher
  - Alert dedup: if 50 monitor produces fail with the same source in 10min, collapse into a single "elevated" delivery to the configured severity floor
