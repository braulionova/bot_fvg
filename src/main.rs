#[cfg(feature = "jemalloc")]
use tikv_jemallocator::Jemalloc;
#[cfg(feature = "jemalloc")]
#[global_allocator]
static GLOBAL: Jemalloc = Jemalloc;

mod bybit_api;
mod config;
mod fvg_detector;
mod position_manager;
mod telegram;
mod types;
mod websocket_handler;
#[cfg(feature = "private-ws")]
mod websocket_private;

use chrono::Timelike;
use config::{
    symbol_params, ACCOUNT_BALANCE, EQUITY_FLOOR_PCT, KLINE_INTERVAL, MAX_DAILY_LOSS_PCT,
    MAX_RISK_PER_TRADE_PCT, TRADING_PAIRS,
};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use types::{PositionData, RiskMetrics, SignalType, TradeSignal};

struct OpenPosition {
    data: PositionData,
    side: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    let tg = telegram::TelegramBot::new();
    let bybit = bybit_api::BybitClient::new();

    let mut metrics = RiskMetrics {
        account_balance: ACCOUNT_BALANCE,
        current_equity: ACCOUNT_BALANCE,
        daily_pnl: 0.0,
        max_daily_loss: ACCOUNT_BALANCE * MAX_DAILY_LOSS_PCT,
        drawdown_percentage: 0.0,
        max_risk_per_trade: ACCOUNT_BALANCE * MAX_RISK_PER_TRADE_PCT,
        trading_enabled: true,
        trades_today: 0,
        wins_today: 0,
    };

    // One open position slot per symbol
    let mut positions: HashMap<String, OpenPosition> = HashMap::new();

    // ── WebSocket: single connection, all symbols ─────────────────────────────
    let ws_client = websocket_handler::BybitWsClient::new(TRADING_PAIRS);
    let candle_map = ws_client.candle_map.clone();
    tokio::spawn(async move {
        websocket_handler::reconnect_with_backoff(&ws_client, 20, 5)
            .await
            .unwrap_or_else(|e| log::error!("WebSocket failed permanently: {}", e));
    });

    // ── Reconcile positions with exchange after restart ───────────────────────
    reconcile_positions(&bybit, &mut positions, TRADING_PAIRS).await;

    // ── Pre-load historical candles via REST in parallel ──────────────────────
    log::info!("Pre-loading 30 candles per symbol via REST (parallel)…");
    let prefetch_handles: Vec<_> = TRADING_PAIRS
        .iter()
        .map(|&symbol| {
            let bybit = bybit.clone();
            let candle_map = candle_map.clone();
            let symbol = symbol.to_string();
            tokio::spawn(async move {
                match bybit.fetch_klines(&symbol, KLINE_INTERVAL, 30).await {
                    Ok(candles) => {
                        let count = {
                            let mut map = candle_map.lock().unwrap();
                            if let Some(buf) = map.get_mut(&symbol) {
                                for c in candles {
                                    buf.push_back(c);
                                }
                                buf.len()
                            } else {
                                0
                            }
                        };
                        log::info!("[{}] pre-loaded {} candles", symbol, count);
                    }
                    Err(e) => log::warn!("[{}] kline prefetch failed: {}", symbol, e),
                }
            })
        })
        .collect();
    for h in prefetch_handles {
        let _ = h.await;
    }

