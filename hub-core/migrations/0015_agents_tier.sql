-- 0015_agents_tier.sql
-- Adds agents.tier for caller identity tier enforcement (free/paid/admin).
-- DOWN: ALTER TABLE agents DROP COLUMN tier;

ALTER TABLE agents ADD COLUMN tier TEXT NOT NULL DEFAULT 'free';
CREATE INDEX idx_agents_tier ON agents(tier);
