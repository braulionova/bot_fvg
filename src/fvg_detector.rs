use crate::config::SymbolParams;
use crate::types::{Candle, FVGType, FVGZone};

const VOL_AVG_PERIOD: usize = 20;

/// Info de una zona FVG detectada que aún no ha disparado entrada.
pub struct PendingFvgInfo {
    pub direction: &'static str, // "bullish" | "bearish"
    pub zone_high: f64,
    pub zone_low:  f64,
    pub missing:   String, // descripción legible de qué falta
}

/// Busca la mejor zona FVG pendiente (válida pero sin breakout confirmado).
/// Retorna la condición que falta para que dispare.
pub fn scan_pending_fvg(candles: &[Candle], p: &SymbolParams) -> Option<PendingFvgInfo> {
    scan_pending_bullish(candles, p).or_else(|| scan_pending_bearish(candles, p))
}

fn scan_pending_bullish(candles: &[Candle], p: &SymbolParams) -> Option<PendingFvgInfo> {
    let n = candles.len();
    if n < p.fvg_lookback + 3 { return None; }

    let current     = &candles[n - 1];
    let avg_vol     = avg_volume(candles, VOL_AVG_PERIOD);
    let search_start = n.saturating_sub(p.fvg_lookback + 2);

    for j in search_start..(n - 2) {
        let c1 = &candles[j];
        let c2 = &candles[j + 1];
        let c3 = &candles[j + 2];
        let imp_vol = avg_volume(&candles[..=j + 1], VOL_AVG_PERIOD);

        if c3.low > c1.high {
            let gap = c3.low - c1.high;
            if gap > c2.close * p.min_gap_pct
                && c2.close > c2.open
                && c2.volume > imp_vol * p.min_vol_mult
            {
                let zone_low  = c1.high;
                let zone_high = c3.low;
                let retest_threshold = zone_high + (zone_high - zone_low) * 0.5;

                let retested = candles[j + 3..].iter()
                    .any(|c| c.low <= retest_threshold);

                let missing = if !retested {
                    format!("Retest pendiente (precio ≤ {retest_threshold:.2})")
                } else if current.close <= zone_high {
                    format!("Cierre superar {zone_high:.2} (actual {:.2})", current.close)
                } else {
                    let req = avg_vol * p.min_vol_mult;
                    format!(
                        "Vol insuf: {:.0} / req {:.0} ({:.2}× avg)",
                        current.volume, req, current.volume / avg_vol
                    )
                };

                return Some(PendingFvgInfo {
                    direction: "bullish",
                    zone_high,
                    zone_low,
                    missing,
                });
            }
        }
    }
    None
}

fn scan_pending_bearish(candles: &[Candle], p: &SymbolParams) -> Option<PendingFvgInfo> {
    let n = candles.len();
    if n < p.fvg_lookback + 3 { return None; }

    let current     = &candles[n - 1];
    let avg_vol     = avg_volume(candles, VOL_AVG_PERIOD);
    let search_start = n.saturating_sub(p.fvg_lookback + 2);

    for j in search_start..(n - 2) {
        let c1 = &candles[j];
        let c2 = &candles[j + 1];
        let c3 = &candles[j + 2];
        let imp_vol = avg_volume(&candles[..=j + 1], VOL_AVG_PERIOD);

        if c1.low > c3.high {
            let gap = c1.low - c3.high;
            if gap > c2.close * p.min_gap_pct
                && c2.close < c2.open
                && c2.volume > imp_vol * p.min_vol_mult
            {
                let zone_low  = c3.high;
                let zone_high = c1.low;
                let retest_threshold = zone_low - (zone_high - zone_low) * 0.5;

                let retested = candles[j + 3..].iter()
                    .any(|c| c.high >= retest_threshold);

                let missing = if !retested {
                    format!("Retest pendiente (precio ≥ {retest_threshold:.2})")
                } else if current.close >= zone_low {
                    format!("Cierre perforar {zone_low:.2} (actual {:.2})", current.close)
                } else {
                    let req = avg_vol * p.min_vol_mult;
                    format!(
                        "Vol insuf: {:.0} / req {:.0} ({:.2}× avg)",
                        current.volume, req, current.volume / avg_vol
                    )
                };

                return Some(PendingFvgInfo {
                    direction: "bearish",
                    zone_high,
                    zone_low,
                    missing,
                });
            }
        }
    }
    None
}

