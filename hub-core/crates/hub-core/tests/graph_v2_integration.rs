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
    let _ = hub_core::graph_v2::compiler::ALLOWED_KINDS;
    let _ = hub_core::graph_v2::compiler::require_evidence as fn(&serde_json::Value)
        -> Result<(), hub_core::error::HubError>;
}

// ===== Fix #1 — ALLOWED_KINDS allowlist (Spec §3.3) =====

#[test]
fn allowed_kinds_contains_v1_set() {
    let k = hub_core::graph_v2::compiler::ALLOWED_KINDS;
    for v1 in [
        "person", "org", "domain", "ip", "location", "event",
        "infrastructure", "software", "handle", "email", "phone", "crypto_wallet",
    ] {
        assert!(k.contains(&v1), "missing v1 kind: {v1}");
    }
}

#[test]
fn allowed_kinds_contains_v2_set() {
    let k = hub_core::graph_v2::compiler::ALLOWED_KINDS;
    for v2 in [
        "Person", "Organization", "GovernmentEntity", "Company",
        "Asset", "Aircraft", "Vessel", "Source", "ManualEntity",
    ] {
        assert!(k.contains(&v2), "missing v2 kind: {v2}");
    }
}

#[test]
fn check_kind_rejects_unknown() {
    use hub_core::error::HubError;
    let r = hub_core::graph_v2::compiler::check_kind("banana");
    match r {
        Err(HubError::Validation(m)) => assert!(m.contains("banana"), "msg: {m}"),
        other => panic!("expected Validation, got {other:?}"),
    }
}

#[test]
fn check_kind_accepts_lowercase_v1() {
    assert!(hub_core::graph_v2::compiler::check_kind("org").is_ok());
    assert!(hub_core::graph_v2::compiler::check_kind("domain").is_ok());
}

#[test]
fn check_kind_accepts_titlecase_v2() {
    assert!(hub_core::graph_v2::compiler::check_kind("Person").is_ok());
    assert!(hub_core::graph_v2::compiler::check_kind("Aircraft").is_ok());
}

#[tokio::test]
async fn dispatch_assert_entity_rejects_unknown_kind() {
    use hub_core::error::HubError;
    let pool = dummy_pool();
    let v = serde_json::json!({
        "action": "assert_entity",
        "kind": "banana",
        "name": "Chiquita",
        "evidence_absent_reason": "manually seeded"
    });
    let r = hub_core::graph_v2::compiler::dispatch(v, "tester", None, &pool).await;
    match r {
        Err(HubError::Validation(m)) => {
            assert!(m.contains("banana"), "msg: {m}");
            assert!(m.contains("ALLOWED_KINDS"), "msg: {m}");
        }
        other => panic!("expected Validation, got {other:?}"),
    }
}

#[tokio::test]
async fn dispatch_assert_entity_accepts_allowed_kind_before_db() {
    // Positive case for Fix #1: kind="org" passes the kind check, then the
    // dispatcher proceeds to the next DB op (resolve_entity). Without a real
    // DB we expect a non-Validation error from the DB layer.
    use hub_core::error::HubError;
    let pool = dummy_pool();
    let v = serde_json::json!({
        "action": "assert_entity",
        "kind": "org",
        "name": "Acme",
        "evidence_absent_reason": "manually seeded"
    });
    let r = hub_core::graph_v2::compiler::dispatch(v, "tester", None, &pool).await;
    // The kind check passed; the dispatch must NOT have failed at the
    // Validation kind-check stage. A DB-layer error (or further Validation
    // from a downstream check) is acceptable here.
    if let Err(HubError::Validation(m)) = &r {
        assert!(!m.contains("ALLOWED_KINDS"), "kind check unexpectedly rejected: {m}");
        assert!(!m.contains("banana"), "stale banana message: {m}");
    }
}

// ===== Fix #2 — confidence ∈ [0,1] (Spec §4 rule 6) =====

#[test]
fn check_confidence_accepts_zero() {
    assert!(hub_core::graph_v2::compiler::check_confidence(0.0, "x").is_ok());
}

#[test]
fn check_confidence_accepts_one() {
    assert!(hub_core::graph_v2::compiler::check_confidence(1.0, "x").is_ok());
}

#[test]
fn check_confidence_accepts_mid() {
    assert!(hub_core::graph_v2::compiler::check_confidence(0.5, "x").is_ok());
}

#[test]
fn check_confidence_rejects_above_one() {
    use hub_core::error::HubError;
    let r = hub_core::graph_v2::compiler::check_confidence(1.5, "assert_entity");
    match r {
        Err(HubError::Validation(m)) => {
            assert!(m.contains("assert_entity"), "msg: {m}");
            assert!(m.contains("1.5"), "msg: {m}");
        }
        other => panic!("expected Validation, got {other:?}"),
    }
}

