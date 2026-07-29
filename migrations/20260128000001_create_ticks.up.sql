CREATE TABLE ticks (
    id BIGSERIAL PRIMARY KEY,
    timestamp TIMESTAMPTZ NOT NULL,
    symbol VARCHAR(20) NOT NULL,
    price NUMERIC(20, 8) NOT NULL,
    quantity NUMERIC(20, 8) NOT NULL,
    trade_id BIGINT NOT NULL,
    is_buyer_maker BOOLEAN NOT NULL DEFAULT false,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_ticks_timestamp ON ticks (timestamp);
CREATE INDEX idx_ticks_symbol_timestamp ON ticks (symbol, timestamp);
CREATE UNIQUE INDEX idx_ticks_trade_id ON ticks (trade_id);
