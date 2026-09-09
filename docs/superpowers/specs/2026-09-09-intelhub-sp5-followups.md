# SP5 — Post-SP4 Follow-ups (bounded, user-directed)

**Date:** 2026-09-09 · **Status:** Delivered, acceptance 9/9 + full regressions green (SP2A 16, SP2B 33, SP3 19, SP4 25)

Three bounded items, decided via batched Q&A (brainstorming bounded path):

## 1. Crucix key-gated sources (Q1)
FIRMS_MAP_KEY / ACLED_EMAIL+PASSWORD (OAuth) / EIA_API_KEY written to VM `compose/.env.crucix` (0600, never committed). Crucix recreated → 27/29 sources OK; **fire layer live** (geo_events kind=fire populating). ACLED OAuth responds; conflict events appear as data arrives.

## 2. SpiderFoot + Huginn activation (Q2=D)
- Both containers now default-on (huginn needed `ALTER ROLE huginn CREATEDB` — the SP1 init-created role couldn't run the image's db:create bootstrap).
- **SpiderFoot bridge** (`spiderfoot.rs` worker, 120s tick): polls `/scanlist`, exports FINISHED scans via `/scaneventresultexportmulti?ids=<id>&filetype=json` (endpoint name verified against sfwebui.py — NOT `scanresultsexport`), ingests each scan as ONE evidence document (§48 normalization layer; no entity flood). Scan IDs are 8-char hex (not 32). Redis set `hub:spiderfoot:done` = idempotency.
- **Huginn bridge**: `POST /api/v1/evidence` (bearer-auth, 2MB cap, http(s) URL validation) → `ingest_content` direct path (§50 Evidence Event). Huginn agents push via HTTP Request Agent to `http://10.10.10.41:8800/api/v1/evidence`.

## 3. Telegram alert channel (Q3)
- `alerts.rs` dispatcher: endpoint `telegram://chat` rows → Bot API sendMessage (severity emoji, action, console link). Same severity filter as webhook; retries + alert_deliveries audit shared.
- Config: `HUB_ALERT_TELEGRAM_BOT_TOKEN` + `HUB_ALERT_TELEGRAM_CHAT_ID` in VM `core/secrets.env` (0600). chat_id discovered via getUpdates (chat 1069705420, @xiiizou).
- Verified live: SP5 test alert DELIVERED on attempt 1; searxng flap alert delivered to Telegram during regression.

## Test-suite fixes (regression hygiene)
- accept-sp2b: alert dedupe **bump** semantics broke fresh-row assumptions → cleanup closes lingering open flap alerts + server-time `created_at > cutoff` window; webhook check now splits webhook vs telegram delivery rows.
- accept-sp5.py added (9 checks incl. SpiderFoot e2e scan `sp5-mini` = sfp_dnsresolve on 93.184.215.14).

## Notes
- SpiderFoot "Passive" full scans can hang at STARTING on this box (upstream flakiness, 0 elements, 0% CPU); minimal modulelist scans work fine. Not a hub defect; use targeted module lists.
- Generic webhook sink (/tmp/webhook_sink.py :18899) still the dev sink; production webhook URL can replace it anytime via HUB_ALERT_WEBHOOK_URL (telegram already covers real notification).