/// Detecta un FVG alcista usando el patrón 3-velas (válido para futuros perpetuos).
///
/// Bullish FVG: c3.low > c1.high  →  zona = [c1.high, c3.low]
/// c2 es la vela impulso (verde, volumen elevado).
/// Busca en las últimas `p.fvg_lookback` velas y confirma con la vela actual.
pub fn detect_bullish_fvg(candles: &[Candle], p: &SymbolParams) -> Option<FVGZone> {
    let n = candles.len();
    if n < p.fvg_lookback + 3 { return None; }

    let current  = &candles[n - 1];
    let avg_vol  = avg_volume(candles, VOL_AVG_PERIOD);
    let search_start = n.saturating_sub(p.fvg_lookback + 2);

    for j in search_start..(n - 2) {
        let c1 = &candles[j];
        let c2 = &candles[j + 1]; // impulso
        let c3 = &candles[j + 2];
        let imp_vol = avg_volume(&candles[..=j + 1], VOL_AVG_PERIOD);

        if c3.low > c1.high {
            let gap = c3.low - c1.high;
            if gap > c2.close * p.min_gap_pct
                && c2.close > c2.open
                && c2.volume > imp_vol * p.min_vol_mult
            {
                let zone_low  = c1.high;
                let zone_high = c3.low;

                // El precio debe haber retestado la zona tras formarse
                let retested = candles[j + 3..].iter()
                    .any(|c| c.low <= zone_high + (zone_high - zone_low) * 0.5);

                // Entrada: vela actual cierra sobre la zona con volumen
                if retested
                    && current.close > zone_high
                    && current.volume > avg_vol * p.min_vol_mult
                {
                    return Some(FVGZone {
                        fvg_type: FVGType::Bullish,
                        zone_high,
                        zone_low,
                        impulse_high: c2.high,
                        impulse_low:  c2.low,
                        created_timestamp: c2.timestamp,
                        is_filled: false,
                    });
                }
            }
        }
    }
    None
}

/// Detecta un FVG bajista usando el patrón 3-velas.
///
/// Bearish FVG: c1.low > c3.high  →  zona = [c3.high, c1.low]
pub fn detect_bearish_fvg(candles: &[Candle], p: &SymbolParams) -> Option<FVGZone> {
    let n = candles.len();
    if n < p.fvg_lookback + 3 { return None; }

    let current  = &candles[n - 1];
    let avg_vol  = avg_volume(candles, VOL_AVG_PERIOD);
    let search_start = n.saturating_sub(p.fvg_lookback + 2);

    for j in search_start..(n - 2) {
        let c1 = &candles[j];
        let c2 = &candles[j + 1];
        let c3 = &candles[j + 2];
        let imp_vol = avg_volume(&candles[..=j + 1], VOL_AVG_PERIOD);

        if c1.low > c3.high {
            let gap = c1.low - c3.high;
            if gap > c2.close * p.min_gap_pct
                && c2.close < c2.open
                && c2.volume > imp_vol * p.min_vol_mult
            {
                let zone_low  = c3.high;
                let zone_high = c1.low;

                let retested = candles[j + 3..].iter()
                    .any(|c| c.high >= zone_low - (zone_high - zone_low) * 0.5);

                if retested
                    && current.close < zone_low
                    && current.volume > avg_vol * p.min_vol_mult
                {
                    return Some(FVGZone {
                        fvg_type: FVGType::Bearish,
                        zone_high,
                        zone_low,
                        impulse_high: c2.high,
                        impulse_low:  c2.low,
                        created_timestamp: c2.timestamp,
                        is_filled: false,
                    });
                }
            }
        }
    }
    None
}

pub fn check_fvg_breakout(fvg: &FVGZone, current_candle: &Candle, avg_volume: f64, p: &SymbolParams) -> bool {
    match fvg.fvg_type {
        FVGType::Bullish => {
            current_candle.close > fvg.zone_high
                && current_candle.volume > avg_volume * p.min_vol_mult
        }
        FVGType::Bearish => {
            current_candle.close < fvg.zone_low
                && current_candle.volume > avg_volume * p.min_vol_mult
        }
    }
}

pub fn check_fvg_filled(fvg: &FVGZone, current_price: f64) -> bool {
    match fvg.fvg_type {
        FVGType::Bullish => current_price < fvg.zone_low,
        FVGType::Bearish => current_price > fvg.zone_high,
    }
}

fn avg_volume(candles: &[Candle], period: usize) -> f64 {
    let n = candles.len().min(period);
    if n == 0 { return 0.0; }
    candles.iter().rev().take(n).map(|c| c.volume).sum::<f64>() / n as f64
}
