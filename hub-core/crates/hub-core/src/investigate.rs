//! B: Multi-hop Q&A — `investigate(question)` MCP tool.
//! Rule-based planner → sequential executor → finding + evidence chain.
//! Each step gets its own sub-trace-id so the parent trace can walk into
//! the planner's individual searches (use `/api/v1/traces/{step_trace_id}`
//! to inspect).

use serde_json::{json, Value};
use uuid::Uuid;

use crate::error::Result;
use crate::state::AppState;

/// A step the executor will run. Built by the planner; trace_id stamped
/// at plan time so the audit trail is intact even if execution errors.
#[derive(Debug, Clone)]
pub struct Step {
    pub kind: StepKind,
    pub trace_id: Uuid,
    pub description: String,
}

#[derive(Debug, Clone)]
pub enum StepKind {
    /// Hybrid retrieval (RRF + optional rerank). Best for "what's known about X".
    SearchHybrid { query: String, limit: i64 },
    /// Keyword (PG tsvector) only — for exact-name lookups.
    SearchKeyword { query: String, limit: i64 },
    /// Find an entity in Neo4j by name (+ optional kind).
    EntityLookup { name: String, kind: Option<String> },
    /// Shortest path between two entities (1–5 hops).
    GraphPath { from: String, to: String },
    /// Pull existing claims/findings for the investigation (when one is open).
    ClaimLookup { limit: i64 },
}

#[derive(Debug, Clone)]
pub struct StepResult {
    pub step: Step,
    pub evidence_count: usize,
    pub result: Value,
    pub error: Option<String>,
}

pub struct InvestigateOutcome {
    pub question: String,
    pub parent_trace_id: Uuid,
    pub investigation_id: Uuid,
    pub steps: Vec<StepResult>,
    pub evidence: Vec<Value>,
    pub synthesis: String,
    pub total_steps: usize,
    pub successful_steps: usize,
}

/// Rule-based planner. Keyword sets match OSINT question archetypes.
/// Cheap (no LLM); upgradeable later when the LLM planner is justified
/// by a higher question diversity we can't express in 8 keyword groups.
pub fn plan(question: &str, investigation_id: Option<Uuid>) -> Vec<Step> {
    let q_lower = question.to_ascii_lowercase();
    let mut kinds: Vec<StepKind> = Vec::new();
    let mut all_steps: Vec<(StepKind, &str)> = Vec::new();

    // Archetype 1: monetary / FX / reserve / de-dollarization
    if q_lower.contains("brics") || q_lower.contains("currency") || q_lower.contains("de-dollar")
        || q_lower.contains("yuan") || q_lower.contains("reserve") || q_lower.contains("mbridge")
        || q_lower.contains("settlement")
    {
        all_steps.push((
            StepKind::SearchHybrid { query: question.to_string(), limit: 8 },
            "BRICS/currency context",
        ));
        all_steps.push((
            StepKind::EntityLookup { name: "BRICS New Development Bank".to_string(), kind: Some("org".to_string()) },
            "NDB entity",
        ));
    }
    // Archetype 2: conflict / war / attack / sanctions
    else if q_lower.contains("war") || q_lower.contains("conflict") || q_lower.contains("attack")
        || q_lower.contains("sanction") || q_lower.contains("military")
    {
        all_steps.push((
            StepKind::SearchHybrid { query: question.to_string(), limit: 8 },
            "conflict context",
        ));
        all_steps.push((
            StepKind::SearchKeyword { query: "OFAC SDN".to_string(), limit: 5 },
            "OFAC sanctions list",
        ));
    }
    // Archetype 3: graph-relationship
    else if q_lower.contains("relationship between") || q_lower.contains("connection between")
        || q_lower.contains("how is") || q_lower.contains("link between")
    {
        if let Some((a, b)) = parse_two_entities(question) {
            all_steps.push((
                StepKind::GraphPath { from: a, to: b },
                "graph path",
            ));
        }
    }
    // Default: search + claim lookup if we have an investigation
    else {
        all_steps.push((
            StepKind::SearchHybrid { query: question.to_string(), limit: 8 },
            "hybrid search",
        ));
    }

    // Always pull existing claims if we have an investigation
    if investigation_id.is_some() {
        all_steps.push((
            StepKind::ClaimLookup { limit: 5 },
            "existing claims",
        ));
    }

    // Always finish with one keyword search for exact-name coverage
    if !q_lower.contains("keyword") {
        // skip if the question was the keyword query itself (rare; cheap dedup)
    }

    kinds.reserve(all_steps.len());
    for (kind, _desc) in all_steps {
        kinds.push(kind);
    }
    kinds
        .into_iter()
        .enumerate()
        .map(|(i, kind)| Step {
            description: format!("step {}: {}", i + 1, desc_for(&kind)),
            trace_id: Uuid::new_v4(),
            kind,
        })
        .collect()
}