#[test]
fn check_confidence_rejects_negative() {
    use hub_core::error::HubError;
    let r = hub_core::graph_v2::compiler::check_confidence(-0.1, "x");
    match r {
        Err(HubError::Validation(m)) => assert!(m.contains("-0.1"), "msg: {m}"),
        other => panic!("expected Validation, got {other:?}"),
    }
}

#[test]
fn check_confidence_rejects_nan() {
    use hub_core::error::HubError;
    let r = hub_core::graph_v2::compiler::check_confidence(f64::NAN, "x");
    match r {
        Err(HubError::Validation(_)) => {}
        other => panic!("expected Validation for NaN, got {other:?}"),
    }
}

#[tokio::test]
async fn dispatch_assert_entity_rejects_confidence_out_of_range() {
    use hub_core::error::HubError;
    let pool = dummy_pool();
    let v = serde_json::json!({
        "action": "assert_entity",
        "kind": "org",
        "name": "Acme",
        "confidence": 1.5,
        "evidence_absent_reason": "manually seeded"
    });
    let r = hub_core::graph_v2::compiler::dispatch(v, "tester", None, &pool).await;
    match r {
        Err(HubError::Validation(m)) => {
            assert!(m.contains("confidence"), "msg: {m}");
            assert!(m.contains("1.5"), "msg: {m}");
        }
        other => panic!("expected Validation, got {other:?}"),
    }
}

#[tokio::test]
async fn dispatch_assert_entity_rejects_negative_confidence() {
    use hub_core::error::HubError;
    let pool = dummy_pool();
    let v = serde_json::json!({
        "action": "assert_entity",
        "kind": "org",
        "name": "Acme",
        "confidence": -0.01,
        "evidence_absent_reason": "manually seeded"
    });
    let r = hub_core::graph_v2::compiler::dispatch(v, "tester", None, &pool).await;
    match r {
        Err(HubError::Validation(m)) => assert!(m.contains("confidence"), "msg: {m}"),
        other => panic!("expected Validation, got {other:?}"),
    }
}

#[tokio::test]
async fn dispatch_assert_relationship_rejects_confidence_out_of_range() {
    use hub_core::error::HubError;
    let pool = dummy_pool();
    let v = serde_json::json!({
        "action": "assert_relationship",
        "subject": "00000000-0000-0000-0000-000000000001",
        "object":  "00000000-0000-0000-0000-000000000002",
        "predicate": "owns",
        "confidence": 2.0,
        "evidence_doc_ids": ["00000000-0000-0000-0000-000000000003"]
    });
    let r = hub_core::graph_v2::compiler::dispatch(v, "tester", None, &pool).await;
    match r {
        Err(HubError::Validation(m)) => {
            assert!(m.contains("confidence"), "msg: {m}");
            assert!(m.contains("2"), "msg: {m}");
        }
        other => panic!("expected Validation, got {other:?}"),
    }
}

#[tokio::test]
async fn dispatch_assert_entity_accepts_in_range_confidence() {
    // Positive case for Fix #2: confidence=0.7 passes the range check, then
    // proceeds to the next DB op. The kind check has already passed (kind="org"),
    // so any Validation error here must be from a check AFTER confidence, not
    // from the confidence check itself.
    use hub_core::error::HubError;
    let pool = dummy_pool();
    let v = serde_json::json!({
        "action": "assert_entity",
        "kind": "org",
        "name": "Acme",
        "confidence": 0.7,
        "evidence_absent_reason": "manually seeded"
    });
    let r = hub_core::graph_v2::compiler::dispatch(v, "tester", None, &pool).await;
    if let Err(HubError::Validation(m)) = &r {
        assert!(!m.contains("confidence"), "confidence check unexpectedly rejected: {m}");
    }
}

// ===== Fix #3 — FK existence pre-validation (Spec §4 rule 7) =====
//
// These checks ARE DB ops — they need a live PG to verify the
// `not found` Validation error path. Without a live DB the dummy
// pool's acquire_timeout fires and the dispatch returns
// `Err(Db(PoolTimedOut))`. The tests below are marked `#[ignore]`
// for the integration harness and will be enabled in Task 5/9
// once a live DB is wired in.

#[tokio::test]
#[ignore = "requires live PG (Task 5 integration)"]
async fn dispatch_assert_contradiction_rejects_missing_claim_a() {
    use hub_core::error::HubError;
    let pool = dummy_pool();
    let v = serde_json::json!({
        "action": "assert_contradiction",
        "claim_a": "00000000-0000-0000-0000-0000000000aa",
        "claim_b": "00000000-0000-0000-0000-0000000000bb",
        "reason": "manual review",
        "evidence_absent_reason": "manually flagged"
    });
    let r = hub_core::graph_v2::compiler::dispatch(v, "tester", None, &pool).await;
    match r {
        Err(HubError::Validation(m)) => {
            assert!(m.contains("claim_a"), "msg: {m}");
            assert!(m.contains("not found"), "msg: {m}");
        }
        other => panic!("expected Validation, got {other:?}"),
    }
}

