//! Grid strategy unit tests
//!
//! Tests for the GridStrategy implementation using TDD methodology.

use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use serde_json::json;
use trading_bot::data::OrderSide;
use trading_bot::strategy::{GridConfig, GridLevel, GridStrategy};
use trading_bot::strategy::{Signal, Strategy, StrategyTick};

fn make_tick(symbol: &str, price: Decimal, timestamp: i64) -> StrategyTick {
    StrategyTick {
        symbol: symbol.to_string(),
        price,
        quantity: dec!(1),
        timestamp,
    }
}

mod grid_level_generation {
    use super::*;

    #[test]
    fn generates_correct_number_of_levels() {
        // center=100, spacing=0.01, levels=3 -> 6 total levels (3 buy, 3 sell)
        let config = GridConfig {
            spacing_pct: dec!(0.01),
            num_levels: 3,
            quantity_per_level: dec!(0.1),
            escape_multiplier: dec!(2.0),
        };
        let strategy = GridStrategy::new(config, dec!(100));

        let state = strategy.get_state();
        let levels: Vec<GridLevel> = serde_json::from_value(state["levels"].clone()).unwrap();

        assert_eq!(levels.len(), 6);
    }

    #[test]
    fn buy_levels_below_center() {
        // center=100, spacing=0.01, levels=3
        // Buy levels: 99, 98, 97
        let config = GridConfig {
            spacing_pct: dec!(0.01),
            num_levels: 3,
            quantity_per_level: dec!(0.1),
            escape_multiplier: dec!(2.0),
        };
        let strategy = GridStrategy::new(config, dec!(100));

        let state = strategy.get_state();
        let levels: Vec<GridLevel> = serde_json::from_value(state["levels"].clone()).unwrap();

        let buy_levels: Vec<&GridLevel> = levels
            .iter()
            .filter(|l| l.side == OrderSide::Buy)
            .collect();

        assert_eq!(buy_levels.len(), 3);
        assert_eq!(buy_levels[0].price, dec!(99));
        assert_eq!(buy_levels[1].price, dec!(98));
        assert_eq!(buy_levels[2].price, dec!(97));
    }

    #[test]
    fn sell_levels_above_center() {
        // center=100, spacing=0.01, levels=3
        // Sell levels: 101, 102, 103
        let config = GridConfig {
            spacing_pct: dec!(0.01),
            num_levels: 3,
            quantity_per_level: dec!(0.1),
            escape_multiplier: dec!(2.0),
        };
        let strategy = GridStrategy::new(config, dec!(100));

        let state = strategy.get_state();
        let levels: Vec<GridLevel> = serde_json::from_value(state["levels"].clone()).unwrap();

        let sell_levels: Vec<&GridLevel> = levels
            .iter()
            .filter(|l| l.side == OrderSide::Sell)
            .collect();

        assert_eq!(sell_levels.len(), 3);
        assert_eq!(sell_levels[0].price, dec!(101));
        assert_eq!(sell_levels[1].price, dec!(102));
        assert_eq!(sell_levels[2].price, dec!(103));
    }

    #[test]
    fn all_levels_start_unfilled() {
        let config = GridConfig {
            spacing_pct: dec!(0.01),
            num_levels: 3,
            quantity_per_level: dec!(0.1),
            escape_multiplier: dec!(2.0),
        };
        let strategy = GridStrategy::new(config, dec!(100));

        let state = strategy.get_state();
        let levels: Vec<GridLevel> = serde_json::from_value(state["levels"].clone()).unwrap();

        for level in &levels {
            assert!(
                !level.filled,
                "Level at {} should not be filled",
                level.price
            );
        }
    }
}

mod level_triggering {
    use super::*;

    #[test]
    fn price_at_center_returns_hold() {
        let config = GridConfig {
            spacing_pct: dec!(0.01),
            num_levels: 3,
            quantity_per_level: dec!(0.1),
            escape_multiplier: dec!(2.0),
        };
        let mut strategy = GridStrategy::new(config, dec!(100));

        let tick = make_tick("BTCUSDT", dec!(100), 1);
        let signal = strategy.on_tick(&tick);

        assert_eq!(signal, Signal::Hold);
    }

    #[test]
    fn price_drops_to_buy_level_returns_buy() {
        let config = GridConfig {
            spacing_pct: dec!(0.01),
            num_levels: 3,
            quantity_per_level: dec!(0.1),
            escape_multiplier: dec!(2.0),
        };
        let mut strategy = GridStrategy::new(config, dec!(100));

        // Price drops to first buy level (99)
        let tick = make_tick("BTCUSDT", dec!(99), 1);
        let signal = strategy.on_tick(&tick);

        assert_eq!(signal, Signal::Buy { quantity: dec!(0.1) });
    }

