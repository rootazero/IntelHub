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

/// Confirm `id` exists as a primary key in `{table}.{pk_col}`. Spec §4 rule 7
/// (FK existence pre-validation): every intent that takes an FK reference
/// (`claim_id`, `document_id`, `finding_id`, `investigation_id`, `claim_a`,
/// `claim_b`) MUST validate the row exists before issuing its INSERT.
///
/// `table` and `pk_col` are static compile-time strings — never user
/// input — so the `format!` here cannot inject SQL. The id is bound
/// through sqlx's parameterized query as a true placeholder.
pub async fn validate_id_exists(
    table: &'static str,
    pk_col: &'static str,
    id: Uuid,
    field: &'static str,
    pool: &PgPool,
) -> Result<(), HubError> {
    let sql = format!("SELECT 1 FROM {table} WHERE {pk_col} = $1 LIMIT 1");
    let row: Option<(i32,)> = sqlx::query_as(&sql)
        .bind(id)
        .fetch_optional(pool)
        .await?;
    if row.is_none() {
        return Err(HubError::Validation(format!(
            "{field} {id} not found in {table}"
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

    /// Compile-time signature check for `validate_id_exists` — pins both
    /// the function shape and the static-string contract on `table` and
    /// `pk_col`.
    #[allow(dead_code)]
    fn _fk_sig_compiles(
        id: uuid::Uuid,
        pool: &sqlx::PgPool,
    ) -> impl std::future::Future<Output = Result<(), crate::error::HubError>> {
        super::validate_id_exists("claims", "claim_id", id, "claim_id", pool)
    }
}
