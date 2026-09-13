-- Allow 'processing' status in entity_resolution_queue.
--
-- Background: the resolve_async worker (graph_v2/resolve_async.rs) claims
-- pending rows via a CTE that flips status='processing' before doing the
-- JW-score classification.  The 0008 schema only allowed
-- pending/merged/rejected/deferred, so every claim attempt died with
-- "violates check constraint entity_resolution_queue_status_check" — the
-- queue grew unboundedly and the Neo4j-mirror-lag acceptance check timed
-- out (sp9 check 14).
--
-- Why 'processing' is the right shape (not just skipping the status flip):
-- concurrent resolve_async workers would otherwise double-process the same
-- pending row; the FOR UPDATE SKIP LOCKED + status flip pair is the
-- standard idempotent-claim pattern for worker queues.

ALTER TABLE entity_resolution_queue
  DROP CONSTRAINT IF EXISTS entity_resolution_queue_status_check;

ALTER TABLE entity_resolution_queue
  ADD CONSTRAINT entity_resolution_queue_status_check
  CHECK (status IN ('pending', 'processing', 'merged', 'rejected', 'deferred'));
