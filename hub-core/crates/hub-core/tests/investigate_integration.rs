//! Integration tests for B: multi-hop Q&A planner. Pure — no IO. Lives
//! in /tests/ as its own binary (sidesteps pre-existing unit-test compile
//! failures in monitor::sources).

use hub_core::investigate::{plan, StepKind};

/// Design invariant: BRICS / currency / de-dollarization questions MUST
/// produce a hybrid_search + entity_lookup pair. This is the most common
/// OSINT archetype.
#[test]
fn plan_brics_question_includes_hybrid_and_entity() {
    let steps = plan("What is the BRICS de-dollarization pace?", None);
    assert!(!steps.is_empty(), "plan must produce at least 1 step");
    let kinds: Vec<&StepKind> = steps.iter().map(|s| &s.kind).collect();
    assert!(
        kinds.iter().any(|k| matches!(k, StepKind::SearchHybrid { .. })),
        "BRICS question must include a hybrid_search step"
    );
    assert!(
        kinds.iter().any(|k| matches!(k, StepKind::EntityLookup { .. })),
        "BRICS question must include an entity_lookup step"
    );
}

/// Design invariant: conflict / war / sanctions questions route to the
/// conflict archetype (hybrid + OFAC keyword search).
#[test]
fn plan_conflict_question_includes_sanctions_keyword() {
    let steps = plan("What is the latest Iran sanctions action?", None);
    let kinds: Vec<&StepKind> = steps.iter().map(|s| &s.kind).collect();
    assert!(
        kinds.iter().any(|k| matches!(k, StepKind::SearchHybrid { .. })),
        "conflict question must include hybrid search"
    );
    assert!(
        kinds.iter().any(|k| matches!(k, StepKind::SearchKeyword { query, .. } if query.contains("OFAC"))),
        "conflict question must include an OFAC keyword lookup"
    );
}

/// Design invariant: relationship-between X and Y routes to a graph path.
#[test]
fn plan_relationship_question_routes_to_graph_path() {
    let steps = plan("What is the relationship between China and Russia?", None);
    let kinds: Vec<&StepKind> = steps.iter().map(|s| &s.kind).collect();
    assert!(
        kinds.iter().any(|k| matches!(k, StepKind::GraphPath { from, to, .. }
            if from.contains("China") && to.contains("Russia"))),
        "relationship question must produce a China → Russia graph path"
    );
}

/// Design invariant: every step has a unique trace_id (parent walks into
/// each via /api/v1/traces/{step_trace_id}).
#[test]
fn plan_assigns_unique_trace_ids_per_step() {
    let steps = plan("What is BRICS de-dollarization pace?", None);
    let mut ids: Vec<uuid::Uuid> = steps.iter().map(|s| s.trace_id).collect();
    let before = ids.len();
    ids.sort_by_key(|u| u.to_string());
    ids.dedup();
    assert_eq!(ids.len(), before, "every step must have a unique trace_id");
}

/// Design invariant: when investigation_id is supplied, the plan ends
/// with a ClaimLookup step (so existing claims are checked before
/// returning).
#[test]
fn plan_with_investigation_id_includes_claim_lookup() {
    let inv = uuid::Uuid::new_v4();
    let steps = plan("What is BRICS doing?", Some(inv));
    let kinds: Vec<&StepKind> = steps.iter().map(|s| &s.kind).collect();
    assert!(
        kinds.iter().any(|k| matches!(k, StepKind::ClaimLookup { .. })),
        "plan with investigation_id must include a ClaimLookup step"
    );
}

/// Empty / weird questions still produce a sensible (single-step hybrid)
/// plan rather than panicking.
#[test]
fn plan_handles_empty_question() {
    let steps = plan("", None);
    assert!(!steps.is_empty(), "empty question must still produce a fallback step");
    let kinds: Vec<&StepKind> = steps.iter().map(|s| &s.kind).collect();
    assert!(
        kinds.iter().any(|k| matches!(k, StepKind::SearchHybrid { .. })),
        "empty question falls back to hybrid search"
    );
}
