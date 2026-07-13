-- Snapshot-oriented data model: one row per (ticker, timestamp).
-- Hot scalar columns for filtering; JSONB payloads for full per-strike profiles.
-- Additive evolution only: new columns arrive with defaults so readers never break.

CREATE TABLE IF NOT EXISTS gex_dex_snapshots (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    timestamp TIMESTAMPTZ NOT NULL,
    ticker TEXT NOT NULL,
    expiry DATE NOT NULL,
    spot DOUBLE PRECISION NOT NULL,
    regime TEXT NOT NULL,
    net_gex_total DOUBLE PRECISION NOT NULL,
    abs_gex_total DOUBLE PRECISION NOT NULL,
    gamma_flip DOUBLE PRECISION,
    atm_iv DOUBLE PRECISION,
    signal_score DOUBLE PRECISION,
    traffic_light TEXT,
    recommendation JSONB NOT NULL DEFAULT '{}',
    vix_current DOUBLE PRECISION,
    expected_moves JSONB NOT NULL DEFAULT '[]',
    call_walls JSONB NOT NULL DEFAULT '[]',
    put_walls JSONB NOT NULL DEFAULT '[]',
    charm_vanna JSONB NOT NULL DEFAULT '{}',
    alert JSONB,
    gex_per_strike JSONB NOT NULL DEFAULT '[]',
    delta_exposure_profile JSONB NOT NULL DEFAULT '[]',
    net_delta_exposure DOUBLE PRECISION,
    trade_signals_advice JSONB,
    raw_options JSONB NOT NULL DEFAULT '[]',
    premarket_context JSONB NOT NULL DEFAULT '{}',
    vwap_data JSONB,
    market_regime TEXT,
    stoch_flags JSONB NOT NULL DEFAULT '{}'
);

-- Query pattern 1: latest snapshot(s) for a ticker
CREATE INDEX IF NOT EXISTS idx_snapshots_ticker_timestamp
    ON gex_dex_snapshots (ticker, timestamp DESC);

-- Query pattern 2: everything for a day
CREATE INDEX IF NOT EXISTS idx_snapshots_date
    ON gex_dex_snapshots (((timestamp AT TIME ZONE 'UTC')::date) DESC);

-- Retention: prune snapshots older than N days
-- DELETE FROM gex_dex_snapshots WHERE timestamp < NOW() - INTERVAL '1 day' * $1;
