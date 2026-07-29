-- Add trading_mode column to separate paper and live order data
ALTER TABLE orders
ADD COLUMN trading_mode VARCHAR(10) NOT NULL DEFAULT 'paper';

-- Index for efficient mode filtering
CREATE INDEX idx_orders_trading_mode ON orders (trading_mode);

-- Composite indexes for common query patterns
CREATE INDEX idx_orders_mode_entry_time ON orders (trading_mode, entry_time DESC);
CREATE INDEX idx_orders_mode_exit_time ON orders (trading_mode, exit_time DESC);
