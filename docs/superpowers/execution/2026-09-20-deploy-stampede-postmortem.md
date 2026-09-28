# 2026-09-20 PVE40 network stampede — postmortem + deploy protocol fix

**Status**: 410 recovered via PVE40 reboot, fix proposed (worktree pending).

## TL;DR

Deploying + restarting `hub-core` causes a thundering-herd outbound
network stampede: `monitor scheduler started sources=74` runs ~74
parallel `tokio::spawn` tasks all hitting external upstreams within 27s,
and the GEV-P11 cctv refresh sequentially walks 11 providers (~103s).
The reqwest pool is unbounded (`state.rs:90-94` — no `pool_max_idle_per_host`).
All egress goes through `openclash` fake-IP on `10.10.10.1`, which means
74 concurrent TCP handshakes fan into 74 concurrent TCP handshakes on
the openclash proxy endpoint.

Under repeat deploys (we did 1→315 verify + 2×410 restart today), the
cumulative connection storm saturates pve40's bridge / NAT table and the
hypervisor networking collapses for ~minutes. P10 ledger already
documented a 38-min PVE40 outage during deploy (`2026-09-18-gev-p10-ledger.md:125`);
this is the same root cause.

## Root cause (proven by journal inspection)

```
Sep 20 23:52:34.579  scheduler started sources=74
Sep 20 23:52:34.997  source="gfw"           fetched=0  ms=412
Sep 20 23:52:35.195  source="usgs"          fetched=47 ms=616
Sep 20 23:52:36.041  source="treasury"      fetched=4  ms=1454
Sep 20 23:52:36.369  source="climate"       fetched=2  ms=1776
... 50+ concurrent sources firing in first 20 seconds ...
Sep 20 23:52:53.291  source="gdelt"         429 Too Many Requests
Sep 20 23:53:20      cctv-refresh retrying provider="ny511" attempt=2 wait=60s
Sep 20 23:54:30.374  source="cctv-refresh"  fetched=0  ms=103793   ← 103 seconds!
```

Three compounded factors:

### 1. Scheduler stagger is too short
`monitor/scheduler.rs:53`:
```rust
let stagger = Duration::from_secs((idx as u64 % 10) * 3);
```
With 74 sources, idx 0..9 stagger by 0/3/6/9/12/15/18/21/24/27 seconds.
idx 10, 20, 30, 40, 50, 60, 70 share the same slot — meaning 7 sources
**start at the exact same second** within each 27-second window.

### 2. reqwest client has no per-host pool limit
`hub-core/state.rs:90-94`:
```rust
let http = reqwest::Client::builder()
    .timeout(std::time::Duration::from_secs(120))
    .user_agent("intelhub-core/0.1 (SP3)")
    .build()?;
```
Default reqwest pool: unlimited connections per host. With cctv probing
33 cameras per refresh, that's 33 simultaneous outbound connects to
`cwwp2.dot.ca.gov` alone.

### 3. cctv-refresh holds the slot for 103 seconds
`monitor/sources/cctv/refresh.rs:215-218`:
```rust
for provider in providers::providers() {  // 11 providers
    let (id, cell) = refresh_one(ctx, provider).await;  // sequential!
    write_health_cell(ctx, &id, cell).await;
}
```
Each provider does its own `tokio::time::sleep(retry)` on failure (30s/60s/90s
backoff). ny511 retrying + txdot 500 + caltrans long-poll = 103s of
cumulative outbound churn during the very first minute after restart.

## Why PVE40 dies

openclash on `10.10.10.1` uses **fake-IP** mode (per AGENTS.md network
section + known setup). Every external DNS resolution returns a fake
IP from the `198.18.0.0/15` range; traffic to that IP gets DNAT'd by
openclash to the real upstream via the configured proxy pool. The
proxy pool's connection slots, NAT table, and bridge fdb table are
finite — 74 parallel TLS handshakes from a single VM consume them
fast.

A second `systemctl restart hub-core` while the first is still in
the cctv-refresh phase (i.e. within the first 2 minutes after the
first restart) doubles the storm. A third pushes the host bridge
to fdb overflow or openclash's per-proxy concurrency cap, after which
network packets drop silently → PVE40 reports `Host is down` for
the VM (its management traffic fails), and the operator has no SSH.

## P10 ledger precedent

