//! Integration tests for B: LLM planner (T8star chat/completions) + parse +
//! trigger condition + fallback. Lives in /tests/ as its own binary
//! (sidesteps pre-existing unit-test compile failures in monitor::sources).
//!
//! Uses `MockPlanner` so no network calls. The real T8star client is
//! structurally identical — swap the impl at production wire-up time.

use hub_core::investigate::{plan, Step, StepKind};
use hub_core::planner_llm::{parse_plan_response, PlanDecision, PlannerLlm, MockPlanner};
use hub_core::investigate::parse_two_entities;

/// Trigger: rule-based plan is "trivial" (default hybrid + maybe claim_lookup)
/// → LLM fallback engages. Archetype matches (BRICS, conflict, relationship)
/// are non-trivial and do NOT trigger LLM.
#[test]
fn trigger_condition_skips_llm_when_archetype_matched() {
    let p = plan("BRICS de-dollarization", None);
    // BRICS archetype hits → plan has [hybrid_search, entity_lookup]
    assert!(p.len() >= 2, "archetype should produce >= 2 steps");
    // Marker: we can detect by checking no step is SearchHybrid alone.
    let has_entity = p.iter().any(|s| matches!(s.kind, StepKind::EntityLookup { .. }));
    assert!(has_entity, "BRICS archetype should include entity_lookup");
}

#[test]
fn trigger_condition_engages_llm_for_default_question() {
    let p = plan("what's happening with the mullah in tehran", None);
    // No archetype keyword matches → default branch → only SearchHybrid
    assert_eq!(p.len(), 1, "default plan should be exactly 1 SearchHybrid step");
    assert!(matches!(p[0].kind, StepKind::SearchHybrid { .. }));
    // This is the trigger — caller should call LLM to expand.
}

#[test]
fn trigger_condition_engages_llm_with_investigation_id() {
    let p = plan("what's happening in tehran", Some(uuid::Uuid::new_v4()));
    // Default branch → hybrid_search + claim_lookup (2 steps). Neither
    // hits any keyword archetype. Trivial.
    assert_eq!(p.len(), 2);
    assert!(matches!(p[0].kind, StepKind::SearchHybrid { .. }));
    assert!(matches!(p[1].kind, StepKind::ClaimLookup { .. }));
    use hub_core::planner_llm::plan_is_trivial;
    assert!(plan_is_trivial(&p));
}

/// Parse: well-formed JSON array of step dicts.
#[tokio::test]
async fn parse_plan_response_happy_path() {
    let raw = r#"[{"tool":"hybrid_search","args":{"query":"sanctions round 2026 Q4","limit":8},"description":"hybrid"},{"tool":"keyword_search","args":{"query":"OFAC SDN list 2026","limit":5},"description":"OFAC list"}]"#;
    let steps = parse_plan_response(raw).expect("should parse");
    assert_eq!(steps.len(), 2);
    match &steps[0].kind {
        StepKind::SearchHybrid { query, limit } => {
            assert_eq!(query, "sanctions round 2026 Q4");
            assert_eq!(*limit, 8);
        }
        _ => panic!("first step should be hybrid_search"),
    }
    match &steps[1].kind {
        StepKind::SearchKeyword { query, limit } => {
            assert_eq!(query, "OFAC SDN list 2026");
            assert_eq!(*limit, 5);
        }
        _ => panic!("second step should be keyword_search"),
    }
}

/// Parse: malformed JSON → empty result → caller falls back to default.
#[tokio::test]
async fn parse_plan_response_malformed_falls_back() {
    let raw = "not json at all";
    let steps = parse_plan_response(raw);
    assert!(steps.is_err(), "malformed JSON should return Err");
    // Caller treats Err as PlanDecision::Fallback("parse_error").
}

/// Parse: empty array → fallback (LLM returned nothing useful).
#[tokio::test]
async fn parse_plan_response_empty_falls_back() {
    let raw = "[]";
    let steps = parse_plan_response(raw).expect("valid empty array");
    assert!(steps.is_empty());
    // Caller treats empty as Fallback("empty_plan").
}

/// Parse: unknown tool name → reject that step, keep valid ones.
#[tokio::test]
async fn parse_plan_response_unknown_tool_filtered() {
    let raw = r#"[{"tool":"hybrid_search","args":{"query":"foo","limit":3},"description":"ok"},{"tool":"espionage_search","args":{"q":"x"},"description":"hallucinated"}]"#;
    let steps = parse_plan_response(raw).expect("parse");
    assert_eq!(steps.len(), 1, "unknown tool should be filtered out");
    assert!(matches!(steps[0].kind, StepKind::SearchHybrid { .. }));
}

/// Parse: GraphPath from/to extraction works.
#[tokio::test]
async fn parse_plan_response_graph_path() {
    let raw = r#"[{"tool":"find_path","args":{"from":"China","to":"Russia"},"description":"path"}]"#;
    let steps = parse_plan_response(raw).expect("parse");
    assert_eq!(steps.len(), 1);
    match &steps[0].kind {
        StepKind::GraphPath { from, to } => {
            assert_eq!(from, "China");
            assert_eq!(to, "Russia");
        }
        _ => panic!("should be graph path"),
    }
}

/// MockPlanner returns canned response. End-to-end through the trait
/// so a future T8star impl can drop in.
#[tokio::test]
async fn mock_planner_returns_canned_steps() {
    let planner = MockPlanner::with_response(
        r#"[{"tool":"hybrid_search","args":{"query":"x","limit":4},"description":"x"}]"#,
    );
    let raw = planner.plan("anything").await.expect("mock plan");
    let steps = parse_plan_response(&raw).expect("parse");
    assert_eq!(steps.len(), 1);
}

/// PlanDecision: success carries steps; failure carries reason.
#[test]
fn plan_decision_variants() {
    let success: PlanDecision = Ok(vec![]);
    let fallback: PlanDecision = Err("timeout".to_string());
    assert!(success.is_ok());
    assert!(fallback.is_err());
    assert_eq!(fallback.unwrap_err(), "timeout");
}

/// parse_two_entities helper (used by rule-based planner) still works.
#[test]
fn parse_two_entities_extracts_names() {
    let pair = parse_two_entities("What is the relationship between China and Russia?");
    assert!(pair.is_some());
    let (a, b) = pair.unwrap();
    assert_eq!(a, "China");
    assert_eq!(b, "Russia");
}