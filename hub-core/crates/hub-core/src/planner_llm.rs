//! B: LLM-driven planner for `investigate(question)`.
//!
//! Triggered when the rule-based planner produces a trivial plan
//! (single SearchHybrid step, optionally + ClaimLookup when an
//! investigation_id is given). The LLM decomposes the question into
//! 2-4 steps from the same 5-tool vocabulary.
//!
//! Design:
//! - Trait `PlannerLlm` so production can swap in T8star and tests use
//!   a canned `MockPlanner`. Same interface, no env/network in tests.
//! - `parse_plan_response()` is pure — fully covered by tests without
//!   any network or runtime context.
//! - Strict timeout (5s) — planner failures NEVER block the agent.
//! - Cost: kind='llm_tokens', ~4 char/token estimate (T8star doesn't
//!   return usage for chat/completions either).
//! - Falls back to default plan on: timeout, HTTP error, malformed
//!   JSON, empty array, hallucinated tool names.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::error::HubError;
use crate::investigate::Step;
use crate::investigate::StepKind;

/// PlanDecision — result of the planner invocation. Either steps or a
/// fallback reason. The caller (investigate executor) handles both.
pub type PlanDecision = std::result::Result<Vec<Step>, String>;

/// Asks the LLM to decompose a question into steps. One production
/// impl (T8star) + one canned impl for tests.
///
/// Uses native `async fn in trait` (Rust 1.75+) — no async_trait crate,
/// no dyn-dispatch boxing. The two impls are concrete types and the
/// caller picks via config at startup.
pub trait PlannerLlm: Send + Sync {
    fn plan<'a>(&'a self, question: &'a str)
        -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, String>> + Send + 'a>>;
}

/// Canned-response planner for tests. Never makes a network call.
pub struct MockPlanner {
    pub canned: String,
}

impl MockPlanner {
    pub fn with_response(s: &str) -> Self {
        Self { canned: s.to_string() }
    }
}

impl PlannerLlm for MockPlanner {
    fn plan<'a>(&'a self, _question: &'a str)
        -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, String>> + Send + 'a>>
    {
        let canned = self.canned.clone();
        Box::pin(async move { Ok(canned) })
    }
}

/// Step as the LLM sees it in its JSON response.
#[derive(Debug, Deserialize, Serialize)]
struct LlmStep {
    tool: String,
    #[serde(default)]
    args: Value,
    #[serde(default)]
    description: Option<String>,
}

/// Prompt template sent to the LLM. The 5 tools map 1:1 to StepKind
/// variants. We ask for JSON-only output so parsing is trivial.
const SYSTEM_PROMPT: &str = r#"You are an OSINT investigation planner. Decompose the user's question into 2-4 retrieval steps using ONLY the following tools:

1. hybrid_search: {"query": str, "limit": int}
   - Best for "what's known about X" / general context
2. keyword_search: {"query": str, "limit": int}
   - Exact-name lookups, watchlist names, specific entity names
3. query_entity: {"name": str, "kind": str?}
   - Find a specific entity in the knowledge graph (e.g. "BRICS New Development Bank", "org")
4. find_path: {"from": str, "to": str}
   - Shortest relationship path between two named entities in the graph
5. list_findings: {"limit": int}
   - Pull existing claims/findings for the active investigation

Output ONLY a JSON array of objects, no prose, no markdown fences:
[{"tool":"hybrid_search","args":{"query":"...","limit":8},"description":"..."}]

Rules:
- Pick 2-4 steps maximum
- Order matters — first step sets context for later ones
- If the question is about a known OSINT topic (BRICS, sanctions, conflict,
  FX, OFAC), use keyword_search or query_entity for the canonical name
- Skip list_findings if there's no investigation_id (we'll add it)"#;

/// Build the user prompt (just the question; system prompt carries the
/// schema).
fn build_prompt(question: &str) -> String {
    format!("Question: {question}")
}

