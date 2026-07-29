DROP INDEX IF EXISTS idx_orders_mode_exit_time;
DROP INDEX IF EXISTS idx_orders_mode_entry_time;
DROP INDEX IF EXISTS idx_orders_trading_mode;
ALTER TABLE orders DROP COLUMN trading_mode;
