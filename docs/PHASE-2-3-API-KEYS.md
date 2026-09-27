# Phase 2 + Phase 3 API Keys — Configuration Guide

> Status: **2026-09-27**. Covers 7 env-gated OSINT collectors shipping in
> Phase 2.2 / 2.3 / 2.5 (3 sources) + Phase 3.3 / 3.4 / 3.5 (3 sources).
> Phase 1 sources are keyless; Phase 2.1 (GreyNoise) / 2.4 (GBIF) /
> Phase 3.1 (ArcNautical free verdict) / Phase 3.2 (Strait of Hormuz)
> are also keyless.

## TL;DR

Most IntelHub OSINT collectors are **keyless**. Of 18 Phase 1-3 sources
shipped so far, **11 are keyless** and run as soon as hub-core comes
up. **7 are env-gated** — they don't fail without a key, they show
"shelved-by-design" in sp6 acceptance until you provide one.

| Phase | Source | Env-var(s) | Status without key | Free? | When you want it |
|---|---|---|---|---|---|
| 1.3 | open_aq | `HUB_OPENAQ_API_KEY` | SHELVE | ✅ free | ✅ **already configured** on 315 + 410 |
| 2.2 | currents | `HUB_CURRENTS_API_KEY` | SHELVE | ✅ free 250 req/day | **NOT configured** — needs signup |
| 2.3 | semantic_scholar | `HUB_SEMANTIC_SCHOLAR_API_KEY` | SHELVE | ✅ free key | **NOT configured** — needs signup |
| 2.5 | fofa | `HUB_FOFA_EMAIL` + `HUB_FOFA_KEY` | SHELVE | ❌ paid tier | **NOT configured** — needs signup + payment |
| 3.3 | compliapi | `HUB_COMPLIAPI_API_KEY` | SHELVE | ❌ paid tier | **NOT configured** — needs signup + payment |
| 3.4 | gitguardian | `HUB_GITGUARDIAN_API_KEY` | SHELVE | ❌ paid tier | **NOT configured** — needs signup + payment |
| 3.5 | congress_invests | `HUB_CONGRESSINVESTS_API_KEY` | SHELVE | ❌ paid tier | **NOT configured** — needs signup + payment |

**Plus 1 already-configured paid-tier reminder**: Phase 1.x already has
the OpenAQ key shipped to both 315 + 410 (commit `ba813f9`).

## Why "shelved" not "failed"

`accept-sp6.py` uses a 3-state result (passed / **shelved** / failed)
so that missing-key collectors don't poison the sp6 exit code. The
collector is **compiled, registered, and ready** — it just doesn't
fetch from upstream until you wire the key. This means:
- All 18 collectors are present in the binary (verified by `strings`).
- `monitor:sweep source="<name>"` lines appear in journal; the
  collector logs "no API key — shelved-by-design" once per sweep.
- sp6 reports the check as `SHELVE` with the signup link inline.

Exit-code interpretation: **only `failed` causes the script to exit
non-zero**. Shelves don't block deploys.

## How to add a key

### 1. Pick which source(s) you want to enable

Open this doc's table; if you're unsure, start with the 3 free
signups (currents, semantic_scholar — fofa is paid). The user has
explicitly asked the agent to **not** register paid services
without an explicit go-ahead.

### 2. Sign up at the upstream

Click each signup URL, complete the registration, copy the resulting
API key (and email, for fofa). Each upstream has its own approval
timeline:

| Upstream | Signup URL | Free tier | Approval time | Notes |
|---|---|---|---|---|
| OpenAQ | https://explore.openaq.org/register | 10k req/day | immediate | ✅ already wired |
| Currents | https://currentsapi.services/en/register | 250 req/day | immediate | |
| Semantic Scholar | https://www.semanticscholar.org/product/api#api-key-form | 100 req/sec | immediate (rate-limited) | |
| FOFA | https://en.fofa.info | paid tiers only | 1-3 business days | needs `email` + `key` |
| CompliAPI | https://docs.compliapi.com | paid tiers only | 1-3 business days | |
| GitGuardian | https://dashboard.gitguardian.com | paid tiers only | 1-3 business days | secret-detection |
| CongressInvests | https://congressinvests.com | paid tiers only | 1-3 business days | US politician trades |

### 3. Edit `/home/zou/IntelHub/core/hub.env` on the VM

Append (or update) one line per env-var. The file is `0600` and
git-ignored — **never** commit it.

```bash
ssh -o BatchMode=yes IntelHub
sudo -e /home/zou/IntelHub/core/hub.env   # opens in $EDITOR
```

Append:

```env
# Phase 2 / 3 env-gated collectors (see docs/PHASE-2-3-API-KEYS.md)
HUB_CURRENTS_API_KEY=your_key_here
HUB_SEMANTIC_SCHOLAR_API_KEY=your_key_here
HUB_FOFA_EMAIL=your_email_here
HUB_FOFA_KEY=your_key_here
HUB_COMPLIAPI_API_KEY=your_key_here
HUB_GITGUARDIAN_API_KEY=your_key_here
HUB_CONGRESSINVESTS_API_KEY=your_key_here
```

