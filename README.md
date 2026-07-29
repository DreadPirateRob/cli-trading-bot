# Trading Bot

A Rust-based cryptocurrency trading bot for Binance.US spot markets with configurable strategies, comprehensive risk controls, and a React web dashboard.

## Features

- **Exchange Connectivity**: Real-time WebSocket connection to Binance.US with automatic reconnection
- **Trading Strategies**: RSI-StdDev and Grid trading strategies with pluggable architecture
- **Risk Management**: Position limits, stop-loss, daily loss limits, and circuit breakers
- **Paper Trading**: Full simulation mode for safe testing without real money
- **Backtesting**: Historical replay with fee/slippage simulation and performance metrics
- **Web Dashboard**: Real-time charts, indicators, and bot controls
- **CLI**: Command-line interface for bot management

## Prerequisites

- **Rust**: 1.75+ ([install](https://rustup.rs/))
- **PostgreSQL**: 16+ (local or Docker)
- **Node.js**: 18+ (for dashboard)
- **Docker** (optional): For containerized deployment

## Quick Start

### 1. Clone the Repository

```bash
git clone <repo-url>
cd trading-bot
```

### 2. Set Up PostgreSQL

**Option A: Docker (recommended)**
```bash
docker run -d --name trading-postgres \
  -p 5432:5432 \
  -e POSTGRES_USER=trading \
  -e POSTGRES_PASSWORD=trading \
  -e POSTGRES_DB=trading_bot \
  postgres:17-alpine
```

**Option B: Local PostgreSQL**
```bash
createdb trading_bot
createuser trading -P  # Set password when prompted
```

### 3. Configure the Bot

Copy example configs:
```bash
cp config/database.example.yaml config/database.yaml
cp config/exchange.example.yaml config/exchange.yaml
cp config/strategy.example.yaml config/strategy.yaml
cp config/logging.example.yaml config/logging.yaml
```

Edit `config/database.yaml` with your database credentials:
```yaml
database:
  url: "postgres://trading:trading@localhost:5432/trading_bot"
```

### 4. Run Database Migrations

```bash
# Install sqlx-cli if not already installed
cargo install sqlx-cli --no-default-features --features postgres

# Run migrations
sqlx migrate run
```

### 5. Build and Run

```bash
# Build the bot
cargo build --release

# Start the bot (paper trading mode by default)
cargo run -- start
```

### 6. Start the Dashboard (Optional)

```bash
cd dashboard
npm install
npm run dev
```

Open http://localhost:5173 in your browser.

## Configuration

### Environment Variables

For production, use environment variables instead of config files:

```bash
# Database
export APP__DATABASE__URL="postgres://user:pass@host:5432/db"

# Exchange credentials (only needed for live trading)
export BINANCE_API_KEY="your_api_key"
export BINANCE_API_SECRET="your_api_secret"

# Paper trading mode (default: true)
export APP__PAPER_TRADING__ENABLED=true

# Logging level
export RUST_LOG=info
```

### Strategy Configuration

Edit `config/strategy.yaml`:

```yaml
strategy:
  active: rsi_stddev  # or "grid"

  risk:
    max_position_size_pct: 0.1    # 10% max position
    stop_loss_pct: 0.02           # 2% stop loss
    daily_loss_limit_pct: 0.05    # 5% daily loss limit
    max_drawdown_pct: 0.15        # 15% circuit breaker

  rsi_stddev:
    rsi_period: 14
    rsi_oversold: 30
    rsi_overbought: 70
    stddev_multiplier: 2.0

  grid:
    levels: 10
    spacing_pct: 0.01
```

## CLI Commands

```bash
# Start the bot
trading-bot start

# Stop the bot
trading-bot stop

# Check status
trading-bot status
trading-bot status --format json

# Validate configuration
trading-bot check-config

# Manual trades (testing)
trading-bot buy BTCUSDT 0.001
trading-bot sell BTCUSDT 0.001

# Initialize config files
trading-bot-init
```

## Docker Deployment

### Using Docker Compose

```bash
# Create .env file
cat > docker/.env << EOF
DB_PASSWORD=your_secure_password
BINANCE_API_KEY=your_api_key
BINANCE_API_SECRET=your_api_secret
PAPER_MODE=true
RUST_LOG=info
EOF

# Start all services
docker compose -f docker/docker-compose.yml up -d

# View logs
docker compose -f docker/docker-compose.yml logs -f trading-bot

# Stop services
docker compose -f docker/docker-compose.yml down
```

### Build Docker Image

```bash
docker build -f docker/Dockerfile -t trading-bot .
```

## Project Structure

```
trading-bot/
├── src/
│   ├── main.rs           # CLI entry point
│   ├── lib.rs            # Library exports
│   ├── config/           # Configuration loading
│   ├── exchange/         # Binance.US connectivity
│   ├── data/             # Database operations
│   ├── risk/             # Risk management
│   ├── strategy/         # Trading strategies
│   ├── execution/        # Order execution
│   ├── backtest/         # Backtesting engine
│   ├── cli/              # CLI commands
│   └── api/              # REST API & WebSocket
├── dashboard/            # React web dashboard
│   ├── src/
│   │   ├── components/   # UI components
│   │   ├── stores/       # Zustand state
│   │   └── hooks/        # React hooks
├── config/               # YAML configuration
├── migrations/           # SQL migrations
├── docker/               # Docker files
└── tests/                # Integration tests
```

## Web Dashboard

The dashboard provides:

- **Price Chart**: Candlestick chart with trade markers
- **Equity Curve**: Portfolio value over time
- **Indicators**: RSI panel and Bollinger bands (toggleable)
- **Performance Metrics**: Win rate, Sharpe ratio, drawdown
- **Bot Controls**: Start/stop, strategy selector, optimization trigger
- **Emergency Close**: One-click position close with confirmation

### Running the Dashboard

**Development mode:**
```bash
cd dashboard
npm install
npm run dev
```

**Production build:**
```bash
cd dashboard
npm run build
# Serve dist/ with any static file server
```

The dashboard connects to the backend API on port 3000 by default.

## API Endpoints

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/api/status` | GET | Bot status and uptime |
| `/api/metrics` | GET | Trading metrics |
| `/api/trades` | GET | Recent trades |
| `/api/equity-curve` | GET | Equity history |
| `/api/indicators` | GET | RSI and Bollinger data |
| `/api/strategies` | GET | Available strategies |
| `/api/start` | POST | Start the bot |
| `/api/stop` | POST | Stop the bot |
| `/api/strategy` | POST | Switch strategy |
| `/api/optimize` | POST | Trigger optimization |
| `/api/positions/close-all` | POST | Emergency close |
| `/api/ws` | WS | Real-time updates |

## Testing

```bash
# Run unit tests
cargo test

# Run integration tests (requires database)
cargo test --test '*' -- --test-threads=1

# Run with coverage
cargo tarpaulin
```

## Safety Features

1. **Paper Trading Default**: Bot starts in paper trading mode by default
2. **Position Limits**: Maximum position size as percentage of portfolio
3. **Stop-Loss**: Automatic stop-loss on every position
4. **Daily Loss Limit**: Halts trading when daily loss threshold is reached
5. **Circuit Breaker**: Pauses bot when drawdown exceeds threshold
6. **Emergency Close**: One-click close all positions from dashboard

## License

MIT

## Disclaimer

This software is for educational purposes only. Cryptocurrency trading involves substantial risk of loss. Past performance does not guarantee future results. Always test thoroughly in paper trading mode before using real funds.
