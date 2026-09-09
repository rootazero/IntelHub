-- IntelHub Hub Core — SP2B governance & write plane schema
-- Spec: docs/superpowers/specs/2026-09-09-intelhub-hub-core-2b-design.md

-- §54 Unified Alert Center
CREATE TABLE alerts (
    alert_id    uuid PRIMARY KEY,
    severity    text NOT NULL,                  -- info|warning|critical
    source      text NOT NULL,                  -- osint|agent|sensor|infra|budget|security
    title       text NOT NULL,
    body        text,
    task_id     uuid REFERENCES tasks(task_id) ON DELETE SET NULL,
    investigation_id uuid REFERENCES investigations(investigation_id) ON DELETE SET NULL,
    entity_name text,
    evidence_id uuid REFERENCES documents(document_id) ON DELETE SET NULL,
    recommended_action text,
    status      text NOT NULL DEFAULT 'open',   -- open|ack|muted
    actor       text,                           -- who acked/muted
    dedupe_key  text,                           -- suppress repeat alerts for the same ongoing condition
    occurrence  int NOT NULL DEFAULT 1,         -- bumped when deduped
    created_at  timestamptz NOT NULL DEFAULT now(),
    updated_at  timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX alerts_status_idx ON alerts (status, created_at DESC);
CREATE INDEX alerts_dedupe_idx ON alerts (dedupe_key, created_at DESC);
CREATE INDEX alerts_source_idx ON alerts (source, created_at DESC);

-- Webhook delivery log (every attempt auditable)
CREATE TABLE alert_deliveries (
    delivery_id uuid PRIMARY KEY,
    alert_id    uuid NOT NULL REFERENCES alerts(alert_id) ON DELETE CASCADE,
    endpoint    text NOT NULL,
    status      text NOT NULL DEFAULT 'PENDING', -- PENDING|DELIVERED|FAILED
    attempts    int NOT NULL DEFAULT 0,
    last_error  text,
    created_at  timestamptz NOT NULL DEFAULT now(),
    delivered_at timestamptz
);
CREATE INDEX alert_deliveries_status_idx ON alert_deliveries (status, created_at);

-- §72: graph updates must enter a queue when Neo4j is unavailable (never lost)
CREATE TABLE graph_sync_queue (
    op_id       uuid PRIMARY KEY,
    op          jsonb NOT NULL,                 -- {type, ...validated params}
    status      text NOT NULL DEFAULT 'PENDING', -- PENDING|DONE|FAILED
    attempts    int NOT NULL DEFAULT 0,
    last_error  text,
    created_at  timestamptz NOT NULL DEFAULT now(),
    processed_at timestamptz
);
CREATE INDEX graph_sync_status_idx ON graph_sync_queue (status, created_at);

-- §36 canonical relationship store (Neo4j is relationship *memory*, PG is canonical)
CREATE TABLE relationships (
    relationship_id uuid PRIMARY KEY,
    from_entity uuid NOT NULL REFERENCES entities(entity_id) ON DELETE CASCADE,
    to_entity   uuid NOT NULL REFERENCES entities(entity_id) ON DELETE CASCADE,
    rel_type    text NOT NULL,                  -- controls|communicates_with|resolves_to|located_in|...
    attributes  jsonb NOT NULL DEFAULT '{}'::jsonb,
    created_by  text NOT NULL,
    created_at  timestamptz NOT NULL DEFAULT now(),
    UNIQUE (from_entity, to_entity, rel_type)
);
CREATE INDEX relationships_from_idx ON relationships (from_entity);
CREATE INDEX relationships_to_idx ON relationships (to_entity);

-- Claims gain entity links + mandatory evidence binding (§43: every claim traces to evidence)
CREATE TABLE claim_entities (
    claim_id    uuid NOT NULL REFERENCES claims(claim_id) ON DELETE CASCADE,
    entity_id   uuid NOT NULL REFERENCES entities(entity_id) ON DELETE CASCADE,
    role        text NOT NULL DEFAULT 'mentioned',  -- subject|object|mentioned
    PRIMARY KEY (claim_id, entity_id, role)
);
CREATE TABLE claim_evidence (
    claim_id    uuid NOT NULL REFERENCES claims(claim_id) ON DELETE CASCADE,
    document_id uuid NOT NULL REFERENCES documents(document_id) ON DELETE CASCADE,
    relation    text NOT NULL DEFAULT 'supports',   -- supports|contradicts
    PRIMARY KEY (claim_id, document_id, relation)
);

-- SP2B: embedding worker needs force flag (agent-requested) + attempt counter
ALTER TABLE embedding_jobs ADD COLUMN force boolean NOT NULL DEFAULT false;
ALTER TABLE embedding_jobs ADD COLUMN attempts int NOT NULL DEFAULT 0;

-- §42 embedding cache-key proof: content_hash + chunk_algorithm_version + model
CREATE TABLE embedding_chunks (
    chunk_id    uuid PRIMARY KEY,
    document_id uuid NOT NULL REFERENCES documents(document_id) ON DELETE CASCADE,
    chunk_ix    int NOT NULL,
    chunk_version text NOT NULL,                -- fixed-800-80-v1
    model       text NOT NULL,
    content_hash text NOT NULL,                 -- sha256 of chunk text
    token_count int NOT NULL,
    qdrant_point uuid NOT NULL,
    created_at  timestamptz NOT NULL DEFAULT now(),
    UNIQUE (document_id, chunk_ix, chunk_version, model)
);
CREATE INDEX embedding_chunks_doc_idx ON embedding_chunks (document_id);
