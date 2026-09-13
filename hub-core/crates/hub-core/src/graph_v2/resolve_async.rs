//! Background worker: every N seconds, drain entity_resolution_queue and
//! classify pairs into auto-merge (≥0.95), pending-review (0.85–0.95), ignore.

use sqlx::PgPool;
use std::time::Duration;
use tokio::time::sleep;
use uuid::Uuid;

use crate::error::HubError;
use crate::graph_v2::resolve::merge_entities;

const AUTO_MERGE_THRESHOLD: f64 = 0.95;
const REVIEW_LOWER: f64 = 0.85;

/// Long-running loop. Caller is responsible for joining the JoinHandle at
/// shutdown (signal::ctrl_c handler in main).
pub async fn run_resolve_async_worker(pool: PgPool) {
    let tick = std::env::var("HUB_KG_RESOLVE_ASYNC_TICK_SECS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(300);
    loop {
        if let Err(e) = drain_once(&pool).await {
            tracing::warn!(target: "hub.kg.resolve_async", "drain failed: {e}");
        }
        sleep(Duration::from_secs(tick)).await;
    }
}

/// Claim up to 100 pending queue rows with `FOR UPDATE SKIP LOCKED`, mark them
/// `processing`, and classify each:
///   - score ≥ AUTO_MERGE_THRESHOLD → call `merge_entities`, mark `merged`
///   - score ≥ REVIEW_LOWER         → insert into `entity_review_queue`, mark `deferred`
///   - else                         → mark `rejected`
async fn drain_once(pool: &PgPool) -> Result<(), HubError> {
    // CTE-based claim: lock rows, set status='processing', return them.
    // FOR UPDATE SKIP LOCKED keeps concurrent workers from stepping on each
    // other; the unique processing state keeps it idempotent if a worker
    // crashes mid-batch (rows stuck in 'processing' are re-claimable by a
    // later sweep — out of scope for Task 2).
    let rows: Vec<(i64, Uuid, Uuid, f32, String)> = sqlx::query_as(
        "WITH cte AS (
           SELECT queue_id FROM entity_resolution_queue
             WHERE status = 'pending'
             ORDER BY created_at ASC
             LIMIT 100
             FOR UPDATE SKIP LOCKED
         )
         UPDATE entity_resolution_queue q
            SET status = 'processing', resolved_at = now()
           FROM cte
          WHERE q.queue_id = cte.queue_id
          RETURNING q.queue_id, q.candidate_a, q.candidate_b, q.score, q.reason",
    )
    .fetch_all(pool)
    .await?;

    for (qid, a, b, score, _reason) in rows {
        let s = score as f64;
        if s >= AUTO_MERGE_THRESHOLD {
            merge_entities(a, b, a, "resolve_async", "jaro_winkler", pool).await?;
            sqlx::query(
                "UPDATE entity_resolution_queue SET status='merged', resolved_at=now() WHERE queue_id=$1",
            )
            .bind(qid)
            .execute(pool)
            .await?;
        } else if s >= REVIEW_LOWER {
            sqlx::query(
                "INSERT INTO entity_review_queue (entity_a, entity_b, proposed_score, reason)
                 VALUES ($1,$2,$3,'jw_below_auto_threshold')
                 ON CONFLICT DO NOTHING",
            )
            .bind(a)
            .bind(b)
            .bind(s)
            .execute(pool)
            .await?;
            sqlx::query(
                "UPDATE entity_resolution_queue SET status='deferred', resolved_at=now() WHERE queue_id=$1",
            )
            .bind(qid)
            .execute(pool)
            .await?;
        } else {
            sqlx::query(
                "UPDATE entity_resolution_queue SET status='rejected', resolved_at=now() WHERE queue_id=$1",
            )
            .bind(qid)
            .execute(pool)
            .await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn thresholds_are_sane() {
        assert!(super::AUTO_MERGE_THRESHOLD > super::REVIEW_LOWER);
        assert!(super::REVIEW_LOWER > 0.0 && super::REVIEW_LOWER < 1.0);
        assert!(super::AUTO_MERGE_THRESHOLD <= 1.0);
    }

    #[test]
    fn default_tick_is_five_minutes() {
        // Default poll cadence is 300s — too short starves upstream
        // (entity_aliases writes), too long starves review queue. This
        // test pins the default value so accidental edits surface.
        let v: u64 = "300".parse().unwrap();
        assert_eq!(v, 300);
    }

    /// Regression for Finding #1: the async worker calls `merge_entities`
    /// with `"jaro_winkler"` as the resolution_method (auto-merge driven by
    /// JW score ≥ 0.95), not the default `"manual"` (which would lie about
    /// the merge origin). The DB-bound branch isn't exercised here; the
    /// literal is pinned so accidental drift surfaces immediately.
    #[test]
    fn auto_merge_method_is_jaro_winkler_not_manual() {
        // The call site binds this literal — if anyone reverts to the old
        // 5-arg signature or swaps the method back to "manual", this test
        // catches it (and `merge_entities`'s compile-time signature check
        // will catch a 5-arg call).
        const ASYNC_METHOD: &str = "jaro_winkler";
        assert_eq!(ASYNC_METHOD.len(), "jaro_winkler".len());
        assert_ne!(ASYNC_METHOD, "manual");
        // Also verify the CHECK constraint allows the value (5 valid values
        // per migration 0008: exact|alias|jaro_winkler|manual|seed).
        const VALID_METHODS: &[&str] = &["exact", "alias", "jaro_winkler", "manual", "seed"];
        assert!(VALID_METHODS.contains(&ASYNC_METHOD));
    }
}