//! Risk control system
//!
//! Provides composable risk checks and centralized risk management:
//! - `types`: Core data types (RiskDecision, PortfolioState, OrderRequest)
//! - `checks`: RiskCheck trait and implementations (MaxPositionSizeCheck, etc.)
//! - `config`: Per-strategy risk configuration and override merging
//! - `engine`: RiskEngine actor for centralized risk evaluation
//! - `circuit_breaker`: TradingState for catastrophic loss prevention
//! - `position`: Position tracking for stop-loss monitoring
//! - `daily_loss`: Daily loss limit monitoring

pub mod checks;
pub mod circuit_breaker;
pub mod config;
pub mod daily_loss;
pub mod engine;
pub mod position;
pub mod types;

// Re-export key types for convenience
pub use checks::{MaxPositionSizeCheck, RiskCheck};
pub use config::{GlobalRiskConfig, StrategyRiskOverrides, StrategyRiskParams};
pub use circuit_breaker::TradingState;
pub use daily_loss::{DailyLossLimitCheck, DailyPnlState};
pub use engine::{RiskEngineConfig, RiskEngineHandle, RiskMessage, RiskStateSnapshot, StopLossEvent, start_risk_engine};
pub use position::{OpenPosition, PositionTracker};
pub use types::{OrderRequest, PortfolioState, RiskDecision};