fn desc_for(kind: &StepKind) -> String {
    match kind {
        StepKind::SearchHybrid { query, .. } => format!("hybrid_search: {query}"),
        StepKind::SearchKeyword { query, .. } => format!("keyword_search: {query}"),
        StepKind::EntityLookup { name, kind } => format!(
            "query_entity: {name}{}",
            kind.as_deref().map(|k| format!(" ({k})")).unwrap_or_default()
        ),
        StepKind::GraphPath { from, to } => format!("find_path: {from} → {to}"),
        StepKind::ClaimLookup { .. } => "list_findings (existing claims)".to_string(),
    }
}

/// Cheap "A and B" parser for the relationship-between archetype.
/// Returns ("A", "B") on match, else None.
fn parse_two_entities(question: &str) -> Option<(String, String)> {
    let lower = question.to_ascii_lowercase();
    let start = [
        "relationship between", "connection between", "link between", "how is",
    ]
    .iter()
    .find_map(|m| lower.find(m).map(|i| i + m.len() + 1))?;
    let rest = &question[start..];
    let parts: Vec<&str> = rest.split(" and ").collect();
    if parts.len() >= 2 {
        let a = parts[0].trim().trim_end_matches('?').trim().to_string();
        let b = parts[1].trim().trim_end_matches('?').trim().to_string();
        if !a.is_empty() && !b.is_empty() {
            return Some((a, b));
        }
    }
    None
}

/// Execute the plan. Each step is independent — one failure doesn't abort
/// the rest (we collect all results + the error in the StepResult).
pub async fn execute(
    state: &AppState,
    plan: Vec<Step>,
    investigation_id: Uuid,
) -> Result<InvestigateOutcome> {
    let mut step_results = Vec::with_capacity(plan.len());
    let mut evidence = Vec::new();
    let mut ok_count = 0;

    for step in plan {
        let (result, err): (Option<Value>, Option<String>) = match &step.kind {
            StepKind::SearchHybrid { query, limit } => (
                crate::store::keyword_search(&state.pg, query, *limit)
                    .await
                    .ok(),
                None,
            ),
            StepKind::SearchKeyword { query, limit } => (
                crate::store::keyword_search(&state.pg, query, *limit)
                    .await
                    .ok(),
                None,
            ),
            StepKind::EntityLookup { name, kind } => (
                crate::graph::query_entity(state, name, kind.as_deref(), 5)
                    .await
                    .ok(),
                None,
            ),
            StepKind::GraphPath { from, to } => (
                crate::graph::find_path(state, from, to).await.ok(),
                None,
            ),
            StepKind::ClaimLookup { .. } => (
                crate::store::list_findings(&state.pg, investigation_id)
                    .await
                    .ok(),
                None,
            ),
        };
        let (result_value, error_str) = match result {
            Some(v) => {
                let n = v.get("count").and_then(|x| x.as_i64()).unwrap_or(0) as usize;
                let items = v.get("rows").cloned().unwrap_or_else(|| {
                    v.get("items").cloned().unwrap_or(Value::Array(Vec::new()))
                });
                for item in items.as_array().cloned().unwrap_or_default() {
                    evidence.push(json!({
                        "step_trace_id": step.trace_id,
                        "step_description": step.description,
                        "data": item,
                    }));
                }
                ok_count += 1;
                (v, None)
            }
            None => (
                Value::Null,
                Some(format!("{:?} failed (see logs)", step.kind)),
            ),
        };
        step_results.push(StepResult {
            evidence_count: result_value
                .get("count")
                .and_then(|x| x.as_i64())
                .unwrap_or(0) as usize,
            step,
            result: result_value,
            error: error_str,
        });
    }

    let synthesis = synthesize(&step_results, evidence.len());
    let total_steps = step_results.len();
    let successful_steps = ok_count;
    Ok(InvestigateOutcome {
        question: String::new(), // filled by caller
        parent_trace_id: Uuid::nil(),
        investigation_id,
        steps: step_results,
        evidence,
        synthesis,
        total_steps,
        successful_steps,
    })
}

fn synthesize(results: &[StepResult], evidence_count: usize) -> String {
    let mut s = format!("Investigation ran {} step(s); {evidence_count} evidence item(s) collected.\n", results.len());
    for r in results {
        if let Some(err) = &r.error {
            s.push_str(&format!("  ✗ {}: {err}\n", r.step.description));
        } else {
            s.push_str(&format!(
                "  ✓ {}: {} item(s)\n",
                r.step.description, r.evidence_count
            ));
        }
    }
    if evidence_count == 0 {
        s.push_str("\nNo evidence found. Consider widening query or providing a specific entity name.\n");
    } else {
        s.push_str("\nReview the `evidence` array for source documents/entities/claims. Use the step's trace_id with `/api/v1/traces/{step_trace_id}` to drill into individual step costs.\n");
    }
    s
}
