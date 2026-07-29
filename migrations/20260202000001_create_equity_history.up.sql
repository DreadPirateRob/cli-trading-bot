CREATE TABLE equity_history (
    id BIGSERIAL PRIMARY KEY,
    equity NUMERIC(20, 8) NOT NULL,
    trading_mode VARCHAR(10) NOT NULL,
    timestamp TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Efficient queries by mode and time range
CREATE INDEX idx_equity_history_mode_timestamp
    ON equity_history (trading_mode, timestamp DESC);

-- For time-based queries across all modes
CREATE INDEX idx_equity_history_timestamp ON equity_history (timestamp);
