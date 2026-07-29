//! Strategy system for trading signal generation.
//!
//! This module provides the core infrastructure for pluggable trading strategies.
//! All strategies implement the [`Strategy`] trait and generate [`Signal`]s
//! in response to market [`StrategyTick`]s.
//!
//! # Architecture
//!
//! ```text
//! Market Ticks ─> StrategyManager::on_tick() ─> Active Strategy ─> Signal ─> Risk Engine ─> Order
//! ```
//!
//! Strategies are stateful components that:
//! - Track technical indicators (RSI, moving averages, etc.)
//! - Generate Buy/Sell/Hold signals based on their logic
//! - Support hot-reload of configuration parameters
//!
//! The [`StrategyManager`] handles multiple strategies with only one active at a time,
//! routing ticks to the active strategy and supporting runtime strategy switching.

mod grid;
mod manager;
mod rsi_stddev;
mod types;

pub use grid::{GridConfig, GridLevel, GridStrategy};
pub use manager::StrategyManager;
pub use rsi_stddev::{RsiStdDevConfig, RsiStdDevStrategy};
pub use types::{Signal, Strategy, StrategyTick};
