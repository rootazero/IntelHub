-- 0017_audit_log.sql
-- Records tier-mismatch queries (free agent → paid source) and admin actions.
-- Retention 90 days (enforced by nightly cron; not in this migration).
-- DOWN: DROP TABLE audit_log;

CREATE TABLE audit_log (
    id BIGSERIAL PRIMARY KEY,
    ts TIMESTAMPTZ NOT NULL DEFAULT now(),
    agent_id UUID REFERENCES agents(agent_id),
    api_key_prefix TEXT NOT NULL,
    action TEXT NOT NULL,
    source_attempted TEXT,
    request_path TEXT,
    trace_id UUID,
    blocked BOOLEAN NOT NULL
);
CREATE INDEX idx_audit_log_ts ON audit_log(ts);
CREATE INDEX idx_audit_log_agent ON audit_log(agent_id);
