-- 0001_init.sql — SP2A canonical schema (spec §2).
-- Canonical metadata / evidence store. All timestamps timestamptz UTC.

CREATE TABLE agents (
    agent_id    uuid PRIMARY KEY,
    name        text NOT NULL UNIQUE,
    version     text,
    created_at  timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE api_keys (
    key_id      uuid PRIMARY KEY,
    agent_id    uuid NOT NULL REFERENCES agents(agent_id) ON DELETE CASCADE,
    key_hash    text NOT NULL UNIQUE,        -- sha256 hex of the presented key
    label       text,
    revoked     boolean NOT NULL DEFAULT false,
    created_at  timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE investigations (
    investigation_id uuid PRIMARY KEY,
    title       text NOT NULL,
    question    text,
    target      text,
    hypothesis  text,
    status      text NOT NULL DEFAULT 'open',   -- open|paused|closed
    created_by  text NOT NULL,                  -- agent name or 'user'
    created_at  timestamptz NOT NULL DEFAULT now(),
    updated_at  timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE tasks (
    task_id     uuid PRIMARY KEY,
    investigation_id uuid REFERENCES investigations(investigation_id) ON DELETE SET NULL,
    kind        text NOT NULL,                  -- search|crawl|ingest|analysis|...
    status      text NOT NULL DEFAULT 'pending',-- pending|running|completed|failed
    created_by  text NOT NULL,
    created_at  timestamptz NOT NULL DEFAULT now(),
    updated_at  timestamptz NOT NULL DEFAULT now(),
    detail      jsonb NOT NULL DEFAULT '{}'::jsonb
);

CREATE TABLE sources (
    source_id   uuid PRIMARY KEY,
    origin      text NOT NULL,                  -- searxng|crawl4ai|spiderfoot|manual|...
    base_url    text,
    reputation  real,
    first_seen  timestamptz NOT NULL DEFAULT now(),
    UNIQUE (origin, base_url)
);

CREATE TABLE documents (
    document_id uuid PRIMARY KEY,
    source_id   uuid REFERENCES sources(source_id) ON DELETE SET NULL,
    url_canonical text NOT NULL,
    url_original  text NOT NULL,
    title       text,
    content_hash text NOT NULL UNIQUE,          -- sha256 of canonical text (exact dedupe)
    simhash     bigint,                         -- 64-bit near-dup fingerprint
    retrieved_at timestamptz NOT NULL,
    published_at timestamptz,
    lang        text,
    raw_path    text,                           -- raw capture under data/raw/
    content_text text NOT NULL,
    content_tsv tsvector GENERATED ALWAYS AS (to_tsvector('simple', coalesce(title,'') || ' ' || content_text)) STORED,
    parent_task uuid REFERENCES tasks(task_id) ON DELETE SET NULL,
    metadata    jsonb NOT NULL DEFAULT '{}'::jsonb,
    provenance  jsonb NOT NULL DEFAULT '{}'::jsonb,
    embedding_status text NOT NULL DEFAULT 'PENDING',   -- PENDING|SKIPPED|DONE|FAILED (§72)
    created_at  timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX documents_tsv_idx ON documents USING gin (content_tsv);
CREATE INDEX documents_retrieved_idx ON documents (retrieved_at DESC);
CREATE INDEX documents_simhash_idx ON documents (simhash);

CREATE TABLE entities (
    entity_id   uuid PRIMARY KEY,
    kind        text NOT NULL,                  -- person|org|domain|ip|location|event|...
    name        text NOT NULL,
    aliases     jsonb NOT NULL DEFAULT '[]'::jsonb,
    attributes  jsonb NOT NULL DEFAULT '{}'::jsonb,
    created_by  text NOT NULL,
    created_at  timestamptz NOT NULL DEFAULT now(),
    UNIQUE (kind, name)
);

CREATE TABLE observations (
    observation_id uuid PRIMARY KEY,
    entity_id   uuid REFERENCES entities(entity_id) ON DELETE CASCADE,
    document_id uuid REFERENCES documents(document_id) ON DELETE CASCADE,
    snippet     text,
    observed_at timestamptz NOT NULL DEFAULT now(),
    created_by  text NOT NULL
);

CREATE TABLE claims (
    claim_id    uuid PRIMARY KEY,
    text        text NOT NULL,
    status      text NOT NULL DEFAULT 'unverified', -- unverified|supported|contradicted|disputed
    created_by  text NOT NULL,
    created_at  timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE findings (
    finding_id  uuid PRIMARY KEY,
    investigation_id uuid NOT NULL REFERENCES investigations(investigation_id) ON DELETE CASCADE,
    title       text NOT NULL,
    claim_text  text NOT NULL,
    source_confidence real,
    claim_confidence  real,
    model_confidence  real,
    created_by  text NOT NULL,                  -- agent name
    agent_id    uuid REFERENCES agents(agent_id) ON DELETE SET NULL,
    task_id     uuid REFERENCES tasks(task_id) ON DELETE SET NULL,
    created_at  timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE finding_evidence (
    finding_id  uuid NOT NULL REFERENCES findings(finding_id) ON DELETE CASCADE,
    document_id uuid NOT NULL REFERENCES documents(document_id) ON DELETE CASCADE,
    relation    text NOT NULL DEFAULT 'supports',   -- supports|contradicts
    PRIMARY KEY (finding_id, document_id, relation)
);

CREATE TABLE agent_runs (
    run_id      uuid PRIMARY KEY,
    agent_id    uuid REFERENCES agents(agent_id) ON DELETE SET NULL,
    session_id  text,
    task_id     uuid REFERENCES tasks(task_id) ON DELETE SET NULL,
    started_at  timestamptz NOT NULL DEFAULT now(),
    ended_at    timestamptz,
    summary     jsonb NOT NULL DEFAULT '{}'::jsonb
);

CREATE TABLE tool_calls (
    tool_call_id uuid PRIMARY KEY,
    agent_id    uuid REFERENCES agents(agent_id) ON DELETE SET NULL,
    session_id  text,
    task_id     uuid REFERENCES tasks(task_id) ON DELETE SET NULL,
    request_id  text NOT NULL,
    trace_id    text NOT NULL,
    tool        text NOT NULL,
    args_digest text,                           -- sha256 of canonical args (no raw secrets)
    status      text NOT NULL,                  -- ok|error|rejected
    latency_ms  integer,
    created_at  timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX tool_calls_agent_idx ON tool_calls (agent_id, created_at DESC);
CREATE INDEX tool_calls_trace_idx ON tool_calls (trace_id);

CREATE TABLE audit_records (
    audit_id    uuid PRIMARY KEY,
    actor       text NOT NULL,                  -- agent:<name>|user|system
    action      text NOT NULL,
    object_type text,
    object_id   text,
    why         text,
    source      text,
    result      text NOT NULL DEFAULT 'ok',
    detail      jsonb NOT NULL DEFAULT '{}'::jsonb,
    created_at  timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX audit_created_idx ON audit_records (created_at DESC);

CREATE TABLE events (
    event_id    uuid PRIMARY KEY,
    event_type  text NOT NULL,
    ts          timestamptz NOT NULL,
    actor       text NOT NULL,
    investigation_id uuid,
    trace_id    text,
    payload     jsonb NOT NULL DEFAULT '{}'::jsonb
);
CREATE INDEX events_ts_idx ON events (ts DESC);
CREATE INDEX events_type_idx ON events (event_type, ts DESC);

CREATE TABLE embedding_jobs (
    job_id      uuid PRIMARY KEY,
    document_id uuid NOT NULL REFERENCES documents(document_id) ON DELETE CASCADE,
    status      text NOT NULL DEFAULT 'PENDING',    -- PENDING|RUNNING|DONE|FAILED|SKIPPED
    reason      text,
    model       text NOT NULL,
    created_at  timestamptz NOT NULL DEFAULT now(),
    updated_at  timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX embedding_jobs_status_idx ON embedding_jobs (status, created_at);

CREATE TABLE cost_records (
    cost_id     uuid PRIMARY KEY,
    agent_id    uuid REFERENCES agents(agent_id) ON DELETE SET NULL,
    task_id     uuid REFERENCES tasks(task_id) ON DELETE SET NULL,
    kind        text NOT NULL,                  -- tool_call|http|crawl_page|llm_tokens|embedding_tokens
    amount      double precision NOT NULL,
    unit        text NOT NULL,
    detail      jsonb NOT NULL DEFAULT '{}'::jsonb,
    created_at  timestamptz NOT NULL DEFAULT now()
);