    #[test]
    fn price_rises_to_sell_level_returns_sell() {
        let config = GridConfig {
            spacing_pct: dec!(0.01),
            num_levels: 3,
            quantity_per_level: dec!(0.1),
            escape_multiplier: dec!(2.0),
        };
        let mut strategy = GridStrategy::new(config, dec!(100));

        // Price rises to first sell level (101)
        let tick = make_tick("BTCUSDT", dec!(101), 1);
        let signal = strategy.on_tick(&tick);

        assert_eq!(signal, Signal::Sell { quantity: dec!(0.1) });
    }

    #[test]
    fn filled_buy_level_does_not_retrigger() {
        let config = GridConfig {
            spacing_pct: dec!(0.01),
            num_levels: 3,
            quantity_per_level: dec!(0.1),
            escape_multiplier: dec!(2.0),
        };
        let mut strategy = GridStrategy::new(config, dec!(100));

        // First tick at 99 -> Buy
        let tick1 = make_tick("BTCUSDT", dec!(99), 1);
        let signal1 = strategy.on_tick(&tick1);
        assert_eq!(signal1, Signal::Buy { quantity: dec!(0.1) });

        // Second tick at 99 -> Hold (level already filled)
        let tick2 = make_tick("BTCUSDT", dec!(99), 2);
        let signal2 = strategy.on_tick(&tick2);
        assert_eq!(signal2, Signal::Hold);
    }

    #[test]
    fn filled_sell_level_does_not_retrigger() {
        let config = GridConfig {
            spacing_pct: dec!(0.01),
            num_levels: 3,
            quantity_per_level: dec!(0.1),
            escape_multiplier: dec!(2.0),
        };
        let mut strategy = GridStrategy::new(config, dec!(100));

        // First tick at 101 -> Sell
        let tick1 = make_tick("BTCUSDT", dec!(101), 1);
        let signal1 = strategy.on_tick(&tick1);
        assert_eq!(signal1, Signal::Sell { quantity: dec!(0.1) });

        // Second tick at 101 -> Hold (level already filled)
        let tick2 = make_tick("BTCUSDT", dec!(101), 2);
        let signal2 = strategy.on_tick(&tick2);
        assert_eq!(signal2, Signal::Hold);
    }

    #[test]
    fn triggers_nearest_unfilled_level() {
        let config = GridConfig {
            spacing_pct: dec!(0.01),
            num_levels: 3,
            quantity_per_level: dec!(0.1),
            escape_multiplier: dec!(2.0),
        };
        let mut strategy = GridStrategy::new(config, dec!(100));

        // Price drops past multiple buy levels (to 97)
        // Should only trigger nearest unfilled (99 first)
        let tick = make_tick("BTCUSDT", dec!(97), 1);
        let signal = strategy.on_tick(&tick);

        // Should trigger the nearest unfilled buy level
        assert_eq!(signal, Signal::Buy { quantity: dec!(0.1) });

        // Verify only one level was filled
        let state = strategy.get_state();
        let levels: Vec<GridLevel> = serde_json::from_value(state["levels"].clone()).unwrap();
        let filled_count = levels.iter().filter(|l| l.filled).count();
        assert_eq!(filled_count, 1);
    }

    #[test]
    fn price_crosses_buy_level_triggers() {
        // Price exactly at level OR below should trigger
        let config = GridConfig {
            spacing_pct: dec!(0.01),
            num_levels: 3,
            quantity_per_level: dec!(0.1),
            escape_multiplier: dec!(2.0),
        };
        let mut strategy = GridStrategy::new(config, dec!(100));

        // Price at 98.5 - below the 99 level
        let tick = make_tick("BTCUSDT", dec!(98.5), 1);
        let signal = strategy.on_tick(&tick);

        assert_eq!(signal, Signal::Buy { quantity: dec!(0.1) });
    }

    #[test]
    fn price_crosses_sell_level_triggers() {
        // Price exactly at level OR above should trigger
        let config = GridConfig {
            spacing_pct: dec!(0.01),
            num_levels: 3,
            quantity_per_level: dec!(0.1),
            escape_multiplier: dec!(2.0),
        };
        let mut strategy = GridStrategy::new(config, dec!(100));

        // Price at 101.5 - above the 101 level
        let tick = make_tick("BTCUSDT", dec!(101.5), 1);
        let signal = strategy.on_tick(&tick);

        assert_eq!(signal, Signal::Sell { quantity: dec!(0.1) });
    }
}

mod grid_reset {
    use super::*;

