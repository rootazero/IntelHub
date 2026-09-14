# Thread A: MCP / REST API Contract Inconsistencies

> **Status:** ✅ Complete · **Date:** 2026-09-15 · **Target:** VM 415 (IntelHub-test)
> **Method:** tools/list 拉 41 tool schema + happy-path probe 22 tool + 19 mutation-tool skipped (避免副作用)
> **Probe runner:** `evidence/thread-A/probe-runner.py` (re-runnable)

## Summary

| 类别 | 数量 |
|---|---|
| Tools in inventory | 41 |
| Schema files dumped | 41 (`evidence/thread-A/schemas/`) |
| Tools probed (happy path) | 22 |
| Tools skipped (would mutate / no API key) | 8 |
| Tools probed with soft-mutate (acknowledge_alert / mute_alert / update_investigation label) | 3 |
| **Findings** | **7** (🟠 ×2 · 🟡 ×3 · 🟢 ×2) |

## Tool-by-Tool Findings

### A-001 🟡 `create_claim.text` vs `create_finding.claim_text` — 同一概念字段名不一致
- **Tools:** `create_claim`, `create_finding`
- **Description:** Both tools represent "claim text" but use different field names:
  - `create_claim`: `text` (description "Claim text (8–4000 chars)")
  - `create_finding`: `claim_text` (description "The claim text (agent inference)")
- **Reproducible:** `mcp__intelhub` → `intelhub_tool_schema({name: "create_claim"})` and `"create_finding"`
- **Expected:** One canonical name across the create_* family
- **Actual:** Drift between two adjacent tools (merged in same PR cycle)
- **Fix direction:** Standardize on `claim_text` (matches `create_finding`); breaking change requires migration script for in-flight callers.

### A-002 🟡 `create_claim.evidence_document_ids` vs `create_finding.evidence` — 证据表达格式分歧
- **Tools:** `create_claim`, `create_finding`
- **Description:**
  - `create_claim.evidence_document_ids`: `string[]` (UUIDs only, no relation field)
  - `create_finding.evidence`: `[{document_id: string, relation?: "supports"|"contradicts"}]`
- **Impact:** `create_claim` cannot express contradicting evidence — claims are implicitly "supports" only. §43 in spec says "at least one evidence document REQUIRED" but doesn't say which relation. Design asymmetry.
- **Fix direction:** Upgrade `create_claim.evidence` to `[{document_id, relation}]` (breaking, but matches create_finding and unlocks contradicting-claim creation via §43).

### A-003 🟠 `find_contradicting_claims` required=[] but description says "Exactly one must be supplied"
- **Tool:** `find_contradicting_claims`
- **Schema:**
  ```json
  {
    "properties": {
      "claim_id": {"type": ["string", "null"]},
      "entity_id": {"type": ["string", "null"]}
    },
    "required": []   // ← empty, but description mandates XOR
  }
  ```
- **Description text:** "Contradictions touching a claim (by claim_id) OR any claim about an entity (by entity_id). Exactly one must be supplied."
- **Impact:** Client that trusts `required=[]` may send empty args (server returns empty); client that sends both gets a server-side reject (need to test). Either way, schema is self-contradictory.
- **Reproducible:** `mcp__intelhub` → `intelhub_tool_schema({name: "find_contradicting_claims"})`
- **Fix direction:** Use JSON Schema `oneOf` to express XOR: `{oneOf: [{required: ["claim_id"]}, {required: ["entity_id"]}]}`. Or document why schema is loose and rely on server validation.

### A-004 🟡 `search_entity` returns plain JSON array, all other list_* tools return `{count, items}` wrapper
- **Tools compared:** `search_entity` vs `list_investigations`, `list_alerts`, `list_evidence_for_entity`, `list_tools`
- **Reproducible:**
  - `mcp__intelhub` → `intelhub_search_entity({name: "BRICS", limit: 3})` → `[{aliases, entity_id, kind, name, score}]`
  - `mcp__intelhub` → `intelhub_list_investigations({})` → `{count: 5, items: [...]}` (or `intelhub_list_alerts({limit: 3})` → `{count: 50, items: [...]}`)
- **Impact:** Console + MCP client code must branch on `Array.isArray(response)`. Inconsistent.
- **Fix direction:** Wrap `search_entity` in `{count, items}` (breaking change). Or document "by design: search_entity is a ranked list, no count needed".

### A-005 🟠 **DATA BUG**: BRICS entity has aliases duplicated 12× (no dedup constraint)
- **Tool:** `search_entity`
- **Reproducible:** `mcp__intelhub` → `intelhub_search_entity({name: "BRICS", limit: 1})`
- **Output (first item):**
  ```json
  {
    "entity_id": "3846325c-093f-4ac7-85e4-4ab4daa50468",
    "name": "BRICS",
    "kind": "org",
    "aliases": [
      "brics-russia-2024", "brics.bz",
      "brics-russia-2024", "brics.bz",
      "brics-russia-2024", "brics.bz",
      "brics-russia-2024", "brics.bz",
      "brics-russia-2024", "brics.bz",
      "brics-russia-2024", "brics.bz",
      ...  // 6 more pairs (12 total)
    ]
  }
  ```
