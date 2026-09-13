// Temporary integration harness for SP9 Task 3.
// Bypasses the broken lib-unit-tests (monitor/sources/{epa,fred}.rs pre-existing).
// All test logic mirrors the inline #[cfg(test)] mod tests blocks in graph_v2/*,
// plus compiler behavior on the JSON-intent path.

use chrono::{DateTime, Utc};

fn t(s: &str) -> DateTime<Utc> {
    chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S")
        .unwrap()
        .and_utc()
}

// ===== temporal::classify_overlap =====

#[test]
fn temporal_identical_is_same_window() {
    assert_eq!(
        hub_core::graph_v2::temporal::classify_overlap(
            Some(t("2025-01-01 00:00:00")),
            Some(t("2025-12-31 00:00:00")),
            Some(t("2025-01-01 00:00:00")),
            Some(t("2025-12-31 00:00:00"))
        ),
        hub_core::graph_v2::temporal::Overlap::SameWindow
    );
}

#[test]
fn temporal_nulls_same_window() {
    assert_eq!(
        hub_core::graph_v2::temporal::classify_overlap(None, None, None, None),
        hub_core::graph_v2::temporal::Overlap::SameWindow
    );
}

#[test]
fn temporal_adjacent_detected() {
    assert_eq!(
        hub_core::graph_v2::temporal::classify_overlap(
            Some(t("2025-01-01 00:00:00")),
            Some(t("2025-06-01 00:00:00")),
            Some(t("2025-06-01 00:00:00")),
            None
        ),
        hub_core::graph_v2::temporal::Overlap::Adjacent
    );
}

#[test]
fn temporal_overlap_detected() {
    assert_eq!(
        hub_core::graph_v2::temporal::classify_overlap(
            Some(t("2025-01-01 00:00:00")),
            Some(t("2025-08-01 00:00:00")),
            Some(t("2025-06-01 00:00:00")),
            Some(t("2025-12-31 00:00:00"))
        ),
        hub_core::graph_v2::temporal::Overlap::Overlap
    );
}

#[test]
fn temporal_disjoint_detected() {
    assert_eq!(
        hub_core::graph_v2::temporal::classify_overlap(
            Some(t("2024-01-01 00:00:00")),
            Some(t("2024-06-01 00:00:00")),
            Some(t("2025-01-01 00:00:00")),
            Some(t("2025-06-01 00:00:00"))
        ),
        hub_core::graph_v2::temporal::Overlap::Disjoint
    );
}

// ===== compiler::require_evidence (mirrors inline #[cfg(test)] mod tests) =====

#[test]
fn require_evidence_rejects_empty() {
    let v = serde_json::json!({"action": "assert_entity", "kind": "org", "name": "x"});
    assert!(hub_core::graph_v2::compiler::require_evidence(&v).is_err());
}

#[test]
fn require_evidence_accepts_ids() {
    let v = serde_json::json!({
        "action": "assert_entity",
        "kind": "org",
        "name": "x",
        "evidence_doc_ids": ["00000000-0000-0000-0000-000000000001"]
    });
    assert!(hub_core::graph_v2::compiler::require_evidence(&v).is_ok());
}

#[test]
fn require_evidence_accepts_absent_reason() {
    let v = serde_json::json!({
        "action": "assert_entity",
        "kind": "org",
        "name": "x",
        "evidence_absent_reason": "manually seeded"
    });
    assert!(hub_core::graph_v2::compiler::require_evidence(&v).is_ok());
}

// ===== compiler::ALLOWED_PREDICATES =====

#[test]
fn predicate_allowlist_contains_core_predicates() {
    let allow = hub_core::graph_v2::compiler::ALLOWED_PREDICATES;
    assert!(allow.contains(&"owns"));
    assert!(allow.contains(&"CONTRADICTS"));
    assert!(!allow.contains(&"hates"));
}

// ===== dispatch() — early-return validation paths (no DB needed) =====
//
// The unknown-action and missing-action branches in `dispatch` return
// before any pool touch, so we can drive them with a lazily-built
// pool whose connection target is bogus — it never connects because we
// never await a query on it.

fn dummy_pool() -> sqlx::PgPool {
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(std::time::Duration::from_millis(1))
        .connect_lazy("postgres://nobody:nobody@127.0.0.1:1/nobody")
        .expect("lazy connect must not fail")
}

