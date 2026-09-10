-- SP6B: financial/economic data plane (spec: docs/superpowers/specs/
-- 2026-09-10-intelhub-sp6b-finance-sources-design.md).

-- Time-series observations: macro series, market quotes, sentiment ratios.
-- Idempotent by (series, observed_at) — re-collection is an upsert.
CREATE TABLE IF NOT EXISTS signal_observations (
  series      text             NOT NULL,   -- 'fred:VIXCLS' / 'quote:AAPL' / 'sentiment:AAPL'
  source      text             NOT NULL,   -- 'monitor:fred' / 'monitor:markets' / 'finnhub' …
  observed_at timestamptz      NOT NULL,   -- the data point's own timestamp, never collection time
  value       double precision NOT NULL,
  payload     jsonb            NOT NULL DEFAULT '{}',
  PRIMARY KEY (series, observed_at)
);
CREATE INDEX IF NOT EXISTS signal_obs_series_time
  ON signal_observations (series, observed_at DESC);

-- Watchlist universe for the markets collector (console/agent manageable).
CREATE TABLE IF NOT EXISTS monitor_watchlist (
  symbol      text        PRIMARY KEY,
  asset_class text        NOT NULL DEFAULT 'us_stock',
  label       text        NOT NULL DEFAULT '',
  enabled     boolean     NOT NULL DEFAULT true,
  created_at  timestamptz NOT NULL DEFAULT now()
);

INSERT INTO monitor_watchlist (symbol, asset_class, label) VALUES
  ('SPY','etf','S&P 500 ETF'), ('QQQ','etf','Nasdaq 100 ETF'),
  ('DIA','etf','Dow 30 ETF'), ('IWM','etf','Russell 2000 ETF'),
  ('AAPL','us_stock','Apple'), ('MSFT','us_stock','Microsoft'),
  ('NVDA','us_stock','NVIDIA'), ('GOOGL','us_stock','Alphabet'),
  ('AMZN','us_stock','Amazon'), ('META','us_stock','Meta'),
  ('TSLA','us_stock','Tesla'), ('AMD','us_stock','AMD'),
  ('AVGO','us_stock','Broadcom'), ('JPM','us_stock','JPMorgan'),
  ('XOM','us_stock','Exxon Mobil'),
  ('GLD','etf','Gold ETF'), ('SLV','etf','Silver ETF'),
  ('USO','etf','Crude Oil ETF'), ('UUP','etf','USD Bull ETF'),
  ('TLT','etf','20Y+ Treasury ETF'),
  ('BTCUSD','crypto','Bitcoin'), ('ETHUSD','crypto','Ethereum')
ON CONFLICT (symbol) DO NOTHING;

-- financialdatasets.ai response cache (pay-per-request provider): one row per
-- ticker, written ONLY when all required endpoints succeeded; 30-day TTL with
-- required-endpoints completeness check (mirrors atlas's file cache semantics).
CREATE TABLE IF NOT EXISTS fd_cache (
  ticker     text        PRIMARY KEY,
  fetched_at timestamptz NOT NULL,
  endpoints  jsonb       NOT NULL,
  payload    jsonb       NOT NULL
);
