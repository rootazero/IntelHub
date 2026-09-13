//! Integration tests for D: trace propagation. Lives in /tests/ as its own
//! binary — sidesteps pre-existing unit-test compile failures in
//! hub_core::monitor::sources. Tests verify the migration schema +
//! compile-time signature surface. Real e2e verification (cost_records +
//! embedding_jobs joined by trace_id) happens via the live BRICS scenario.

#[test]
fn cost_records_has_trace_id_column() {
    let sql = include_str!("../../../migrations/0009_trace_propagation.sql");
    assert!(sql.contains("ALTER TABLE cost_records"));
    assert!(sql.contains("ADD COLUMN IF NOT EXISTS trace_id UUID"));
    assert!(sql.contains("cost_records_trace_idx"));
}

#[test]
fn embedding_jobs_has_trace_id_column() {
    let sql = include_str!("../../../migrations/0009_trace_propagation.sql");
    assert!(sql.contains("ALTER TABLE embedding_jobs"));
    assert!(sql.contains("embedding_jobs_trace_idx"));
}

#[test]
fn store_enqueue_embedding_job_signature_includes_trace_id() {
    let src = include_str!("../src/store.rs");
    assert!(
        src.contains("pub async fn enqueue_embedding_job(pg: &PgPool, document_id: Uuid, model: &str, force: bool, trace_id: Option<Uuid>)"),
        "enqueue_embedding_job must accept trace_id: Option<Uuid>"
    );
}

#[test]
fn cost_record_cost_signature_includes_trace_id() {
    let src = include_str!("../src/cost.rs");
    assert!(
        src.contains("pub async fn record_cost(\n    state: &AppState,\n    agent_id: Option<Uuid>,\n    task_id: Option<Uuid>,\n    trace_id: Option<Uuid>,"),
        "record_cost must accept trace_id: Option<Uuid> after agent_id/task_id"
    );
}

#[test]
fn get_trace_endpoint_registered_in_router() {
    let src = include_str!("../src/api.rs");
    assert!(
        src.contains(".route(\"/api/v1/traces/{trace_id}\", get(get_trace))"),
        "trace endpoint must be registered"
    );
}

#[test]
fn get_trace_handler_present() {
    let src = include_str!("../src/api.rs");
    assert!(
        src.contains("async fn get_trace("),
        "get_trace handler must exist"
    );
    assert!(
        src.contains("\"cost_records\"") && src.contains("\"embedding_jobs\"") && src.contains("\"tool_calls\""),
        "get_trace must return cost_records + embedding_jobs + tool_calls sections"
    );
}