#[tokio::test]
async fn dispatch_rejects_missing_action() {
    use hub_core::error::HubError;
    let pool = dummy_pool();
    let v = serde_json::json!({"kind": "org", "name": "x",
                               "evidence_doc_ids": ["00000000-0000-0000-0000-000000000001"]});
    let r = hub_core::graph_v2::compiler::dispatch(v, "tester", None, &pool).await;
    match r {
        Err(HubError::Validation(m)) => assert!(m.contains("action"), "msg: {m}"),
        other => panic!("expected Validation, got {other:?}"),
    }
}

#[tokio::test]
async fn dispatch_rejects_unknown_action() {
    use hub_core::error::HubError;
    let pool = dummy_pool();
    let v = serde_json::json!({"action": "delete_everything"});
    let r = hub_core::graph_v2::compiler::dispatch(v, "tester", None, &pool).await;
    match r {
        Err(HubError::Validation(m)) => assert!(m.contains("delete_everything"), "msg: {m}"),
        other => panic!("expected Validation, got {other:?}"),
    }
}

#[tokio::test]
async fn dispatch_assert_entity_validates_evidence_before_db() {
    use hub_core::error::HubError;
    let pool = dummy_pool();
    // No evidence_doc_ids AND no evidence_absent_reason → require_evidence
    // rejects BEFORE the bogus pool is touched.
    let v = serde_json::json!({"action": "assert_entity", "kind": "org", "name": "x"});
    let r = hub_core::graph_v2::compiler::dispatch(v, "tester", None, &pool).await;
    match r {
        Err(HubError::Validation(m)) => {
            assert!(m.contains("evidence"), "msg: {m}");
        }
        other => panic!("expected Validation, got {other:?}"),
    }
}

#[tokio::test]
async fn dispatch_assert_entity_rejects_missing_kind() {
    use hub_core::error::HubError;
    let pool = dummy_pool();
    let v = serde_json::json!({"action": "assert_entity", "name": "x",
                               "evidence_absent_reason": "manually seeded"});
    let r = hub_core::graph_v2::compiler::dispatch(v, "tester", None, &pool).await;
    match r {
        Err(HubError::Validation(m)) => assert!(m.contains("kind"), "msg: {m}"),
        other => panic!("expected Validation, got {other:?}"),
    }
}

#[tokio::test]
async fn dispatch_assert_entity_rejects_missing_name() {
    use hub_core::error::HubError;
    let pool = dummy_pool();
    let v = serde_json::json!({"action": "assert_entity", "kind": "org",
                               "evidence_absent_reason": "manually seeded"});
    let r = hub_core::graph_v2::compiler::dispatch(v, "tester", None, &pool).await;
    match r {
        Err(HubError::Validation(m)) => assert!(m.contains("name"), "msg: {m}"),
        other => panic!("expected Validation, got {other:?}"),
    }
}

#[tokio::test]
async fn dispatch_assert_relationship_rejects_unknown_predicate() {
    use hub_core::error::HubError;
    let pool = dummy_pool();
    let v = serde_json::json!({
        "action": "assert_relationship",
        "subject": "00000000-0000-0000-0000-000000000001",
        "object":  "00000000-0000-0000-0000-000000000002",
        "predicate": "hates",
        "evidence_doc_ids": ["00000000-0000-0000-0000-000000000003"]
    });
    let r = hub_core::graph_v2::compiler::dispatch(v, "tester", None, &pool).await;
    match r {
        Err(HubError::Validation(m)) => assert!(m.contains("hates"), "msg: {m}"),
        other => panic!("expected Validation, got {other:?}"),
    }
}

// ===== compile-time smoke that all the modules are public =====

#[test]
fn modules_are_public() {
    // Touch each module's type to make sure it compiles + is reachable.
    let _: hub_core::graph_v2::temporal::Overlap = hub_core::graph_v2::temporal::Overlap::Disjoint;
    let _: fn(
        Option<DateTime<Utc>>,
        Option<DateTime<Utc>>,
        Option<DateTime<Utc>>,
        Option<DateTime<Utc>>,
    ) -> hub_core::graph_v2::temporal::Overlap = hub_core::graph_v2::temporal::classify_overlap;
    // compiler's public surface:
    let _ = hub_core::graph_v2::compiler::ALLOWED_PREDICATES;
    let _ = hub_core::graph_v2::compiler::require_evidence as fn(&serde_json::Value)
        -> Result<(), hub_core::error::HubError>;
}
