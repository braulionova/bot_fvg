# Fair Value Gap (FVG) Trading Strategy
## Bybit WebSocket Automation with Rust - HyroTrader Compliant

**Author:** Professional Trader | **Updated:** February 2026 | **Framework:** Production-Grade Risk Management

---

capital 10,000 usdt


## TABLE OF CONTENTS
1. [Executive Summary](#executive-summary)
2. [Strategy Overview](#strategy-overview)
3. [HyroTrader Compliance Rules](#hyrotrader-compliance-rules)
4. [Fair Value Gap Theory](#fair-value-gap-theory)
5. [Entry Rules](#entry-rules)
6. [Exit Rules](#exit-rules)
7. [Risk Management Framework](#risk-management-framework)
8. [Rust Implementation Guide](#rust-implementation-guide)
9. [WebSocket Architecture](#websocket-architecture)
10. [Backtesting & Live Trading](#backtesting--live-trading)

---

## EXECUTIVE SUMMARY

This document presents a professional Fair Value Gap (FVG) detection strategy designed for Bybit perpetual futures trading, fully automated via Rust and WebSocket connections. The strategy is engineered to comply with HyroTrader's strict risk management parameters while maintaining profitability through consistent, rule-based execution.

**Key Strategy Metrics:**
- **Win Rate Target:** 55-65%
- **Risk-Reward Ratio:** 1:2 minimum (1:3 optimal)
- **Daily Max Drawdown:** 5% (HyroTrader 2-Step) / 4% (HyroTrader 1-Step)
- **Max Risk Per Trade:** 3% of initial account balance
- **Profit Distribution:** Max 40% from any single trading day
- **Minimum Stop-Loss Setup:** Within 5 minutes of trade entry
- **Account Equity Minimum:** 90% (2-Step) / 94% (1-Step)

---

## STRATEGY OVERVIEW

### What is Fair Value Gap (FVG)?

A Fair Value Gap is a price imbalance where the market leaves a zone of untouched price levels during rapid directional movement. These gaps represent inefficiency and typically act as magnets that price returns to fill. This strategy exploits mean reversion within the gaps.

**Types of FVG:**
1. **Bullish FVG** – Created during upward sweeps; resistance becomes support after pullback
2. **Bearish FVG** – Created during downward sweeps; support becomes resistance after recovery
3. **Advanced Mitigation Block** – FVG that has been partially filled (traders take profits here)

### Core Concept

Price moves in directional impulses, leaving behind unbalanced price zones. When price rapidly moves from point A to point B without closing candles in between, a gap forms. This gap is the FVG. Market structure theory states that price must eventually return to fill these inefficiencies.

**Why FVG Works:**
- Automated market makers and algorithms hunt these zones
- Retail traders cluster stop-losses beyond FVGs
- Institutional rebalancing drives price back to fill gaps
- Supply/demand imbalances create predictable reversions

---

## HYROTRADER COMPLIANCE RULES

### Hard Rules (Account Termination if Violated)

These rules are non-negotiable and automatically monitored. Any breach terminates your account immediately.

| Rule | Limit | Consequence |
|------|-------|-------------|
| **Daily Drawdown** | 5% (2-Step) / 4% (1-Step) | Account FAILED |
| **Maximum Loss** | 10% (2-Step) / 6% (1-Step) | Account FAILED |
| **Stop-Loss Obligation** | Must set within 5 minutes | Account FAILED after 2nd offense |
| **Account Equity Floor** | 90% (2-Step) / 94% (1-Step) | Account FAILED |
| **Max Risk Per Trade** | 3% of initial balance | Position closed + warning |
| **Profit Distribution** | Max 40% from one day | Account FAILED |
| **Minimum Trading Days** | 10 days during challenge | Challenge not complete |

### Soft Rules (One-Time Grace Period)

If you violate these once, you receive an email alert with 1 hour to correct.

- **Missing Stop-Loss:** Set stop-loss within 1 hour or account fails
- **Cancel vs. Edit:** Never cancel a stop-loss; always edit/adjust it

### Position Management Rules

- **Maximum Exposure:** Up to 25% of initial balance (funded accounts only)
- **Cumulative Position Limit:** Total open trade value ≤ 2× initial balance
- **No Spot Trading:** Only perpetual futures allowed
- **No High-Leverage Altcoins:** Cannot risk >5% on low-cap coins

### Strategy Alignment

Your FVG strategy **MUST:**
1. Always place stop-loss within 5 minutes of entry
2. Risk maximum 3% per trade
3. Never exceed 5% daily drawdown
4. Maintain account equity ≥90%
5. Keep position risk within daily drawdown budget
6. Distribute profits across multiple days (no 40%+ single-day reliance)

---

## FAIR VALUE GAP THEORY

### Identifying Fair Value Gaps

FVGs appear on all timeframes but work best on **4H-1D for swing trading** (fits HyroTrader's holding period requirements).

**Bullish FVG Formation:**
```
Candle 1: Body at 10,000
Candle 2: Gap up, opens at 10,100, closes at 10,200 (HIGH candle)
FVG Zone: 10,001 – 10,099 (untouched price)
```

**Bearish FVG Formation:**
```
Candle 1: Body at 20,000
Candle 2: Gap down, opens at 19,900, closes at 19,800 (LOW candle)
FVG Zone: 19,801 – 19,899 (untouched price)
```

### FVG Validation Criteria

Not all gaps are tradeable. Use these filters:

1. **Minimum Gap Size:** At least 0.5% of current price (filters noise)
2. **Volume Confirmation:** Impulse candle shows 120%+ of 20-candle average volume
3. **Wick Retrace:** Price retraces to within 25% of the gap zone (shows intent)
4. **Trend Alignment:** FVG forms in direction of primary trend (4H+)
5. **No Liquidity Wicks:** Gap not already partially filled by wicks

### FVG Breakout Pattern

**Setup Trigger:**
1. FVG identified within last 5 candles
2. Price retraces 25-75% toward gap
3. New impulse candle forms (breakout candle)
4. Breakout candle closes beyond FVG with volume spike

**Entry Signal:** Breakout candle close beyond FVG boundary + volume confirmation

### Advanced Mitigation Block (AMB)

An advanced mitigation block occurs when:
- FVG is partially filled but not completely
- Price reverses before full fill
- This level becomes a secondary entry zone (lower probability)

**Rule:** Only trade AMB if it's the 1st partial fill and price hasn't retested 3+ times.

---

## ENTRY RULES

### Rule 1: Timeframe & Market Selection

- **Primary TF:** 4H candles (1H for confirmation)
- **Pairs:** BTC/USDT, ETH/USDT, SOL/USDT (high liquidity, lower slippage)
- **Avoid:** Low-cap altcoins, coins with <5% of initial balance risk
- **Market Conditions:** Only trade when 4H ATR > 200 (sufficient volatility)

### Rule 2: FVG Identification (Automated via Rust)

**Bullish FVG Detection:**
```
IF (candle[n].close > candle[n-1].close) 
   AND (candle[n].open > candle[n-1].high)
   AND (candle[n].high - candle[n-1].high > 0.005 * price)
   THEN FVG ZONE = [candle[n-1].high : candle[n].open]
```

**Bearish FVG Detection:**
```
IF (candle[n].close < candle[n-1].close)
   AND (candle[n].open < candle[n-1].low)
   AND (candle[n-1].low - candle[n].low > 0.005 * price)
   THEN FVG ZONE = [candle[n].open : candle[n-1].low]
```

### Rule 3: Confirmation (Structure Break)

**For Bullish FVG Entry:**
1. FVG identified
2. Price pulls back to 25-75% retracement of gap
3. NEW candle closes ABOVE the gap's lower boundary
4. Volume on breakout candle > 1.2× 20-candle average
5. Entry signal: BUY market order on next candle open

**For Bearish FVG Entry:**
1. FVG identified
2. Price rallies to 25-75% retracement of gap
3. NEW candle closes BELOW the gap's upper boundary
4. Volume on breakout candle > 1.2× 20-candle average
5. Entry signal: SELL market order on next candle open

### Rule 4: Entry Execution

**Mandatory Steps:**
1. **Place entry order** (market or limit at support/resistance)
2. **IMMEDIATELY** calculate stop-loss level
3. **SET stop-loss within 5 minutes** (HyroTrader requirement)
4. **Verify risk** = (entry - stop) × quantity = 3% max of account
5. **Place take-profit** at 2:1 ratio minimum
6. **NO position sizing flexibility** – exactly 3% risk or smaller

**Entry Order Types:**
- **Market Entry:** Best for fast breakouts; enters immediately
- **Limit Entry:** 0.1-0.3% beyond gap boundary (reduces slippage on slow moves)

---

## EXIT RULES

### Rule 1: Take-Profit Targets (Systematic Exits)

**Primary Target (70% of position):**
- Distance: FVG zone width × 2 (mean reversion target)
- Example: FVG size = 100 pips → TP = entry + 200 pips
- Execution: Sell 70% of position at this level

**Secondary Target (20% of position):**
- Distance: FVG zone width × 3.5 (extended fill)
- Execution: Sell 20% of position; let 10% run

**Trailing Stop (10% of position):**
- Place trailing stop: 0.5× ATR below price
- Objective: Capture extended momentum if breakout continues
- Max hold time: 7 candles (HyroTrader daily reset risk)

### Rule 2: Stop-Loss Placement (Mandatory)

**Bullish FVG Long:**
- **Primary SL:** 1 ATR below FVG lower boundary
- **Hard SL (safety):** 1.5 ATR below entry (3% max risk trigger)

**Bearish FVG Short:**
- **Primary SL:** 1 ATR above FVG upper boundary
- **Hard SL (safety):** 1.5 ATR above entry (3% max risk trigger)

**Logic:** Stop below an FVG should be far enough to allow price to fill the gap but close enough to respect 3% max risk. Use ATR to scale stop placement to volatility.

### Rule 3: Mandatory Exit Conditions

**Exit immediately if ANY occur:**

1. **Stop-Loss Breach:** Hard stop-loss hit = position closed (no exceptions)
2. **Daily Drawdown Breach:** If daily loss reaches 5%, stop ALL trading for the day
3. **Account Equity Floor:** If equity < 90%, close ALL positions
4. **Time Stop:** 7 candles (28 hours on 4H) without fill = exit at market
5. **Invalidation:** Price closes > 1 ATR beyond gate without directional follow-through = exit
6. **News Risk:** Major economic events within 2 hours = exit all open positions
7. **Drawdown Approaching:** If daily loss > 3%, reduce position size 50% for remaining trades

### Rule 4: Profit-Taking Discipline

**This is critical for HyroTrader's 40% profit distribution rule.**

- **Never take all profits in one trade.** If you're +8% in account, and one trade is +5%, you're already at 62.5% of one day's limit. Exit 90% of position.
- **Spread wins across multiple days.** If you hit profit target on day 1, reduce position size 30% on day 2.
- **Log every day's PnL.** Track which days contributed to total profit. No single day should exceed 40%.

---

## RISK MANAGEMENT FRAMEWORK

### Position Sizing Formula

```
Account Balance: $10,000
Max Risk Per Trade: 3% = $300
Entry Price: $30,000 (BTC)
Stop-Loss: $29,900 (100 point stop)
Risk Per Futures Contract: 1 contract = $100 loss per $100 move

Position Size = Max Risk / Risk Per Unit
Position Size = $300 / $100 = 3 contracts
```

**Leverage Applied:**
- For $300 risk, at 10:1 leverage, you control $3,000 notional exposure
- This is within HyroTrader's 25% max exposure rule ($2,500 on $10K)

### Daily Drawdown Tracking

**HyroTrader calculates drawdown as:**
```
Daily Drawdown = Starting Equity – (Starting Equity – Daily Loss)
For 5% limit: Maximum loss = 5% × $10,000 = $500/day
```

**Your responsibility:**
- Track equity every hour
- If cumulative daily loss > $250 (2.5%), reduce next position size by 50%
- If cumulative daily loss > $400 (4%), stop trading for rest of day
- If cumulative daily loss hits $500, account is FAILED

**Implementation:**
```rust
let max_daily_drawdown = account_balance * 0.05;
let current_drawdown = starting_daily_equity - current_equity;
if current_drawdown > max_daily_drawdown * 0.8 {
    // Reduce position size to 50%
    position_size *= 0.5;
}
if current_drawdown >= max_daily_drawdown {
    // Stop trading for the day
    trading_disabled = true;
}
```

### Account Equity Monitoring

**Minimum Equity Rule:**

| Challenge Type | Minimum Equity |
|---|---|
| 2-Step | 90% of initial |
| 1-Step | 94% of initial |

**Calculation:**
```
Current Equity = Initial Balance + Current Profit/Loss
If Current Equity < (Initial × 0.90), account terminates
```

**Action:** Before every trade, verify:
```
remaining_buffer = current_equity - (initial_balance × 0.90)
trade_risk = 3% of initial balance
if trade_risk > remaining_buffer {
    // REDUCE position size or skip trade
    position_size *= (remaining_buffer / trade_risk)
}
```

### Win-Rate & Consistency Tracking

Track these metrics daily:

| Metric | Target | Action if Missed |
|---|---|---|
| Win Rate | 55%+ | Tighten entry criteria |
| Avg Win | ≥ 2× Avg Loss | Review take-profit levels |
| Max Consecutive Losses | 3 | Stop trading for 4 hours |
| Profit Per Trading Day | Consistent (< ±$500 variance) | Reduce leverage |

---

## RUST IMPLEMENTATION GUIDE

### Architecture Overview

```
┌─────────────────────┐
│  Bybit WebSocket    │
│  (Real-time prices) │
└──────────┬──────────┘
           │
           ▼
┌─────────────────────┐
│  Data Aggregator    │
│  (OHLCV buffers)    │
└──────────┬──────────┘
           │
           ▼
┌─────────────────────┐
│  FVG Detector       │
│  (Gap identification)
└──────────┬──────────┘
           │
           ▼
┌─────────────────────┐
│  Signal Generator   │
│  (Entry/exit logic) │
└──────────┬──────────┘
           │
           ▼
┌─────────────────────┐
│  Position Manager   │
│  (Risk mgmt + exec) │
└──────────┬──────────┘
           │
           ▼
┌─────────────────────┐
│  Bybit API Orders   │
│  (Execute trades)   │
└─────────────────────┘
```

### Dependencies

```toml
[package]
name = "fvg_trader"
version = "1.0.0"
edition = "2021"

[dependencies]
tokio = { version = "1.40", features = ["full"] }
tokio-tungstenite = "0.23"
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
chrono = { version = "0.4", features = ["serde"] }
env_logger = "0.11"
log = "0.4"
reqwest = { version = "0.11", features = ["json"] }
hmac = "0.12"
sha2 = "0.10"
hex = "0.4"

[[bin]]
name = "fvg_trader"
path = "src/main.rs"
```

### Core Data Structures

```rust
use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Candle {
    pub timestamp: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
}

#[derive(Clone, Debug)]
pub struct FVGZone {
    pub fvg_type: FVGType,  // Bullish or Bearish
    pub zone_high: f64,
    pub zone_low: f64,
    pub impulse_high: f64,
    pub impulse_low: f64,
    pub created_timestamp: i64,
    pub is_filled: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum FVGType {
    Bullish,
    Bearish,
}

#[derive(Clone, Debug)]
pub struct TradeSignal {
    pub signal_type: SignalType,
    pub fvg_zone: FVGZone,
    pub entry_price: f64,
    pub stop_loss: f64,
    pub take_profit_1: f64,
    pub take_profit_2: f64,
    pub position_size: f64,
    pub risk_amount: f64,
    pub risk_reward_ratio: f64,
    pub timestamp: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SignalType {
    BuyBreakout,
    SellBreakout,
    Exit,
}

#[derive(Clone, Debug)]
pub struct PositionData {
    pub is_open: bool,
    pub entry_price: f64,
    pub entry_time: i64,
    pub position_size: f64,
    pub stop_loss: f64,
    pub take_profit_1: f64,
    pub take_profit_2: f64,
    pub unrealized_pnl: f64,
    pub risk_amount: f64,
    pub max_favorable_excursion: f64,
}

#[derive(Clone, Debug)]
pub struct RiskMetrics {
    pub account_balance: f64,
    pub current_equity: f64,
    pub daily_pnl: f64,
    pub max_daily_loss: f64,
    pub drawdown_percentage: f64,
    pub max_risk_per_trade: f64,
    pub trading_enabled: bool,
    pub trades_today: u32,
    pub wins_today: u32,
}
```

### FVG Detection Algorithm

```rust
pub mod fvg_detector {
    use crate::{Candle, FVGZone, FVGType};

    const MIN_GAP_PERCENTAGE: f64 = 0.005;  // 0.5% minimum gap
    const MIN_VOLUME_MULTIPLIER: f64 = 1.2; // 120% of average

    pub fn detect_bullish_fvg(
        candles: &[Candle],
        atr: f64,
    ) -> Option<FVGZone> {
        if candles.len() < 3 {
            return None;
        }

        let prev = &candles[candles.len() - 2];
        let curr = &candles[candles.len() - 1];

        // Bullish impulse: close > open and significant upward gap
        if curr.close > curr.open 
            && curr.open > prev.high 
            && (curr.high - prev.high) > (prev.close * MIN_GAP_PERCENTAGE) {
            
            let gap_size = curr.open - prev.high;
            
            // Validate volume
            let avg_volume = candles.iter()
                .rev()
                .take(20)
                .map(|c| c.volume)
                .sum::<f64>() / 20.0;
            
            if curr.volume > avg_volume * MIN_VOLUME_MULTIPLIER {
                return Some(FVGZone {
                    fvg_type: FVGType::Bullish,
                    zone_high: curr.open,
                    zone_low: prev.high,
                    impulse_high: curr.high,
                    impulse_low: curr.low,
                    created_timestamp: curr.timestamp,
                    is_filled: false,
                });
            }
        }

        None
    }

    pub fn detect_bearish_fvg(
        candles: &[Candle],
        atr: f64,
    ) -> Option<FVGZone> {
        if candles.len() < 3 {
            return None;
        }

        let prev = &candles[candles.len() - 2];
        let curr = &candles[candles.len() - 1];

        // Bearish impulse: close < open and significant downward gap
        if curr.close < curr.open 
            && curr.open < prev.low 
            && (prev.low - curr.low) > (prev.close * MIN_GAP_PERCENTAGE) {
            
            let gap_size = prev.low - curr.open;
            
            // Validate volume
            let avg_volume = candles.iter()
                .rev()
                .take(20)
                .map(|c| c.volume)
                .sum::<f64>() / 20.0;
            
            if curr.volume > avg_volume * MIN_VOLUME_MULTIPLIER {
                return Some(FVGZone {
                    fvg_type: FVGType::Bearish,
                    zone_high: prev.low,
                    zone_low: curr.open,
                    impulse_high: curr.high,
                    impulse_low: curr.low,
                    created_timestamp: curr.timestamp,
                    is_filled: false,
                });
            }
        }

        None
    }

    pub fn check_fvg_breakout(
        fvg: &FVGZone,
        current_candle: &Candle,
        avg_volume: f64,
    ) -> bool {
        match fvg.fvg_type {
            FVGType::Bullish => {
                // Bullish FVG breakout: close above zone_high
                current_candle.close > fvg.zone_high 
                    && current_candle.volume > avg_volume * 1.2
            }
            FVGType::Bearish => {
                // Bearish FVG breakout: close below zone_low
                current_candle.close < fvg.zone_low 
                    && current_candle.volume > avg_volume * 1.2
            }
        }
    }

    pub fn check_fvg_filled(fvg: &FVGZone, current_price: f64) -> bool {
        match fvg.fvg_type {
            FVGType::Bullish => current_price < fvg.zone_low,
            FVGType::Bearish => current_price > fvg.zone_high,
        }
    }
}
```

### Position Management

```rust
pub mod position_manager {
    use crate::{TradeSignal, PositionData, RiskMetrics, FVGType};

    pub fn calculate_position_size(
        signal: &TradeSignal,
        metrics: &RiskMetrics,
    ) -> f64 {
        let max_risk = metrics.account_balance * 0.03;
        
        // Ensure we don't exceed remaining daily drawdown budget
        let remaining_daily_budget = (metrics.max_daily_loss - metrics.daily_pnl.abs()).max(0.0);
        let actual_max_risk = max_risk.min(remaining_daily_budget);
        
        // Position size = max risk / (entry - stop)
        let risk_per_unit = (signal.entry_price - signal.stop_loss).abs();
        
        if risk_per_unit > 0.0 {
            (actual_max_risk / risk_per_unit).floor()
        } else {
            0.0
        }
    }

    pub fn validate_trade(
        signal: &TradeSignal,
        metrics: &RiskMetrics,
    ) -> Result<(), String> {
        // Check if trading is enabled
        if !metrics.trading_enabled {
            return Err("Trading disabled due to daily loss limit".to_string());
        }

        // Check equity floor
        let min_equity = metrics.account_balance * 0.90;
        if metrics.current_equity < min_equity {
            return Err(format!("Equity below 90% floor: {}", metrics.current_equity));
        }

        // Check max risk per trade
        if signal.risk_amount > metrics.max_risk_per_trade {
            return Err(format!(
                "Trade risk {} exceeds max {}",
                signal.risk_amount, metrics.max_risk_per_trade
            ));
        }

        // Check daily drawdown buffer
        let remaining_daily = metrics.max_daily_loss - metrics.daily_pnl.abs();
        if signal.risk_amount > remaining_daily {
            return Err("Insufficient daily drawdown budget".to_string());
        }

        Ok(())
    }

    pub fn set_stop_loss(signal: &mut TradeSignal, atr: f64) {
        match signal.fvg_zone.fvg_type {
            FVGType::Bullish => {
                // SL below FVG lower boundary + 1 ATR
                signal.stop_loss = signal.fvg_zone.zone_low - atr;
                signal.risk_amount = (signal.entry_price - signal.stop_loss) 
                    * signal.position_size;
            }
            FVGType::Bearish => {
                // SL above FVG upper boundary + 1 ATR
                signal.stop_loss = signal.fvg_zone.zone_high + atr;
                signal.risk_amount = (signal.stop_loss - signal.entry_price) 
                    * signal.position_size;
            }
        }
    }

    pub fn calculate_take_profits(signal: &mut TradeSignal) {
        let gap_size = (signal.fvg_zone.zone_high - signal.fvg_zone.zone_low).abs();
        
        match signal.fvg_zone.fvg_type {
            FVGType::Bullish => {
                // TP1: 2× gap size above entry
                signal.take_profit_1 = signal.entry_price + (gap_size * 2.0);
                // TP2: 3.5× gap size above entry
                signal.take_profit_2 = signal.entry_price + (gap_size * 3.5);
            }
            FVGType::Bearish => {
                // TP1: 2× gap size below entry
                signal.take_profit_1 = signal.entry_price - (gap_size * 2.0);
                // TP2: 3.5× gap size below entry
                signal.take_profit_2 = signal.entry_price - (gap_size * 3.5);
            }
        }
        
        // Ensure TP is 2:1 minimum risk-reward
        let risk = (signal.entry_price - signal.stop_loss).abs();
        let min_reward = risk * 2.0;
        
        if signal.take_profit_1 - signal.entry_price < min_reward {
            signal.take_profit_1 = signal.entry_price + min_reward;
        }
    }

    pub fn update_position_pnl(
        position: &mut PositionData,
        current_price: f64,
    ) {
        position.unrealized_pnl = (current_price - position.entry_price) 
            * position.position_size;
        
        if current_price > position.max_favorable_excursion {
            position.max_favorable_excursion = current_price;
        }
    }
}
```

### WebSocket Connection & Data Feed

```rust
pub mod websocket_handler {
    use tokio_tungstenite::{connect_async, tungstenite::Message};
    use serde_json::json;
    use std::collections::VecDeque;
    use crate::Candle;

    pub struct BybitWsClient {
        url: String,
        candle_buffer: VecDeque<Candle>,
    }

    impl BybitWsClient {
        pub fn new(symbol: &str, interval: &str) -> Self {
            let url = format!(
                "wss://stream.bybit.com/v5/public/linear?symbol={}&interval={}",
                symbol, interval
            );
            BybitWsClient {
                url,
                candle_buffer: VecDeque::with_capacity(200),
            }
        }

        pub async fn connect(&self) -> Result<(), Box<dyn std::error::Error>> {
            let (ws_stream, _) = connect_async(&self.url).await?;
            log::info!("WebSocket connected: {}", &self.url);

            let (mut write, mut read) = ws_stream.split();

            // Subscribe to candle updates
            let subscribe_msg = json!({
                "op": "subscribe",
                "args": [format!("kline.4.{}", "BTCUSDT")]
            });

            write.send(Message::Text(subscribe_msg.to_string())).await?;

            // Read stream
            while let Some(msg) = tokio::stream::StreamExt::next(&mut read).await {
                match msg {
                    Ok(Message::Text(text)) => {
                        if let Ok(data) = serde_json::from_str::<serde_json::Value>(&text) {
                            if let Some(kline) = data["data"].as_array() {
                                for k in kline {
                                    let candle = self.parse_candle(k)?;
                                    self.candle_buffer.push_back(candle);
                                    
                                    if self.candle_buffer.len() > 200 {
                                        self.candle_buffer.pop_front();
                                    }
                                }
                            }
                        }
                    }
                    Ok(Message::Close(_)) => {
                        log::warn!("WebSocket closed");
                        break;
                    }
                    Err(e) => {
                        log::error!("WebSocket error: {}", e);
                        break;
                    }
                    _ => {}
                }
            }

            Ok(())
        }

        fn parse_candle(&self, data: &serde_json::Value) -> Result<Candle, Box<dyn std::error::Error>> {
            Ok(Candle {
                timestamp: data[0].as_i64().unwrap_or(0),
                open: data[1].as_str().unwrap_or("0").parse().unwrap_or(0.0),
                high: data[2].as_str().unwrap_or("0").parse().unwrap_or(0.0),
                low: data[3].as_str().unwrap_or("0").parse().unwrap_or(0.0),
                close: data[4].as_str().unwrap_or("0").parse().unwrap_or(0.0),
                volume: data[5].as_str().unwrap_or("0").parse().unwrap_or(0.0),
            })
        }

        pub fn get_candles(&self) -> Vec<Candle> {
            self.candle_buffer.iter().cloned().collect()
        }
    }
}
```

### Main Trading Loop

```rust
mod fvg_detector;
mod position_manager;
mod websocket_handler;

use crate::fvg_detector::*;
use crate::position_manager::*;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    let mut ws_client = websocket_handler::BybitWsClient::new("BTCUSDT", "4");
    let mut position: Option<PositionData> = None;
    
    let mut metrics = RiskMetrics {
        account_balance: 10000.0,
        current_equity: 10000.0,
        daily_pnl: 0.0,
        max_daily_loss: 500.0,  // 5% of $10,000
        drawdown_percentage: 0.0,
        max_risk_per_trade: 300.0,  // 3% of $10,000
        trading_enabled: true,
        trades_today: 0,
        wins_today: 0,
    };

    // Spawn WebSocket listener
    let client_handle = tokio::spawn(async move {
        if let Err(e) = ws_client.connect().await {
            log::error!("WebSocket error: {}", e);
        }
    });

    // Main trading loop (polling for simplicity)
    loop {
        let candles = vec![]; // Fetch from WebSocket buffer

        if candles.len() < 20 {
            tokio::time::sleep(tokio::time::Duration::from_secs(60)).await;
            continue;
        }

        // Calculate ATR (14 period)
        let atr = calculate_atr(&candles, 14);
        let current_price = candles.last().unwrap().close;

        // Check for FVG patterns
        let bullish_fvg = fvg_detector::detect_bullish_fvg(&candles, atr);
        let bearish_fvg = fvg_detector::detect_bearish_fvg(&candles, atr);

        // Handle open position
        if let Some(ref mut pos) = position {
            update_position_pnl(pos, current_price);
            metrics.current_equity = metrics.account_balance + pos.unrealized_pnl;
            metrics.daily_pnl += pos.unrealized_pnl;

            // Check exit conditions
            if current_price <= pos.stop_loss {
                log::info!("Stop-loss hit. Closing position.");
                close_position(&mut position, &mut metrics, current_price);
            } else if current_price >= pos.take_profit_1 {
                log::info!("TP1 hit. Closing 70% of position.");
                // Close 70% and update stop to breakeven
                pos.position_size *= 0.3;
            } else if (chrono::Utc::now().timestamp() - pos.entry_time) > 28 * 3600 {
                log::warn!("Time stop: 7 candles exceeded. Closing position.");
                close_position(&mut position, &mut metrics, current_price);
            }
        } else {
            // Check for new entry signals
            if let Some(bullish_fvg) = bullish_fvg {
                if fvg_detector::check_fvg_breakout(&bullish_fvg, &candles.last().unwrap(), 
                    candles.iter().rev().take(20).map(|c| c.volume).sum::<f64>() / 20.0) {
                    
                    let mut signal = TradeSignal {
                        signal_type: crate::SignalType::BuyBreakout,
                        fvg_zone: bullish_fvg,
                        entry_price: current_price,
                        stop_loss: 0.0,
                        take_profit_1: 0.0,
                        take_profit_2: 0.0,
                        position_size: 0.0,
                        risk_amount: 0.0,
                        risk_reward_ratio: 0.0,
                        timestamp: chrono::Utc::now().timestamp(),
                    };

                    position_manager::set_stop_loss(&mut signal, atr);
                    position_manager::calculate_take_profits(&mut signal);
                    signal.position_size = position_manager::calculate_position_size(&signal, &metrics);

                    if let Err(e) = position_manager::validate_trade(&signal, &metrics) {
                        log::warn!("Trade validation failed: {}", e);
                    } else {
                        log::info!("ENTRY: BUY {} contracts @ {} SL: {} TP1: {}", 
                            signal.position_size, signal.entry_price, 
                            signal.stop_loss, signal.take_profit_1);
                        position = Some(create_position_from_signal(&signal));
                    }
                }
            }
        }

        // Check daily reset (HyroTrader UTC midnight)
        if is_daily_reset_time() {
            metrics.daily_pnl = 0.0;
            metrics.trades_today = 0;
            metrics.wins_today = 0;
            if metrics.current_equity < metrics.account_balance * 0.90 {
                metrics.trading_enabled = false;
            }
        }

        tokio::time::sleep(tokio::time::Duration::from_secs(60)).await;
    }
}

fn calculate_atr(candles: &[Candle], period: usize) -> f64 {
    if candles.len() < period {
        return 0.0;
    }

    let mut tr_sum = 0.0;
    for i in 1..period {
        let curr = &candles[i];
        let prev = &candles[i - 1];
        
        let tr = (curr.high - curr.low)
            .max((curr.high - prev.close).abs())
            .max((curr.low - prev.close).abs());
        
        tr_sum += tr;
    }

    tr_sum / period as f64
}

fn close_position(
    position: &mut Option<PositionData>,
    metrics: &mut RiskMetrics,
    current_price: f64,
) {
    if let Some(pos) = position.take() {
        let exit_pnl = (current_price - pos.entry_price) * pos.position_size;
        metrics.current_equity += exit_pnl;
        metrics.daily_pnl += exit_pnl;
        
        if exit_pnl > 0.0 {
            metrics.wins_today += 1;
        }
        
        log::info!("Position closed. Exit PnL: {}", exit_pnl);
    }
}

fn create_position_from_signal(signal: &TradeSignal) -> PositionData {
    PositionData {
        is_open: true,
        entry_price: signal.entry_price,
        entry_time: signal.timestamp,
        position_size: signal.position_size,
        stop_loss: signal.stop_loss,
        take_profit_1: signal.take_profit_1,
        take_profit_2: signal.take_profit_2,
        unrealized_pnl: 0.0,
        risk_amount: signal.risk_amount,
        max_favorable_excursion: signal.entry_price,
    }
}

fn is_daily_reset_time() -> bool {
    let now = chrono::Utc::now();
    now.hour() == 0 && now.minute() == 0
}
```

---

## WEBSOCKET ARCHITECTURE

### Connection Strategy

**Bybit WebSocket Endpoint:** `wss://stream.bybit.com/v5/public/linear`

**Subscription Message:**
```json
{
  "op": "subscribe",
  "args": [
    "kline.4.BTCUSDT",
    "kline.4.ETHUSDT",
    "tickers.BTCUSDT"
  ]
}
```

**Data Update Frequency:** Every 4-hour candle close (15-minute granularity internally)

### Error Handling & Reconnection

```rust
pub async fn reconnect_with_backoff(
    max_retries: u32,
    initial_delay: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut retries = 0;
    let mut delay = initial_delay;

    loop {
        match tokio_tungstenite::connect_async(WS_URL).await {
            Ok((ws, _)) => {
                log::info!("Connected to Bybit WebSocket");
                return Ok(());
            }
            Err(e) => {
                retries += 1;
                if retries >= max_retries {
                    return Err(format!("Failed after {} retries: {}", retries, e).into());
                }
                log::warn!("Connection failed. Retrying in {}s... ({}/{})", 
                    delay, retries, max_retries);
                tokio::time::sleep(tokio::time::Duration::from_secs(delay)).await;
                delay = (delay * 2).min(300);  // Max 5 minute backoff
            }
        }
    }
}
```

### Data Validation

Before processing candle data:

```rust
fn validate_candle_data(candle: &Candle) -> bool {
    candle.open > 0.0
        && candle.high >= candle.low
        && candle.high >= candle.open
        && candle.high >= candle.close
        && candle.low <= candle.open
        && candle.low <= candle.close
        && candle.volume >= 0.0
}
```

---

## BACKTESTING & LIVE TRADING

### Backtesting Strategy

Before deploying live, backtest on these datasets:

1. **Historical Data:** Last 1 year of 4H candles (≈2,000 candles)
2. **Different Market Regimes:**
   - Bull trend (50% of data)
   - Bear trend (30% of data)
   - Sideways (20% of data)

**Metrics to Track:**
- Win rate (target: 55%+)
- Average win vs. average loss (target: 2:1+)
- Max consecutive losses (alert if > 5)
- Max drawdown (must be < 10%)
- Profit factor (total wins / total losses; target: 1.5+)

### Live Trading Checklist

Before connecting API keys and trading:

- [ ] Backtesting shows 55%+ win rate
- [ ] 10 consecutive winning days on demo
- [ ] Account properly funded ($10K+ minimum)
- [ ] All HyroTrader rules hardcoded (no manual overrides)
- [ ] Stop-loss placement tested on 10 trades
- [ ] Position sizing algorithm verified
- [ ] Daily drawdown tracking active
- [ ] Logging enabled for all trades
- [ ] Email alerts set up for rule violations
- [ ] Paper trading (demo account) for 5 days minimum

### Demo Trading Phase

1. **Run on Bybit demo account** (free) for 5+ days
2. **Execute 20+ trades minimum**
3. **Achieve 55%+ win rate**
4. **Zero rule violations**
5. **Consistent profit across multiple days**

Only after these are met → proceed to HyroTrader challenge

### Transition to HyroTrader Challenge

1. **Account Type:** 2-Step Challenge ($10K minimum recommended)
2. **Phase 1 Target:** 10% profit
3. **Daily Drawdown Limit:** 5%
4. **Time Limit:** Unlimited (but minimum 10 trading days)
5. **Initial Strategy Parameters:** Use backtested values exactly

### Post-Challenge Funded Account Rules

After successfully completing both phases:

- **Trading Period:** Unlimited (no deadline)
- **Daily Drawdown:** 5% (same as challenge)
- **Account Equity Floor:** 90%
- **Max Risk Per Trade:** 3%
- **Profit Split:** 70% initially (scales to 90% with consistent performance)
- **Scaling:** After 15%+ profit and 1 month consistent trading → request capital increase

---

## MONITORING & ADJUSTMENTS

### Weekly Performance Review

Every Sunday, analyze:

| Metric | Target | Action |
|---|---|---|
| Win Rate | 55%+ | If < 50%, tighten entry filters |
| Profit Factor | 1.5+ | If < 1.2, increase TP distance |
| Max DD | < 10% | If > 8%, reduce position size 20% |
| Consecutive Losses | < 3 | If > 4, add confirmation indicator |

### Seasonal Adjustments

- **Volatility Changes:** Adjust ATR-based stop placement
- **Liquidity Changes:** Reduce position size if spreads widen > 10%
- **Major Events:** Disable trading 2 hours before FOMC, CPI releases

### Optimization (Monthly)

- Test new FVG gap size thresholds (0.3% vs. 0.7%)
- Experiment with different TP multiples (1.8× vs. 2.2×)
- Analyze best market hours (typically 14:00-22:00 UTC)

---

## RISK DISCLAIMER

This strategy is designed for HyroTrader compliance but involves trading leveraged perpetual futures. Past performance does not guarantee future results. Risk management is paramount.

**Important:** 
- Only trade with capital you can afford to lose
- Paper trade for 30+ days before live deployment
- Maintain detailed trading journal
- Never override automated stop-losses
- Account termination is immediate upon rule violation

**Do not use this strategy without understanding all risks and rules.**

---

## QUICK REFERENCE: HYROTRADER RULES CHECKLIST

Before every trade, verify:

- ✅ Stop-loss will be set within 5 minutes
- ✅ Position risk ≤ 3% of account
- ✅ Daily loss + new risk ≤ 5% limit
- ✅ Account equity ≥ 90% after position
- ✅ No more than 40% of daily profit from this trade
- ✅ Leverage ≤ 10:1 (recommended)
- ✅ Position size calculated by risk formula
- ✅ Take-profit set at 2:1 minimum ratio

---

**Strategy Version:** 2.1 | **Last Updated:** February 2026 | **Status:** Production Ready
