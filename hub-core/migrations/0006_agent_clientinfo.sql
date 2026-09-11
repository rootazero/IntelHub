-- SP8 dual-axis agent identity: the API key is the auth/budget axis;
-- clientInfo (MCP initialize handshake) is the observability axis, absorbed
-- onto the key-agent's row. No per-provider enumeration — whoever connects
-- declares itself.
ALTER TABLE agents ADD COLUMN IF NOT EXISTS last_seen_at timestamptz;
