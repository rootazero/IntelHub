-- entities: temporal + resolution state
ALTER TABLE entities
  ADD COLUMN IF NOT EXISTS valid_from        timestamptz NULL,
  ADD COLUMN IF NOT EXISTS valid_until       timestamptz NULL,
  ADD COLUMN IF NOT EXISTS discovered_at     timestamptz NOT NULL DEFAULT now(),
  ADD COLUMN IF NOT EXISTS merged_into       uuid NULL REFERENCES entities(entity_id) ON DELETE SET NULL,
  ADD COLUMN IF NOT EXISTS resolution_method text NULL CHECK (resolution_method IN ('exact','alias','jaro_winkler','manual','seed')),
  ADD COLUMN IF NOT EXISTS resolution_confidence real NULL CHECK (resolution_confidence BETWEEN 0 AND 1);

-- relationships: temporal + evidence binding + provenance
ALTER TABLE relationships
  ADD COLUMN IF NOT EXISTS valid_from        timestamptz NULL,
  ADD COLUMN IF NOT EXISTS valid_until       timestamptz NULL,
  ADD COLUMN IF NOT EXISTS discovered_at     timestamptz NOT NULL DEFAULT now(),
  ADD COLUMN IF NOT EXISTS confidence        real NULL CHECK (confidence BETWEEN 0 AND 1),
  ADD COLUMN IF NOT EXISTS evidence_doc_ids  uuid[] NULL,
  ADD COLUMN IF NOT EXISTS source_ids        uuid[] NULL,
  ADD COLUMN IF NOT EXISTS created_by_task_id uuid NULL REFERENCES tasks(task_id) ON DELETE SET NULL;

CREATE INDEX IF NOT EXISTS entities_kind_name_norm     ON entities (kind, lower(name));
CREATE INDEX IF NOT EXISTS entities_merged_into        ON entities (merged_into) WHERE merged_into IS NOT NULL;
CREATE INDEX IF NOT EXISTS relationships_valid_from    ON relationships (valid_from) WHERE valid_from IS NOT NULL;
CREATE INDEX IF NOT EXISTS relationships_valid_until   ON relationships (valid_until) WHERE valid_until IS NOT NULL;
CREATE INDEX IF NOT EXISTS relationships_evidence_gin  ON relationships USING gin (evidence_doc_ids);
CREATE INDEX IF NOT EXISTS relationships_created_by    ON relationships (created_by_task_id) WHERE created_by_task_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS entity_aliases (
  alias_id     bigserial PRIMARY KEY,
  entity_id    uuid NOT NULL REFERENCES entities(entity_id) ON DELETE CASCADE,
  alias        text NOT NULL,
  alias_norm   text NOT NULL,
  kind         text NOT NULL,
  source       text NOT NULL,
  confidence   real NOT NULL CHECK (confidence BETWEEN 0 AND 1),
  created_at   timestamptz NOT NULL DEFAULT now(),
  UNIQUE (kind, alias_norm)
);
CREATE INDEX IF NOT EXISTS entity_aliases_entity ON entity_aliases (entity_id);

CREATE TABLE IF NOT EXISTS claim_contradictions (
  contradiction_id   bigserial PRIMARY KEY,
  claim_a            uuid NOT NULL REFERENCES claims(claim_id) ON DELETE CASCADE,
  claim_b            uuid NOT NULL REFERENCES claims(claim_id) ON DELETE CASCADE,
  reason             text NOT NULL,
  raised_by_task_id  uuid NULL REFERENCES tasks(task_id) ON DELETE SET NULL,
  raised_by_agent    text NULL,
  raised_at          timestamptz NOT NULL DEFAULT now(),
  resolved           boolean NOT NULL DEFAULT false,
  resolution_note    text NULL,
  CHECK (claim_a <> claim_b)
);
CREATE INDEX IF NOT EXISTS claim_contradictions_a ON claim_contradictions (claim_a);
CREATE INDEX IF NOT EXISTS claim_contradictions_b ON claim_contradictions (claim_b);

CREATE TABLE IF NOT EXISTS graph_change_log (
  change_id    bigserial PRIMARY KEY,
  op           text NOT NULL CHECK (op IN ('insert','update','merge','contradict','temporal_close')),
  target_kind  text NOT NULL CHECK (target_kind IN ('entity','relationship','claim','contradiction')),
  target_id    text NOT NULL,
  before       jsonb NULL,
  after        jsonb NOT NULL,
  changed_by   text NOT NULL,
  task_id      uuid NULL REFERENCES tasks(task_id) ON DELETE SET NULL,
  changed_at   timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS graph_change_log_target ON graph_change_log (target_kind, target_id, changed_at DESC);
CREATE INDEX IF NOT EXISTS graph_change_log_at      ON graph_change_log (changed_at DESC);

CREATE TABLE IF NOT EXISTS entity_resolution_queue (
  queue_id      bigserial PRIMARY KEY,
  candidate_a   uuid NOT NULL REFERENCES entities(entity_id) ON DELETE CASCADE,
  candidate_b   uuid NOT NULL REFERENCES entities(entity_id) ON DELETE CASCADE,
  score         real NOT NULL,
  reason        text NOT NULL,
  status        text NOT NULL DEFAULT 'pending' CHECK (status IN ('pending','merged','rejected','deferred')),
  created_at    timestamptz NOT NULL DEFAULT now(),
  resolved_at   timestamptz NULL,
  CHECK (candidate_a <> candidate_b)
);
CREATE INDEX IF NOT EXISTS entity_resolution_queue_pending ON entity_resolution_queue (status, created_at) WHERE status = 'pending';

CREATE TABLE IF NOT EXISTS entity_review_queue (
  review_id     bigserial PRIMARY KEY,
  entity_a      uuid NOT NULL REFERENCES entities(entity_id),
  entity_b      uuid NOT NULL REFERENCES entities(entity_id),
  proposed_score real NOT NULL,
  reason        text NOT NULL,
  status        text NOT NULL DEFAULT 'open' CHECK (status IN ('open','approved','rejected')),
  reviewer_note text NULL,
  reviewed_at   timestamptz NULL,
  created_at    timestamptz NOT NULL DEFAULT now()
);
