use serde::{Deserialize, Serialize};

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
    pub fvg_type: FVGType,
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
    pub order_id: String,
    pub actual_entry: Option<f64>,  // Fill real (de WS privado en producción)
    pub actual_exit: Option<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum BiasDirection {
    Bullish,
    Bearish,
    Neutral,
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
