-- 0007_retry_short_docs.sql — reset SKIPPED embedding jobs so the worker
-- re-evaluates them against the new lower word threshold (300 -> 50).
--
-- Pre-migration diagnosis (Sept 2026 e2e audit):
--   embedding_jobs: 908 total, 890 SKIPPED (98%), 18 DONE
--   documents word-count buckets: 461 <30w, 439 30-99w, 5 >=300w
--   All 890 SKIPPED reasons: "N words < min 300" — short-form OSINT (alerts,
--   RSS titles, Telegram messages) was universally rejected, leaving the
--   vector store sparse and semantic/hybrid search returning noise.
--
-- Post-migration: with HUB_EMBED_MIN_WORDS=50, these jobs will be
-- re-processed by the existing worker loop (claim PENDING via SKIP LOCKED,
-- re-run the word-count gate, embed if passing). Idempotent — the
-- content_hash + chunk_version + model cache key (§42) prevents duplicate
-- work on docs already in Qdrant.

BEGIN;

-- 1. Re-queue any embedding_jobs that were SKIPPED under the old threshold.
UPDATE embedding_jobs
SET    status = 'PENDING',
       reason = NULL,
       updated_at = now()
WHERE  status = 'SKIPPED'
  AND  reason LIKE '%< min %';

-- 2. Re-flag the corresponding documents so observers / dashboards reflect
--    the new state. (Worker will overwrite with DONE / SKIPPED again on
--    its next pass.)
UPDATE documents
SET    embedding_status = 'PENDING'
WHERE  embedding_status = 'SKIPPED';

COMMIT;