    // ── Private WebSocket (production only, not available on demo) ────────────
    #[cfg(feature = "private-ws")]
    let _private_ws_positions = {
        let (private_ws, mut exec_rx) = websocket_private::BybitPrivateWs::new();
        let ws_pos_state = private_ws.position_state.clone();

        tokio::spawn(async move {
            loop {
                if let Err(e) = private_ws.connect().await {
                    log::warn!("Private WS dropped: {}. Reconnecting in 5s…", e);
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
            }
        });

        // Log fill prices from execution stream
        tokio::spawn(async move {
            while let Some(exec) = exec_rx.recv().await {
                log::info!(
                    "[{}] Fill: orderId={} price={:.2} qty={:.4} fee={:.4}",
                    exec.symbol,
                    exec.order_id,
                    exec.exec_price,
                    exec.exec_qty,
                    exec.exec_fee
                );
            }
        });

        ws_pos_state
    };

    let pairs_str = TRADING_PAIRS.join(", ");
    tg.send(&format!(
        "🤖 <b>FVG Trader started</b>\nPairs: {} | TF: 4H | Capital: ${:.0}",
        pairs_str, ACCOUNT_BALANCE
    ))
    .await;
    log::info!("FVG Trader started — pairs: {}", pairs_str);

    // ── Main loop ─────────────────────────────────────────────────────────────
    let status_interval = Duration::from_secs(5 * 60);
    let mut last_status_ts = Instant::now()
        .checked_sub(status_interval)
        .unwrap_or_else(Instant::now);

    loop {
        // Snapshot candles for all symbols under a single lock
        let all_candles: HashMap<String, Vec<types::Candle>> = {
            let map = candle_map.lock().unwrap();
            map.iter()
                .map(|(sym, buf)| (sym.clone(), buf.iter().cloned().collect()))
                .collect()
        };

        let mut status_lines: Vec<String> = Vec::new();
        // Collect validated entry signals; orders executed in parallel after loop
        let mut pending_orders: Vec<(String, TradeSignal, String)> = Vec::new();

        for symbol in TRADING_PAIRS {
            let symbol = symbol.to_string();
            let candles = match all_candles.get(&symbol) {
                Some(c) if c.len() >= 20 => c,
                _ => continue,
            };

            let p = symbol_params(&symbol);
            let atr = calculate_atr(candles, 14);
            let current_price = candles.last().unwrap().close;
            let avg_volume =
                candles.iter().rev().take(20).map(|c| c.volume).sum::<f64>() / 20.0;

            // ── Manage existing position ──────────────────────────────────────
            if let Some(op) = positions.get_mut(&symbol) {
                position_manager::update_position_pnl(&mut op.data, current_price);
                metrics.current_equity = metrics.account_balance + op.data.unrealized_pnl;

                let now_ts = chrono::Utc::now().timestamp();
                let entry = op.data.actual_entry.unwrap_or(op.data.entry_price);
                let side = op.side.clone();
                let pos_sl = op.data.stop_loss;
                let pos_tp1 = op.data.take_profit_1;
                let pos_pnl = op.data.unrealized_pnl;
                let pos_qty = op.data.position_size;
                let pos_entry_time = op.data.entry_time;

                let sl_hit = (side == "Buy" && current_price <= pos_sl)
                    || (side == "Sell" && current_price >= pos_sl);
                let tp1_hit = (side == "Buy" && current_price >= pos_tp1)
                    || (side == "Sell" && current_price <= pos_tp1);
                let time_stop = (now_ts - pos_entry_time) > p.time_stop as i64 * 4 * 3600;

                let close_reason = if sl_hit {
                    Some("Stop-loss hit")
                } else if tp1_hit {
                    Some("TP1 reached")
                } else if time_stop {
                    Some("Time stop (28 h)")
                } else {
                    None
                };

                if let Some(reason) = close_reason {
                    match bybit.close_position(&symbol, &side, pos_qty).await {
                        Ok(_) => {
                            let multiplier = if side == "Buy" { 1.0 } else { -1.0 };
                            let exit = op.data.actual_exit.unwrap_or(current_price);
                            let pnl = (exit - entry) * pos_qty * multiplier;
                            tg.notify_trade_close(&symbol, &side, entry, exit, pnl, reason)
                                .await;
                            close_position_local(
                                &mut positions,
                                &symbol,
                                &mut metrics,
                                current_price,
                            );
                        }
                        Err(e) => {
                            log::error!("[{}] Close order failed: {}", symbol, e);
                            tg.notify_risk_alert(&format!(
                                "[{}] Close order failed: {}",
                                symbol, e
                            ))
                            .await;
                        }
                    }
                }

                let h = (now_ts - pos_entry_time) / 3600;
                let pnl_emoji = if pos_pnl >= 0.0 { "📈" } else { "📉" };
                let side_emoji = if side == "Buy" { "🟢" } else { "🔴" };
                status_lines.push(format!(
                    "{side_emoji} <b>{symbol}</b> — posición abierta\n\
                     {side} @ <code>{entry:.2}</code> → <code>{current_price:.2}</code>\n\
                     SL: <code>{pos_sl:.2}</code> | TP: <code>{pos_tp1:.2}</code>\n\
                     {pnl_emoji} PnL: <code>{pos_pnl:+.2} USDT</code> | {h}h abierta",
                ));
                continue; // skip entry logic while position is open for this symbol
            }

            // ── Look for new entry signals ────────────────────────────────────
            if !metrics.trading_enabled {
                status_lines.push(format!(
                    "⛔ <b>{symbol}</b> | <code>{:.2}</code> | trading deshabilitado",
                    current_price
                ));
                continue;
            }

            let bullish = fvg_detector::detect_bullish_fvg(candles, &p);
            let bearish = fvg_detector::detect_bearish_fvg(candles, &p);
            let last_candle = candles.last().unwrap();

            let fvg_direction = if bullish.is_some() {
                "bullish"
            } else if bearish.is_some() {
                "bearish"
            } else {
                "none"
            };

            let entry_signal: Option<(TradeSignal, &str)> = if let Some(fvg) = bullish {
                if fvg_detector::check_fvg_breakout(&fvg, last_candle, avg_volume, &p) {
                    let mut sig = build_signal(SignalType::BuyBreakout, fvg, current_price);
                    position_manager::set_stop_loss(&mut sig, atr, &p);
                    position_manager::calculate_take_profits(&mut sig, &p);
                    sig.position_size =
                        position_manager::calculate_position_size(&sig, &metrics);
                    Some((sig, "Buy"))
                } else {
                    None
                }
            } else if let Some(fvg) = bearish {
                if fvg_detector::check_fvg_breakout(&fvg, last_candle, avg_volume, &p) {
                    let mut sig = build_signal(SignalType::SellBreakout, fvg, current_price);
                    position_manager::set_stop_loss(&mut sig, atr, &p);
                    position_manager::calculate_take_profits(&mut sig, &p);
                    sig.position_size =
                        position_manager::calculate_position_size(&sig, &metrics);
                    Some((sig, "Sell"))
                } else {
                    None
                }
            } else {
                None
            };

            let has_entry = entry_signal.is_some();

            if let Some((sig, side)) = entry_signal {
                match position_manager::validate_trade(&sig, &metrics) {
                    Err(e) => {
                        log::warn!("[{}] Trade skipped: {}", symbol, e);
                    }
                    Ok(_) => {
                        pending_orders.push((symbol.clone(), sig, side.to_string()));
                    }
                }
            }

            // Status line
            let status_line = match (fvg_direction, has_entry) {
                ("bullish", true) => format!(
                    "🟢 <b>{symbol}</b> | <code>{current_price:.2}</code> | Bullish FVG → <b>señal activada</b>"
                ),
                ("bearish", true) => format!(
                    "🔴 <b>{symbol}</b> | <code>{current_price:.2}</code> | Bearish FVG → <b>señal activada</b>"
                ),
                _ => {
                    if let Some(pend) = fvg_detector::scan_pending_fvg(candles, &p) {
                        let dir_emoji = if pend.direction == "bullish" { "🔼" } else { "🔽" };
                        let dir_label = if pend.direction == "bullish" { "Bullish" } else { "Bearish" };
                        format!(
                            "{dir_emoji} <b>{symbol}</b> | <code>{current_price:.2}</code> | {dir_label} FVG [<code>{:.2}</code>–<code>{:.2}</code>]\n    ⏳ Falta: {}",
                            pend.zone_low, pend.zone_high, pend.missing
                        )
                    } else {
                        format!(
                            "⚪ <b>{symbol}</b> | <code>{current_price:.2}</code> | Sin FVG en ventana ({}v)",
                            p.fvg_lookback
                        )
                    }
                }
            };
            status_lines.push(status_line);
        } // end symbol loop

        // ── Execute all pending entry orders in parallel ───────────────────────
        if !pending_orders.is_empty() {
            let order_handles: Vec<_> = pending_orders
                .into_iter()
                .map(|(symbol, sig, side)| {
                    let bybit = bybit.clone();
                    let tg = tg.clone();
                    tokio::spawn(async move {
                        match bybit
                            .place_order(
                                &symbol,
                                &side,
                                sig.position_size,
                                sig.stop_loss,
                                sig.take_profit_1,
                            )
                            .await
                        {
                            Ok(order_id) => {
                                tg.notify_trade_open(
                                    &symbol,
                                    &side,
                                    sig.position_size,
                                    sig.entry_price,
                                    sig.stop_loss,
                                    sig.take_profit_1,
                                    sig.take_profit_2,
                                )
                                .await;
                                log::info!(
                                    "[{}] {} qty={:.4} entry={:.2} sl={:.2} tp1={:.2} orderId={}",
                                    symbol,
                                    side,
                                    sig.position_size,
                                    sig.entry_price,
                                    sig.stop_loss,
                                    sig.take_profit_1,
                                    order_id
                                );
                                Some((symbol, sig, side, order_id))
                            }
                            Err(e) => {
                                log::error!("[{}] Place order failed: {}", symbol, e);
                                tg.notify_risk_alert(&format!(
                                    "[{}] Order placement failed: {}",
                                    symbol, e
                                ))
                                .await;
                                None
                            }
                        }
                    })
                })
                .collect();

            for handle in order_handles {
                if let Ok(Some((symbol, sig, side, order_id))) = handle.await {
                    positions.insert(
                        symbol.clone(),
                        OpenPosition {
                            data: create_position(&sig, &order_id),
                            side,
                        },
                    );
                    metrics.trades_today += 1;
                }
            }
        }

        // ── Status report every 5 minutes ────────────────────────────────────
        if last_status_ts.elapsed() >= status_interval && !status_lines.is_empty() {
            tg.notify_status(
                &status_lines,
                metrics.current_equity,
                metrics.daily_pnl,
                metrics.trades_today,
                metrics.trading_enabled,
            )
            .await;
            last_status_ts = Instant::now();
        }

        // ── Daily reset at UTC midnight ───────────────────────────────────────
        if is_daily_reset_time() {
            tg.notify_daily_summary(
                metrics.daily_pnl,
                metrics.trades_today,
                metrics.wins_today,
                metrics.current_equity,
            )
            .await;
            log::info!(
                "Daily reset | PnL: {:.2} | Trades: {} | Wins: {}",
                metrics.daily_pnl,
                metrics.trades_today,
                metrics.wins_today
            );
            metrics.daily_pnl = 0.0;
            metrics.trades_today = 0;
            metrics.wins_today = 0;
            metrics.trading_enabled =
                metrics.current_equity >= metrics.account_balance * EQUITY_FLOOR_PCT;
        }

        // Disable trading if daily drawdown limit reached
        if metrics.daily_pnl < -(metrics.max_daily_loss) && metrics.trading_enabled {
            metrics.trading_enabled = false;
            tg.notify_risk_alert(
                "Daily drawdown limit reached. Trading halted for today across all pairs.",
            )
            .await;
            log::warn!("Daily drawdown limit reached. Trading disabled.");
        }

        tokio::time::sleep(tokio::time::Duration::from_secs(60)).await;
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Reconcile local position state with exchange after restart.
/// Detects orphan exchange positions (no local state) and stale local state
/// (local position but exchange reports zero size). Only logs — no auto-correct
/// during runtime to avoid race conditions.
async fn reconcile_positions(
    bybit: &bybit_api::BybitClient,
    local_positions: &mut HashMap<String, OpenPosition>,
    symbols: &[&str],
) {
    log::info!("Reconciling positions with exchange…");
    for &symbol in symbols {
        match bybit.get_position(symbol).await {
            Err(e) => {
                log::warn!("[{}] Reconcile fetch failed: {}", symbol, e);
                continue;
            }
            Ok(json) => {
                // Parse size and side from Bybit V5 position list
                let exchange_size: f64 = json["result"]["list"]
                    .as_array()
                    .and_then(|a| a.first())
                    .and_then(|p| p["size"].as_str())
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0.0);

                match (local_positions.get_mut(symbol), exchange_size > 0.0) {
                    (None, false) => {
                        log::debug!("[{}] No position (match OK)", symbol);
                    }
                    (Some(local), true) => {
                        if (local.data.position_size - exchange_size).abs() > 0.001 {
                            log::warn!(
                                "[{}] Size mismatch: local={:.4}, exchange={:.4}. Using exchange.",
                                symbol,
                                local.data.position_size,
                                exchange_size
                            );
                            local.data.position_size = exchange_size;
                        }
                    }
                    (None, true) => {
                        log::error!(
                            "[{}] Orphan exchange position: size={:.4}. Manual intervention needed.",
                            symbol,
                            exchange_size
                        );
                    }
                    (Some(_), false) => {
                        log::error!(
                            "[{}] Local position exists but exchange size=0. Clearing local state.",
                            symbol
                        );
                        local_positions.remove(symbol);
                    }
                }
            }
        }
    }
    log::info!("Position reconciliation complete.");
}

fn build_signal(
    signal_type: SignalType,
    fvg_zone: types::FVGZone,
    current_price: f64,
) -> TradeSignal {
    TradeSignal {
        signal_type,
        fvg_zone,
        entry_price: current_price,
        stop_loss: 0.0,
        take_profit_1: 0.0,
        take_profit_2: 0.0,
        position_size: 0.0,
        risk_amount: 0.0,
        risk_reward_ratio: 0.0,
        timestamp: chrono::Utc::now().timestamp(),
    }
}

fn calculate_atr(candles: &[types::Candle], period: usize) -> f64 {
    if candles.len() < period + 1 {
        return 0.0;
    }
    let start = candles.len() - period - 1;
    let mut tr_sum = 0.0;
    for i in (start + 1)..candles.len() {
        let curr = &candles[i];
        let prev = &candles[i - 1];
        let tr = (curr.high - curr.low)
            .max((curr.high - prev.close).abs())
            .max((curr.low - prev.close).abs());
        tr_sum += tr;
    }
    tr_sum / period as f64
}

fn close_position_local(
    positions: &mut HashMap<String, OpenPosition>,
    symbol: &str,
    metrics: &mut RiskMetrics,
    exit_price: f64,
) {
    if let Some(op) = positions.remove(symbol) {
        let multiplier = if op.side == "Buy" { 1.0 } else { -1.0 };
        let entry = op.data.actual_entry.unwrap_or(op.data.entry_price);
        let exit = op.data.actual_exit.unwrap_or(exit_price);
        let pnl = (exit - entry) * op.data.position_size * multiplier;
        metrics.account_balance += pnl;
        metrics.current_equity = metrics.account_balance;
        metrics.daily_pnl += pnl;
        if pnl > 0.0 {
            metrics.wins_today += 1;
        }
        log::info!(
            "[{}] Closed @ {:.2} | PnL: {:+.2} | Balance: {:.2}",
            symbol,
            exit,
            pnl,
            metrics.account_balance
        );
    }
}

fn create_position(signal: &TradeSignal, order_id: &str) -> PositionData {
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
        order_id: order_id.to_string(),
        actual_entry: None,
        actual_exit: None,
    }
}

fn is_daily_reset_time() -> bool {
    let now = chrono::Utc::now();
    now.hour() == 0 && now.minute() == 0
}