    #[test]
    fn resets_when_price_escapes_above_bounds() {
        // Grid 97-103 with 2x escape multiplier means escape above 105
        // (103 + 0.01 * 100 * 2 = 105)
        let config = GridConfig {
            spacing_pct: dec!(0.01),
            num_levels: 3,
            quantity_per_level: dec!(0.1),
            escape_multiplier: dec!(2.0),
        };
        let mut strategy = GridStrategy::new(config, dec!(100));

        // Fill first sell level
        let tick1 = make_tick("BTCUSDT", dec!(101), 1);
        strategy.on_tick(&tick1);

        // Price escapes above (105 is beyond threshold)
        let tick2 = make_tick("BTCUSDT", dec!(106), 2);
        let signal = strategy.on_tick(&tick2);

        // After reset, should hold (new center around 106, no levels crossed yet)
        assert_eq!(signal, Signal::Hold);

        // Verify grid was reset around new price
        let state = strategy.get_state();
        let center_price: Decimal = serde_json::from_value(state["center_price"].clone()).unwrap();
        assert_eq!(center_price, dec!(106));

        // All levels should be unfilled after reset
        let levels: Vec<GridLevel> = serde_json::from_value(state["levels"].clone()).unwrap();
        for level in &levels {
            assert!(
                !level.filled,
                "Level at {} should be unfilled after reset",
                level.price
            );
        }
    }

    #[test]
    fn resets_when_price_escapes_below_bounds() {
        // Grid 97-103 with 2x escape multiplier means escape below 95
        // (97 - 0.01 * 100 * 2 = 95)
        let config = GridConfig {
            spacing_pct: dec!(0.01),
            num_levels: 3,
            quantity_per_level: dec!(0.1),
            escape_multiplier: dec!(2.0),
        };
        let mut strategy = GridStrategy::new(config, dec!(100));

        // Price escapes below (94 is beyond threshold)
        let tick = make_tick("BTCUSDT", dec!(94), 1);
        let signal = strategy.on_tick(&tick);

        // After reset, should hold (new center around 94)
        assert_eq!(signal, Signal::Hold);

        // Verify grid was reset
        let state = strategy.get_state();
        let center_price: Decimal = serde_json::from_value(state["center_price"].clone()).unwrap();
        assert_eq!(center_price, dec!(94));
    }

    #[test]
    fn reset_creates_new_levels_around_current_price() {
        let config = GridConfig {
            spacing_pct: dec!(0.01),
            num_levels: 3,
            quantity_per_level: dec!(0.1),
            escape_multiplier: dec!(2.0),
        };
        let mut strategy = GridStrategy::new(config, dec!(100));

        // Force escape and reset
        let tick = make_tick("BTCUSDT", dec!(110), 1);
        strategy.on_tick(&tick);

        let state = strategy.get_state();
        let levels: Vec<GridLevel> = serde_json::from_value(state["levels"].clone()).unwrap();

        // New buy levels should be around 110: 108.9, 107.8, 106.7
        let buy_levels: Vec<&GridLevel> = levels
            .iter()
            .filter(|l| l.side == OrderSide::Buy)
            .collect();

        // First buy level should be 110 * (1 - 0.01) = 108.9
        assert_eq!(buy_levels[0].price, dec!(108.9));

        // New sell levels should be around 110: 111.1, 112.2, 113.3
        let sell_levels: Vec<&GridLevel> = levels
            .iter()
            .filter(|l| l.side == OrderSide::Sell)
            .collect();

        // First sell level should be 110 * (1 + 0.01) = 111.1
        assert_eq!(sell_levels[0].price, dec!(111.1));
    }
}

mod state_inspection {
    use super::*;

    #[test]
    fn get_state_returns_center_price() {
        let config = GridConfig {
            spacing_pct: dec!(0.01),
            num_levels: 3,
            quantity_per_level: dec!(0.1),
            escape_multiplier: dec!(2.0),
        };
        let strategy = GridStrategy::new(config, dec!(100));

        let state = strategy.get_state();

        let center_price: Decimal = serde_json::from_value(state["center_price"].clone()).unwrap();
        assert_eq!(center_price, dec!(100));
    }

    #[test]
    fn get_state_returns_levels_with_filled_status() {
        let config = GridConfig {
            spacing_pct: dec!(0.01),
            num_levels: 3,
            quantity_per_level: dec!(0.1),
            escape_multiplier: dec!(2.0),
        };
        let mut strategy = GridStrategy::new(config, dec!(100));

        // Trigger one level
        let tick = make_tick("BTCUSDT", dec!(99), 1);
        strategy.on_tick(&tick);

        let state = strategy.get_state();
        let levels: Vec<GridLevel> = serde_json::from_value(state["levels"].clone()).unwrap();

        // Find the 99 level and verify it's filled
        let level_99 = levels.iter().find(|l| l.price == dec!(99)).unwrap();
        assert!(level_99.filled);
    }

