use crate::config::SymbolParams;
use crate::types::{FVGType, PositionData, RiskMetrics, TradeSignal};

pub fn calculate_position_size(signal: &TradeSignal, metrics: &RiskMetrics) -> f64 {
    let max_risk = metrics.account_balance * 0.03;

    // Don't exceed remaining daily drawdown budget
    let remaining_daily_budget = (metrics.max_daily_loss - metrics.daily_pnl.abs()).max(0.0);
    let actual_max_risk = max_risk.min(remaining_daily_budget);

    let risk_per_unit = (signal.entry_price - signal.stop_loss).abs();

    if risk_per_unit > 0.0 {
        (actual_max_risk / risk_per_unit).floor()
    } else {
        0.0
    }
}

pub fn validate_trade(signal: &TradeSignal, metrics: &RiskMetrics) -> Result<(), String> {
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