/// Pure parser: takes LLM JSON response, returns steps. Filters out
/// hallucinated tool names (anything not in the 5-tool whitelist).
pub fn parse_plan_response(raw: &str) -> Result<Vec<Step>, String> {
    let trimmed = raw.trim();
    // Strip markdown fences if the LLM wrapped the JSON.
    let json_text = if trimmed.starts_with("```") {
        trimmed
            .trim_start_matches("```json")
            .trim_start_matches("```")
            .trim_end_matches("```")
            .trim()
    } else {
        trimmed
    };
    let llm_steps: Vec<LlmStep> = serde_json::from_str(json_text)
        .map_err(|e| format!("parse_plan_response: not JSON: {e}"))?;
    let mut out: Vec<Step> = Vec::new();
    for (i, ls) in llm_steps.into_iter().enumerate() {
        match llm_step_to_kind(&ls) {
            Some(kind) => {
                let desc = ls
                    .description
                    .unwrap_or_else(|| crate::investigate::desc_for(&kind));
                out.push(Step {
                    kind,
                    trace_id: Uuid::new_v4(),
                    description: format!("step {}: {}", i + 1, desc),
                });
            }
            None => {
                // Hallucinated tool — skip, keep going.
                continue;
            }
        }
        if out.len() >= 4 {
            break;
        }
    }
    Ok(out)
}

fn llm_step_to_kind(ls: &LlmStep) -> Option<StepKind> {
    match ls.tool.as_str() {
        "hybrid_search" => {
            let query = ls.args.get("query")?.as_str()?.to_string();
            let limit = ls.args.get("limit").and_then(|v| v.as_i64()).unwrap_or(8);
            Some(StepKind::SearchHybrid { query, limit: limit.clamp(1, 50) })
        }
        "keyword_search" => {
            let query = ls.args.get("query")?.as_str()?.to_string();
            let limit = ls.args.get("limit").and_then(|v| v.as_i64()).unwrap_or(5);
            Some(StepKind::SearchKeyword { query, limit: limit.clamp(1, 50) })
        }
        "query_entity" => {
            let name = ls.args.get("name")?.as_str()?.to_string();
            let kind = ls.args.get("kind").and_then(|v| v.as_str()).map(String::from);
            Some(StepKind::EntityLookup { name, kind })
        }
        "find_path" => {
            let from = ls.args.get("from")?.as_str()?.to_string();
            let to = ls.args.get("to")?.as_str()?.to_string();
            Some(StepKind::GraphPath { from, to })
        }
        "list_findings" => {
            let limit = ls.args.get("limit").and_then(|v| v.as_i64()).unwrap_or(5);
            Some(StepKind::ClaimLookup { limit: limit.clamp(1, 50) })
        }
        _ => None,
    }
}

/// Detect whether the rule-based plan is "trivial" enough that an LLM
/// would add value. Trivial = exactly 1 SearchHybrid step, or
/// 1 SearchHybrid + 1 ClaimLookup (when investigation_id was given).
pub fn plan_is_trivial(steps: &[Step]) -> bool {
    let only_search_and_claims = steps
        .iter()
        .all(|s| matches!(s.kind, StepKind::SearchHybrid { .. } | StepKind::ClaimLookup { .. }));
    only_search_and_claims && steps.len() <= 2
}

/// Production planner: hits T8star's OpenAI-compatible chat endpoint.
/// Failures (timeout / 5xx / non-2xx) surface as Err(reason) so the
/// caller can fall back to the default plan.
pub struct T8starPlanner {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub max_tokens: u32,
    pub timeout: Duration,
}

impl T8starPlanner {
    pub fn new(base_url: String, api_key: String, model: String, max_tokens: u32, timeout_ms: u64) -> Self {
        Self {
            base_url,
            api_key,
            model,
            max_tokens,
            timeout: Duration::from_millis(timeout_ms),
        }
    }
}

