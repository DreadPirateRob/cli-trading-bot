//! Data persistence layer
//!
//! Handles database connections, tick storage, and order tracking.
//!
//! # Architecture
//!
//! The data pipeline flows as follows:
//!
//! ```text
//! WebSocket (TradeEvent)
//!        |
//!        v
//! tick_router::route_ticks_to_db()  -- converts TradeEvent -> NewTick
//!        |
//!        v (via mpsc channel)
//! writer::DbWriterActor  -- buffers ticks, batch inserts
//!        |
//!        v
//! repository::batch_insert_ticks()  -- UNNEST insert to PostgreSQL
//!        |
//!        v
//! PostgreSQL (ticks table)
//! ```
//!
//! # Usage
//!
//! ```ignore
//! // 1. Create database pool
//! let pool = create_pool(&config.database).await?;
//!
//! // 2. Start background database writer
//! let db_writer = start_db_writer(pool.clone(), WriterConfig::default());
//!
//! // 3. Create channel for WebSocket -> Router communication
//! let (tick_tx, tick_rx) = tokio::sync::mpsc::channel(1000);
//!
//! // 4. Spawn tick router to wire WebSocket to database
//! let router_handle = spawn_tick_router(tick_rx, db_writer.clone());
//!
//! // 5. In WebSocket handler, send TradeEvents to the channel
//! // tick_tx.send(trade_event).await?;
//!
//! // 6. On shutdown, drop tick_tx and await router_handle
//! ```

mod equity;
mod order;
mod pool;
mod repository;
mod tick;
mod tick_router;
mod writer;

pub use equity::{
    delete_equity_history, get_equity_history, get_latest_equity, insert_equity_point, EquityPoint,
};
pub use order::{
    calculate_pnl, close_order, get_open_orders, get_order, insert_order, CloseOrder, NewOrder,
    Order, OrderSide,
};
pub use pool::create_pool;
pub use repository::batch_insert_ticks;
pub use tick::{NewTick, Tick, TickConversionError};
pub use tick_router::{route_ticks_to_db, spawn_tick_router};
pub use writer::{start_db_writer, DbWriterHandle, WriterConfig};
