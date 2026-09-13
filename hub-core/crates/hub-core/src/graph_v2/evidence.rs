//! Evidence-binding helpers.
//!
//! Every typed-intent write in `compiler::dispatch` carries either ≥1
//! `evidence_doc_ids[]` entries (which we verify actually exist in
//! `documents`) or a non-empty `evidence_absent_reason`. The dispatcher
//! enforces the rule before calling `validate_doc_ids`; this module
//! implements the DB-side existence check.

use sqlx::PgPool;
use uuid::Uuid;

use crate::error::HubError;

/// Confirm every document_id exists in `documents`. Returns `Err` if any
/// ID is missing — the caller MUST also enforce "ids non-empty OR absent
/// reason provided" before calling, since an empty slice is allowed at
/// the API surface only when paired with `evidence_absent_reason`.
pub async fn validate_doc_ids(ids: &[Uuid], pool: &PgPool) -> Result<(), HubError> {
    if ids.is_empty() {
        return Err(HubError::Validation(
            "evidence_doc_ids required (or evidence_absent_reason)".into(),
        ));
    }
    let row: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM documents WHERE document_id = ANY($1)",
    )
    .bind(ids)
    .fetch_one(pool)
    .await?;
    let found = row.0 as usize;
    if found != ids.len() {
        return Err(HubError::Validation(format!(
            "evidence refers to {found}/{} existing documents",
            ids.len()
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    // Integration tests live in Task 5/9 (require live PG).

    /// Compile-time signature check — pins the function shape so a future
    /// refactor that changes the signature surfaces as a compile error in
    /// the integration harness rather than silently at the call sites.
    #[allow(dead_code)]
    fn _sig_compiles(
        ids: &[uuid::Uuid],
        pool: &sqlx::PgPool,
    ) -> impl std::future::Future<Output = Result<(), crate::error::HubError>> {
        super::validate_doc_ids(ids, pool)
    }
}