### 4. Restart hub-core (one stampede per restart — see AGENTS.md)

```bash
ssh -o BatchMode=yes IntelHub 'sudo systemctl restart hub-core'
# CRITICAL: wait ≥300s before the next acceptance / restart
# (stampede rule — 80+ monitor sources sweep on restart)
```

### 5. Re-run sp6 to verify the SHELVE → PASS promotion

```bash
KEY=$(ssh -o BatchMode=yes IntelHub 'grep -o "ihk_[a-f0-9]*" /home/zou/IntelHub/core/agent-keys.txt | head -1')
python3 scripts/accept-sp6.py "$KEY" "http://10.10.10.41:8800" \
  | grep -E "currents|semantic_scholar|fofa|compliapi|gitguardian|congress_invests|^==.*(passed|shelved|failed)"
```

Expected delta: each previously-`SHELVE` line becomes `PASS events
present (key configured)` for the collectors that have data flowing.

## What if my key doesn't work?

Each collector emits a single log line per sweep. Common failure
modes:

| Journal message | Likely cause | Fix |
|---|---|---|
| `no HUB_X_API_KEY — collector not registered` | forgot to add the env | re-edit hub.env, restart |
| `upstream non-2xx status=401` | key rejected | re-confirm with upstream |
| `upstream non-2xx status=403` | key approved but tier insufficient (FOFA / CompliAPI gated tiers) | upgrade tier at upstream |
| `upstream non-2xx status=429` | rate-limited | raise cadence (rare — defaults are already conservative) |

## Cross-VM rollout

The hub.env file is **VM-local**, not in git. To roll a key out from
one VM to another:

```bash
# On the source VM (e.g. 315 if you tested there first):
ssh Debian-test 'cat /home/zou/IntelHub/core/hub.env' > /tmp/hub.env
# Edit /tmp/hub.env to keep only the keys you want to promote
# Then on the target VM (410):
scp /tmp/hub.env IntelHub:/tmp/hub.env
ssh IntelHub 'sudo cp /tmp/hub.env /home/zou/IntelHub/core/hub.env \
  && sudo chown root:root /home/zou/IntelHub/core/hub.env \
  && sudo chmod 0600 /home/zou/IntelHub/core/hub.env'
```

## Why per-VM (not a shared file in git)?

Secrets.env / hub.env are explicitly **git-ignored** and held on the
VM only. Reasons:

1. **No accidental secrets in commit history** — even if a key is
   later revoked, an old commit would still have it.
2. **Different VMs may need different tier keys** — e.g. 315's
   test key vs 410's production key.
3. **Defense in depth** — every deploy is an `update.sh` + restart,
   not a code change. The env file survives the deploy.

## Self-service automation (optional)

For VMs where you want hub.env edits to take effect without a
restart (e.g., a daily key-rotation cron), the collectors re-read
`HUB_X_API_KEY` on each sweep. So editing hub.env + waiting one
cadence interval (typically 30 min) is enough — **no restart
required** for most cases. The one-time restart on first key-add
ensures the new env is in the systemd `EnvironmentFile` cache.

## sp6 output reference

```text
PASS collector health cells >= 8 | acled,adsb,adsbx,ahmia,arcnautical,...,currents,fofa,...
PASS arcnautical events present | arcnautical=25
SHELVE currents events present (key configured) | HUB_CURRENTS_API_KEY not configured — collector shelved-by-design until signup at https://currentsapi.services/en/register
SHELVE fofa events present (key configured) | HUB_FOFA_EMAIL or HUB_FOFA_KEY not configured — collector shelved-by-design until signup at https://en.fofa.info
== 44 passed, 8 shelved, 32 failed ==
```

The 32 failed are pre-existing upstream flakes (ADS-B 429, Celestrak
502, etc.) — not caused by the SHELVE'd collectors.

## OpenAQ reminder

`HUB_OPENAQ_API_KEY` is the only Phase 2/3 env already configured. It's
on both 315 + 410 (`commit ba813f9` was the Phase 1.3 deploy that
introduced the env-gate pattern + shipped the key). No action needed.

## Related docs

- `docs/superpowers/roadmaps/2026-09-27-public-api-integration-roadmap.md`
  §3 (Phase 2) + §4 (Phase 3) — full design.
- `AGENTS.md` — `Phase 2/3 env-gated sources` rules + stampede cooldown.
- `scripts/accept-sp6.py` — line `if secret("HUB_X_KEY"):` per source.

---

Last updated: 2026-09-27, alongside commit ba80e71 (Phase 3.2) + 30afea5
(Phase 3.1). Owned by the public-API integration workstream.