use crate::config::{SymbolParams, MAX_RISK_PER_TRADE_PCT};
use crate::types::{FVGType, PositionData, RiskMetrics, TradeSignal};

pub fn calculate_position_size(signal: &TradeSignal, metrics: &RiskMetrics, p: &SymbolParams) -> f64 {
    let max_risk = metrics.account_balance * MAX_RISK_PER_TRADE_PCT;

    // Don't exceed remaining daily drawdown budget
    let remaining_daily_budget = (metrics.max_daily_loss - metrics.daily_pnl.abs()).max(0.0);
    let actual_max_risk = max_risk.min(remaining_daily_budget);

    let risk_per_unit = (signal.entry_price - signal.stop_loss).abs();

    if risk_per_unit > 0.0 {
        let raw_qty = actual_max_risk / risk_per_unit;
        // Round DOWN to the exchange's minimum lot step (e.g. 0.001 BTC, 0.01 ETH, 1 XRP)
        let steps = (raw_qty / p.qty_step).floor();
        steps * p.qty_step
    } else {
        0.0
    }
}

const MIN_ORDER_NOTIONAL: f64 = 100.0; // Bybit minimum order value in USDT

pub fn validate_trade(signal: &TradeSignal, metrics: &RiskMetrics) -> Result<(), String> {
    if signal.position_size <= 0.0 {
        return Err("Position size is zero (SL distance exceeds risk budget)".to_string());
    }

    let notional = signal.position_size * signal.entry_price;
    if notional < MIN_ORDER_NOTIONAL {
        return Err(format!(
            "Notional {:.2} USDT below minimum {:.0} USDT (qty={:.4} @ {:.2})",
            notional, MIN_ORDER_NOTIONAL, signal.position_size, signal.entry_price
        ));
    }

    if !metrics.trading_enabled {
        return Err("Trading disabled due to daily loss limit".to_string());
    }

    let min_equity = metrics.account_balance * 0.90;
    if metrics.current_equity < min_equity {
        return Err(format!(
            "Equity below 90% floor: {:.2}",
            metrics.current_equity
        ));
    }

    if signal.risk_amount > metrics.max_risk_per_trade {
        return Err(format!(
            "Trade risk {:.2} exceeds max {:.2}",
            signal.risk_amount, metrics.max_risk_per_trade
        ));
    }

    let remaining_daily = metrics.max_daily_loss - metrics.daily_pnl.abs();
    if signal.risk_amount > remaining_daily {
        return Err("Insufficient daily drawdown budget".to_string());
    }

    Ok(())
}

pub fn set_stop_loss(signal: &mut TradeSignal, atr: f64, p: &SymbolParams) {
    match signal.fvg_zone.fvg_type {
        FVGType::Bullish => {
            signal.stop_loss = signal.fvg_zone.zone_low - atr * p.sl_atr_mult;
            signal.risk_amount =
                (signal.entry_price - signal.stop_loss) * signal.position_size;
        }
        FVGType::Bearish => {
            signal.stop_loss = signal.fvg_zone.zone_high + atr * p.sl_atr_mult;
            signal.risk_amount =
                (signal.stop_loss - signal.entry_price) * signal.position_size;
        }
    }
}

pub fn calculate_take_profits(signal: &mut TradeSignal, p: &SymbolParams) {
    let risk = (signal.entry_price - signal.stop_loss).abs();
    match signal.fvg_zone.fvg_type {
        FVGType::Bullish => {
            signal.take_profit_1 = signal.entry_price + risk * p.tp_mult;
            signal.take_profit_2 = signal.entry_price + risk * p.tp_mult * 1.5;
        }
        FVGType::Bearish => {
            signal.take_profit_1 = signal.entry_price - risk * p.tp_mult;
            signal.take_profit_2 = signal.entry_price - risk * p.tp_mult * 1.5;
        }
    }
    signal.risk_reward_ratio = p.tp_mult;
}

pub fn update_position_pnl(position: &mut PositionData, current_price: f64) {
    let entry = position.actual_entry.unwrap_or(position.entry_price);
    position.unrealized_pnl = (current_price - entry) * position.position_size;

    if current_price > position.max_favorable_excursion {
        position.max_favorable_excursion = current_price;
    }
}