    #[test]
    fn get_state_returns_bounds() {
        let config = GridConfig {
            spacing_pct: dec!(0.01),
            num_levels: 3,
            quantity_per_level: dec!(0.1),
            escape_multiplier: dec!(2.0),
        };
        let strategy = GridStrategy::new(config, dec!(100));

        let state = strategy.get_state();

        let lower_bound: Decimal = serde_json::from_value(state["lower_bound"].clone()).unwrap();
        let upper_bound: Decimal = serde_json::from_value(state["upper_bound"].clone()).unwrap();

        // Lower bound should be lowest buy level
        assert_eq!(lower_bound, dec!(97));
        // Upper bound should be highest sell level
        assert_eq!(upper_bound, dec!(103));
    }
}

mod config_update {
    use super::*;

    #[test]
    fn update_config_changes_spacing() {
        let config = GridConfig {
            spacing_pct: dec!(0.01),
            num_levels: 3,
            quantity_per_level: dec!(0.1),
            escape_multiplier: dec!(2.0),
        };
        let mut strategy = GridStrategy::new(config, dec!(100));

        // Update to 2% spacing
        let new_config = json!({
            "spacing_pct": "0.02",
            "num_levels": 3,
            "quantity_per_level": "0.1",
            "escape_multiplier": "2.0"
        });

        let result = strategy.update_config(&new_config);
        assert!(result.is_ok());

        // Verify new spacing applied - first buy level should be 98 (not 99)
        let state = strategy.get_state();
        let levels: Vec<GridLevel> = serde_json::from_value(state["levels"].clone()).unwrap();
        let buy_levels: Vec<&GridLevel> = levels
            .iter()
            .filter(|l| l.side == OrderSide::Buy)
            .collect();

        assert_eq!(buy_levels[0].price, dec!(98));
    }

    #[test]
    fn update_config_changes_num_levels() {
        let config = GridConfig {
            spacing_pct: dec!(0.01),
            num_levels: 3,
            quantity_per_level: dec!(0.1),
            escape_multiplier: dec!(2.0),
        };
        let mut strategy = GridStrategy::new(config, dec!(100));

        // Update to 5 levels
        let new_config = json!({
            "spacing_pct": "0.01",
            "num_levels": 5,
            "quantity_per_level": "0.1",
            "escape_multiplier": "2.0"
        });

        let result = strategy.update_config(&new_config);
        assert!(result.is_ok());

        // Should now have 10 levels total (5 buy, 5 sell)
        let state = strategy.get_state();
        let levels: Vec<GridLevel> = serde_json::from_value(state["levels"].clone()).unwrap();
        assert_eq!(levels.len(), 10);
    }

    #[test]
    fn update_config_regenerates_grid_unfilled() {
        let config = GridConfig {
            spacing_pct: dec!(0.01),
            num_levels: 3,
            quantity_per_level: dec!(0.1),
            escape_multiplier: dec!(2.0),
        };
        let mut strategy = GridStrategy::new(config, dec!(100));

        // Fill a level
        let tick = make_tick("BTCUSDT", dec!(99), 1);
        strategy.on_tick(&tick);

        // Update config (any change triggers regeneration)
        let new_config = json!({
            "spacing_pct": "0.01",
            "num_levels": 3,
            "quantity_per_level": "0.2",
            "escape_multiplier": "2.0"
        });

        let result = strategy.update_config(&new_config);
        assert!(result.is_ok());

        // All levels should be unfilled after config update
        let state = strategy.get_state();
        let levels: Vec<GridLevel> = serde_json::from_value(state["levels"].clone()).unwrap();
        for level in &levels {
            assert!(!level.filled, "Level should be unfilled after config update");
        }
    }

    #[test]
    fn update_config_invalid_returns_error() {
        let config = GridConfig {
            spacing_pct: dec!(0.01),
            num_levels: 3,
            quantity_per_level: dec!(0.1),
            escape_multiplier: dec!(2.0),
        };
        let mut strategy = GridStrategy::new(config, dec!(100));

        // Missing required fields
        let invalid_config = json!({
            "spacing_pct": "0.01"
        });

        let result = strategy.update_config(&invalid_config);
        assert!(result.is_err());
    }
}

mod strategy_trait {
    use super::*;

    #[test]
    fn name_returns_grid() {
        let config = GridConfig {
            spacing_pct: dec!(0.01),
            num_levels: 3,
            quantity_per_level: dec!(0.1),
            escape_multiplier: dec!(2.0),
        };
        let strategy = GridStrategy::new(config, dec!(100));

        assert_eq!(strategy.name(), "grid");
    }

    #[test]
    fn implements_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<GridStrategy>();
    }
}