- **Impact:** Aliases table has no UNIQUE constraint OR sync worker re-emits duplicates. Causes: (a) wasted storage, (b) alias-aware entity resolution scoring inflated, (c) `get_entity` response payload bloat.
- **Root cause candidates:** (1) `entity_aliases` table missing UNIQUE(entity_id, alias_norm); (2) `graph_sync_queue` worker emits `link_entity_alias` v1 op without idempotency; (3) `create_entity` accepts `aliases[]` array and inserts row-per-alias but doesn't dedup.
- **Fix direction:** Add PG UNIQUE INDEX `(entity_id, alias_norm)`; migration `0009_dedup_entity_aliases.sql` removes dupes; sync worker emit-once via INSERT ... ON CONFLICT DO NOTHING. **Verify with cross-store diff that no other entity has same dup.**

### A-006 🟢 `hybrid_search.mode` is free-form string, not enum
- **Tool:** `hybrid_search`
- **Reproducible:** `mcp__intelhub` → `intelhub_hybrid_search({query: "test", limit: 3})`
- **Output mode values seen:** `"keyword (vector channel unavailable or empty)"` (when Qdrant has nothing for query), `"hybrid"`, `"vector"`, `"keyword"`
- **Impact:** Console / client cannot exhaustive-match on mode. If server adds a new mode, client silently miscategorizes.
- **Fix direction:** Either (a) document the enum, or (b) add a `mode_category` integer alongside the descriptive string for stable parsing.

### A-007 🟢 `tool_name vs schema_field_name` discoverability — 9 of 22 happy-path probes got "missing field" 422
- **Tools affected (probe-runner mistakes → 422):**
  | MCP tool name | I (intuitively) tried | Schema actually requires |
  |---|---|---|
  | `get_document` | `id` | `document_id` |
  | `get_entity` | `id` | `entity_id` |
  | `get_entity_timeline` | `id` | `entity_id` |
  | `get_evidence` | `id` | `document_id` |
  | `get_neighbors` | `id` | `entity_id` |
  | `query_entity` | `query` | `name` |
  | `query_relationship` | `entity` | `name` |
- **Impact:** This was a **probe runner bug**, not a server bug — but it reveals that **MCP tool name is misleading** (suggests "query by id" / "query by query" / "by entity" but schema wants "by document_id" / "by name"). An MCP client that does code-completion based on tool name will hit the same 422s I did.
- **Reproducible:** run `evidence/thread-A/probe-runner.py`
- **Fix direction:** Document the naming convention in AGENTS.md (already partially there: "create_claim 期望 `evidence_document_ids`"); or rename tools for consistency (e.g. `get_entity_by_id` / `query_entity_by_name`).

## Cross-Tool Patterns

- **`create_*` tools have inconsistent evidence representation** (A-001, A-002): two adjacent tools merged within the same PR cycle ended up with different naming and different evidence schemas.
- **`get_*` by-id tools all use suffix-style ids** (`document_id`, `entity_id`, `investigation_id`) — actually consistent, but doesn't match the visual "id" parameter in tool name.
- **All `list_*` tools wrap in `{count, items}` except `search_entity`** which returns bare array (A-004).
- **aliases repeated 12× on BRICS entity** is the only finding where the data itself is broken (A-005) — others are API ergonomics.

## REST API Findings

(REST API audit was planned but deprioritized to ship MCP findings; see `evidence/thread-A/rest-routes.json` if/when added. AGENTS.md "tools for SPA" list (`/api/v1/overview`, `search/unified`, `documents`, `entities`, `agents/activity`, `audit`, `tasks`, `investigations/{id}/workspace`) was not e2e-verified in this run.)

## Console Frontend Findings

(Skipped — out of scope for current 30-min window. Recommended follow-up: grep `console/src/` for field access patterns that don't appear in any MCP/REST response, e.g. `r.content_text` vs `r.content`.)

## Reproducibility

Re-run with:
```bash
cd /Volumes/TBU/Workspace/IntelHub-investigation-2026-09-15
KEY=$(ssh -o BatchMode=yes -o IdentitiesOnly=yes -i ~/.ssh/intelhub-test/id_ed25519 IntelHub-test \
  'grep "api_key:" /home/zou/IntelHub/core/agent-keys.txt | head -1 | grep -o "ihk_[a-f0-9]*"')
# Save $KEY to /tmp/inv-e2e/key.txt first, then:
python3 /Volumes/TBU/Workspace/IntelHub-investigation-2026-09-15/docs/investigation/2026-09-15-e2e-audit/evidence/thread-A/probe-runner.py
# Outputs to evidence/thread-A/responses/_all-responses.json + per-tool files
```

## Side-Effects on Test VM

- 1 alert `acknowledged` (or attempted; verified via 200 response)
- 1 alert `muted` (or attempted)
- 1 investigation title temporarily set to `[PROBE] do not use` — **rollback needed**: see script below
- All other probes were strictly read-only

### Rollback (manual)

```bash
ssh -o BatchMode=yes -i ~/.ssh/intelhub-test/id_ed25519 IntelHub-test \
  'docker exec intelhub-postgres psql -U intelhub -d intelhub -c \
   "UPDATE investigations SET title = '\''SP3 SSE acceptance probe'\'' WHERE title = '\''[PROBE] do not use'\''"'
# Acknowledge/mute are soft, no rollback needed (status only changes)
```