#[tokio::test]
#[ignore = "requires live PG (Task 5 integration)"]
async fn dispatch_link_evidence_rejects_missing_document_id() {
    use hub_core::error::HubError;
    let pool = dummy_pool();
    let v = serde_json::json!({
        "action": "link_evidence_to_claim",
        "claim_id": "00000000-0000-0000-0000-0000000000cc",
        "document_id": "00000000-0000-0000-0000-0000000000dd",
        "relation": "supports"
    });
    let r = hub_core::graph_v2::compiler::dispatch(v, "tester", None, &pool).await;
    match r {
        Err(HubError::Validation(m)) => {
            assert!(m.contains("claim_id"), "msg: {m}");
        }
        other => panic!("expected Validation, got {other:?}"),
    }
}

#[tokio::test]
#[ignore = "requires live PG (Task 5 integration)"]
async fn dispatch_mark_finding_rejects_missing_finding_id() {
    use hub_core::error::HubError;
    let pool = dummy_pool();
    let v = serde_json::json!({
        "action": "mark_finding_about_entity",
        "finding_id": "00000000-0000-0000-0000-0000000000ee",
        "entity_id":  "00000000-0000-0000-0000-0000000000ff",
        "confidence": 0.5
    });
    let r = hub_core::graph_v2::compiler::dispatch(v, "tester", None, &pool).await;
    match r {
        Err(HubError::Validation(m)) => assert!(m.contains("finding_id"), "msg: {m}"),
        other => panic!("expected Validation, got {other:?}"),
    }
}

#[tokio::test]
#[ignore = "requires live PG (Task 5 integration)"]
async fn dispatch_close_investigation_rejects_missing_investigation_id() {
    use hub_core::error::HubError;
    let pool = dummy_pool();
    let v = serde_json::json!({
        "action": "close_investigation_extract",
        "investigation_id": "00000000-0000-0000-0000-0000000000aa",
        "extraction": []
    });
    let r = hub_core::graph_v2::compiler::dispatch(v, "tester", None, &pool).await;
    match r {
        Err(HubError::Validation(m)) => {
            assert!(m.contains("investigation_id"), "msg: {m}");
        }
        other => panic!("expected Validation, got {other:?}"),
    }
}

// Compile-time / signature smoke for the FK helper.

#[tokio::test]
async fn fk_helper_signature_is_reachable() {
    // This test never actually queries — it just constructs the call so
    // a future refactor that changes the signature surfaces here as a
    // compile error rather than silently at the call sites.
    let pool = dummy_pool();
    let id = uuid::Uuid::nil();
    // The call will return Err(Db(...)) due to the lazy pool, but the
    // important thing is the call compiled and ran.
    let r = hub_core::graph_v2::evidence::validate_id_exists(
        "claims",
        "claim_id",
        id,
        "claim_id",
        &pool,
    )
    .await;
    assert!(r.is_err(), "expected Err from bogus pool");
}

// ===== Positive case for Fix #3 — valid path does NOT trip FK validation =====
//
// The pure-logic check that the FK validation is wired into the dispatch
// path: the dispatch function must reach the FK check (i.e., the helper
// is called). With a lazy pool this surfaces as Err(Db(PoolTimedOut)),
// not Err(Validation(...)), proving the helper was reached and the SQL
// query was attempted (not short-circuited by an upstream Validation).

#[tokio::test]
async fn dispatch_assert_contradiction_runs_fk_check_then_hits_db() {
    use hub_core::error::HubError;
    let pool = dummy_pool();
    let v = serde_json::json!({
        "action": "assert_contradiction",
        "claim_a": "00000000-0000-0000-0000-0000000000aa",
        "claim_b": "00000000-0000-0000-0000-0000000000bb",
        "reason": "manual review",
        "evidence_absent_reason": "manually flagged"
    });
    let r = hub_core::graph_v2::compiler::dispatch(v, "tester", None, &pool).await;
    // Must fail (no DB), but NOT at any pre-FK Validation gate:
    // require_evidence passed (absent reason present), parse_uuid passed
    // (valid UUIDs). The error must therefore originate from the FK check
    // (DB layer), not from a pre-FK Validation check.
    match r {
        Err(HubError::Validation(m)) => {
            panic!("unexpected pre-FK Validation failure: {m}");
        }
        Err(HubError::Db(_)) => {
            // FK check attempted against the bogus pool → PoolTimedOut, surfaced as Db.
            // This proves the FK check is in the path.
        }
        other => panic!("expected Err(Db) from FK check, got {other:?}"),
    }
}