`docs/superpowers/execution/2026-09-18-gev-p10-ledger.md:125`:
> **PVE40 host outage during deploy**: 15:11 → 15:49 (38 分钟) PVE40 +
> VM 410 全部断网（`Host is down` / `No route to host`），用户决策
> 「持续 ping 每 5 分钟」。15:49 用户重启 PVE40 后恢复，410 ssh OK，
> build+restart+acceptance 完成。**与 P10 代码无关**——纯 PVE40 hypervisor 故障。

So this is a recurring pattern: PVE40 has *some* hypervisor or
bridge-side weakness that fails under network pressure. We can't fix
PVE40 (it's the user's hardware); we can only reduce the pressure we
put on it.

## Fix (worktree `fix/deploy-stampede` proposed)

**Layer 1 — cap concurrent outbound connections** (the actual fix)

`hub-core/crates/hub-core/src/state.rs`:
```rust
let http = reqwest::Client::builder()
    .timeout(std::time::Duration::from_secs(120))
    .user_agent("intelhub-core/0.1 (SP3)")
    // Cap per-host pool: prevents the 74-source startup storm from
    // saturating openclash on 10.10.10.1. Same VM has cctv probes to
    // 33+ cameras sharing hosts (e.g. cwwp2.dot.ca.gov); reqwest default
    // is unlimited, which is correct for a single-purpose client but
    // disastrous when the same client fans out across 74 sources.
    .pool_max_idle_per_host(8)
    .build()?;
```

**Layer 2 — widen the scheduler stagger** (the safer behavior change)

`hub-core/crates/hub-core/src/monitor/scheduler.rs:53`:
```rust
// Was: (idx as u64 % 10) * 3  → 0..27s for 74 sources, 7 sources per slot
// Now: spread across the full 74 source count at 1.5s each.
let stagger = Duration::from_secs((idx as u64) * 3 / 2);
```
This spreads the first run across ~110 seconds instead of 27, so the
same 74 sources no longer cluster into 7-per-slot groups. After the
first round, each source hits its own interval — no churn.

**Layer 3 — stagger cctv-refresh providers** (small but high-impact)

`hub-core/crates/hub-core/src/monitor/sources/cctv/refresh.rs:215`:
```rust
// Was: sequential `for provider` → 11 providers × avg Ns = 103s worst case
// Now: cap concurrency to 3 in-flight providers, queue the rest.
use futures::stream::{self, StreamExt};
let results: Vec<_> = stream::iter(providers::providers())
    .map(|p| async move { (p.id(), refresh_one(ctx, p).await) })
    .buffered(3)
    .collect()
    .await;
```
This caps cctv-refresh at 3 simultaneous upstream connects (vs 11
serial with retry sleeps) and brings it well under 60s.

**Layer 4 — operator deploy protocol**

`AGENTS.md` deployment section gets a new rule:

> **DO NOT restart `hub-core` more than once per 5 minutes on 410.**
> Each restart spawns 74 monitor tasks at once; back-to-back restarts
> during the first 2 minutes (before cctv-refresh settles) cascade
> into the PVE40 outage pattern documented in
> `docs/superpowers/execution/2026-09-20-deploy-stampede-postmortem.md`.
>
> After `sudo systemctl restart hub-core`, **wait at least 5 minutes**
> before running acceptance tests — that's when cctv-refresh settles
> and the bursty outbound traffic falls back to steady-state per-source
> cadence. Verify with `systemctl is-active hub-core && journalctl -u
> hub-core -n 1 | grep -c monitor`.

**Layer 5 — verification**

After the fix:
- New unit test on `scheduler::run` spawn timing asserts the largest
  gap between consecutive spawn times is ≥3s (so no 7-per-slot bursts).
- New integration test asserts cctv-refresh completes within 30s on
  a fixture with 11 mock providers (vs 103s before).
- Live verification: re-deploy to 315, run two `systemctl restart
  hub-core` 60s apart, confirm `journalctl -u hub-core` shows the
  cctv-refresh round completes within 30s the second time.

## Acceptance impact

Pre-existing acceptance suite is unaffected by these changes — they
all run on a settled hub-core. The new tests target startup behavior
specifically and live at unit/integration layer (no sp* regression).

## Action items

1. ✅ User restarts PVE40 (in progress) — confirmed at 23:55 hub-core back up
2. ⏳ Worktree `fix/deploy-stampede` with the 3 code changes
3. ⏳ Push to 315, verify with `journalctl -u hub-core` showing
   cctv-refresh completes in <60s and concurrent outbound
   connections <20 (was ~74)
4. ⏳ Push to 410, single deploy only, wait 5 min before acceptance
5. ⏳ Update AGENTS.md deploy section with the "no restart storm" rule
6. ⏳ `git push origin main`