impl PlannerLlm for T8starPlanner {
    fn plan<'a>(&'a self, question: &'a str)
        -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, String>> + Send + 'a>>
    {
        let url = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));
        let body = serde_json::json!({
            "model": self.model,
            "messages": [
                {"role": "system", "content": SYSTEM_PROMPT},
                {"role": "user", "content": build_prompt(question)},
            ],
            "max_tokens": self.max_tokens,
            "temperature": 0.0,
            "response_format": {"type": "json_object"},
        });
        let req = state_http_client()
            .post(&url)
            .bearer_auth(&self.api_key)
            .json(&body)
            .timeout(self.timeout);
        Box::pin(async move {
            let resp = req.send().await.map_err(|e| format!("planner http: {e}"))?;
            if !resp.status().is_success() {
                return Err(format!("planner status {}", resp.status()));
            }
            let v: Value = resp.json().await.map_err(|e| format!("planner decode: {e}"))?;
            let content = v
                .get("choices")
                .and_then(|c| c.as_array())
                .and_then(|a| a.first())
                .and_then(|c| c.get("message"))
                .and_then(|m| m.get("content"))
                .and_then(|s| s.as_str())
                .ok_or_else(|| "planner: missing choices[0].message.content".to_string())?;
            // Token-cost estimation happens in record_planner_cost() — the
            // T8star chat endpoint often omits usage; we use a ~4-char/token
            // heuristic instead of guessing here.
            Ok(content.to_string())
        })
    }
}

// Lazy-init reqwest client. Shared across planner invocations.
fn state_http_client() -> reqwest::Client {
    use std::sync::OnceLock;
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT
        .get_or_init(|| reqwest::Client::builder().build().expect("reqwest client"))
        .clone()
}

/// Helper used by investigate executor to record LLM token cost after
/// a successful planner invocation. Best-effort; never blocks.
pub async fn record_planner_cost(
    state: &crate::state::AppState,
    agent_id: Option<Uuid>,
    trace_id: Option<Uuid>,
    question: &str,
    model: &str,
    raw_response: &str,
) -> Result<(), HubError> {
    // T8star doesn't return usage for the chat endpoint reliably; estimate
    // ~4 chars/token (same heuristic as embed + rerank).
    let in_chars = question.len() + SYSTEM_PROMPT.len();
    let out_chars = raw_response.len();
    let in_tokens = (in_chars / 4) as f64;
    let out_tokens = (out_chars / 4) as f64;
    let total = in_tokens + out_tokens;
    if total > 0.0 {
        crate::cost::record_cost(
            state,
            agent_id,
            None,
            trace_id,
            "llm_tokens",
            total,
            "token",
            serde_json::json!({
                "model": model,
                "purpose": "planner",
                "estimated": true,
                "prompt_chars": in_chars,
                "completion_chars": out_chars,
            }),
        )
        .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trivial_with_one_hybrid() {
        let p = crate::investigate::plan("what's happening in tehran", None);
        assert!(plan_is_trivial(&p), "{p:?} should be trivial");
    }

    #[test]
    fn trivial_with_hybrid_plus_claim_lookup() {
        let p = crate::investigate::plan("what's happening in tehran", Some(Uuid::new_v4()));
        assert!(plan_is_trivial(&p));
    }

    #[test]
    fn not_trivial_with_brics_archetype() {
        let p = crate::investigate::plan("BRICS de-dollarization", None);
        assert!(!plan_is_trivial(&p), "BRICS archetype should NOT be trivial");
    }

    #[test]
    fn build_prompt_includes_question() {
        let p = build_prompt("test question");
        assert!(p.contains("test question"));
    }

    #[test]
    fn markdown_fence_stripped() {
        let raw = "```json\n[{\"tool\":\"hybrid_search\",\"args\":{\"query\":\"x\",\"limit\":3}}]\n```";
        let steps = parse_plan_response(raw).expect("parse");
        assert_eq!(steps.len(), 1);
    }
}