#[cfg(feature = "jemalloc")]
use tikv_jemallocator::Jemalloc;
#[cfg(feature = "jemalloc")]
#[global_allocator]
static GLOBAL: Jemalloc = Jemalloc;

#[cfg(feature = "jemalloc")]
fn jemalloc_purge() {
    use tikv_jemalloc_ctl::epoch;
    // Advancing the epoch causes jemalloc to evaluate all decay windows
    // and release dirty pages back to the OS via its background purge logic.
    if let Ok(e) = epoch::mib() {
        let _ = e.advance();
    }
    log::debug!("jemalloc: epoch advanced — dirty pages scheduled for release");
}

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
    symbol_params, tick_decimals, ACCOUNT_BALANCE, EQUITY_FLOOR_PCT, KLINE_INTERVALS,
    MAX_DAILY_LOSS_PCT, MAX_OPEN_POSITIONS, MAX_RISK_PER_TRADE_PCT, TRADING_PAIRS, TF_BIAS,
    TF_ENTRY, TF_STRUCT, USE_ALL_PAIRS,
};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;
use bybit_api::ExchangePositionInfo;
use types::{BiasDirection, PositionData, RiskMetrics, SignalType, TradeSignal};

struct OpenPosition {
    data: PositionData,
    side: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    let tg = telegram::TelegramBot::new();
    let bybit = bybit_api::BybitClient::new();

    // ── Determine trading pairs ───────────────────────────────────────────────
    let trading_pairs: Vec<String> = if USE_ALL_PAIRS {
        match bybit.fetch_linear_symbols().await {
            Ok(pairs) => {
                log::info!("Fetched {} USDT linear symbols from Bybit", pairs.len());
                pairs
            }
            Err(e) => {
                log::warn!("fetch_linear_symbols failed: {} — falling back to default pairs", e);
                TRADING_PAIRS.iter().map(|s| s.to_string()).collect()
            }
        }
    } else {
        TRADING_PAIRS.iter().map(|s| s.to_string()).collect()
    };
    // Slice of &str for APIs that take &[&str]
    let pair_refs: Vec<&str> = trading_pairs.iter().map(|s| s.as_str()).collect();

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
    let ws_client = websocket_handler::BybitWsClient::new(&pair_refs, KLINE_INTERVALS);
    let candle_map = ws_client.candle_map.clone();
    tokio::spawn(async move {
        websocket_handler::reconnect_with_backoff(&ws_client, 20, 5)
            .await
            .unwrap_or_else(|e| log::error!("WebSocket failed permanently: {}", e));
    });

    // ── Reconcile positions with exchange after restart ───────────────────────
    reconcile_positions(&bybit, &mut positions, &pair_refs).await;

    // ── Pre-load historical candles via REST in parallel ─────────────────────
    // Semaphore limits concurrent HTTP requests (important with many pairs).
    let sem = Arc::new(Semaphore::new(20));
    log::info!(
        "Pre-loading 30 candles × {} symbols × {} TFs via REST…",
        trading_pairs.len(), KLINE_INTERVALS.len()
    );
    let prefetch_handles: Vec<_> = trading_pairs
        .iter()
        .flat_map(|symbol| {
            let sem = sem.clone();
            let bybit = bybit.clone();
            let candle_map = candle_map.clone();
            KLINE_INTERVALS.iter().map(move |&tf| {
                let sem = sem.clone();
                let bybit = bybit.clone();
                let candle_map = candle_map.clone();
                let symbol = symbol.clone();
                let tf = tf.to_string();
                tokio::spawn(async move {
                    let _permit = sem.acquire().await.unwrap();
                    let key = format!("{}_{}", symbol, tf);
                    match bybit.fetch_klines(&symbol, &tf, 30).await {
                        Ok(candles) => {
                            let count = {
                                let mut map = candle_map.lock().unwrap();
                                if let Some(buf) = map.get_mut(&key) {
                                    for c in candles { buf.push_back(c); }
                                    buf.len()
                                } else { 0 }
                            };
                            log::info!("[{} {}] pre-loaded {} candles", symbol, tf, count);
                        }
                        Err(e) => log::warn!("[{} {}] prefetch failed: {}", symbol, tf, e),
                    }
                })
            })
        })
        .collect();
    for h in prefetch_handles { let _ = h.await; }

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

    let pairs_str = if USE_ALL_PAIRS {
        format!("{} pares USDT linear", trading_pairs.len())
    } else {
        trading_pairs.join(", ")
    };
    tg.send(&format!(
        "🤖 <b>FVG Trader started</b>\nPairs: {} | TF: 4H bias / 1H BOS / 15M FVG | Capital: ${:.0}",
        pairs_str, ACCOUNT_BALANCE
    ))
    .await;
    log::info!("FVG Trader started — {} pairs", trading_pairs.len());

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
        let mut pending_orders: Vec<(String, TradeSignal, String, usize)> = Vec::new();

        // ── Detect manually closed positions ─────────────────────────────────
        // Single REST call fetches all open positions; any locally tracked symbol
        // absent from the exchange result was closed outside the bot.
        if !positions.is_empty() {
            match bybit.get_all_open_positions().await {
                Ok(exchange_pos) => {
                    let manually_closed: Vec<String> = positions
                        .keys()
                        .filter(|sym| !exchange_pos.contains_key(*sym))
                        .cloned()
                        .collect();

                    for sym in manually_closed {
                        if let Some(op) = positions.get(&sym) {
                            let entry = op.data.actual_entry.unwrap_or(op.data.entry_price);
                            let side = op.side.clone();
                            // unrealized_pnl is the best estimate we have at this point
                            let pnl_estimate = op.data.unrealized_pnl;
                            log::warn!(
                                "[{}] Manual close detected — {} entry={:.2} pnl≈{:+.2}",
                                sym, side, entry, pnl_estimate
                            );
                            tg.notify_manual_close(&sym, &side, entry, pnl_estimate).await;
                        }
                        // Derive an approximate exit price from unrealized_pnl so metrics stay consistent
                        let exit_price = if let Some(op) = positions.get(&sym) {
                            let entry = op.data.actual_entry.unwrap_or(op.data.entry_price);
                            let multiplier = if op.side == "Buy" { 1.0_f64 } else { -1.0_f64 };
                            let qty = op.data.position_size;
                            if qty > 0.0 {
                                entry + (op.data.unrealized_pnl / qty) * multiplier
                            } else {
                                entry
                            }
                        } else {
                            0.0
                        };
                        close_position_local(&mut positions, &sym, &mut metrics, exit_price);
                    }
                }
                Err(e) => {
                    log::warn!("Manual-close check failed: {}", e);
                }
            }
        }

        for symbol in &trading_pairs {
            let symbol = symbol.clone();

            // Collect candles per TF from the snapshot (keys = "SYMBOL_TF")
            let key_4h  = format!("{}_{}", symbol, TF_BIAS);
            let key_1h  = format!("{}_{}", symbol, TF_STRUCT);
            let key_15m = format!("{}_{}", symbol, TF_ENTRY);

            let candles_4h = match all_candles.get(&key_4h) {
                Some(c) if c.len() >= 20 => c,
                _ => continue,
            };
            let candles_15m = match all_candles.get(&key_15m) {
                Some(c) if c.len() >= 20 => c,
                _ => continue,
            };

            let p = symbol_params(&symbol);
            // ATR on 4H as fallback; BB(20,2σ) on 4H for primary SL/TP
            let atr = calculate_atr(candles_4h, 14);
            let bb_4h = fvg_detector::bollinger_bands(candles_4h, 20);
            let current_price = candles_15m.last().unwrap().close;

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

                let mut position_closed = false;
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
                            position_closed = true;
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

                // Si la posición sigue abierta, mostrar estado y saltar detección de entrada.
                // Si se cerró en este ciclo, dejar que el flujo continúe para buscar nueva señal.
                if !position_closed {
                    let h = (now_ts - pos_entry_time) / 3600;
                    let pnl_emoji = if pos_pnl >= 0.0 { "📈" } else { "📉" };
                    let side_emoji = if side == "Buy" { "🟢" } else { "🔴" };
                    status_lines.push(format!(
                        "{side_emoji} <b>{symbol}</b> — posición abierta\n\
                         {side} @ <code>{entry:.2}</code> → <code>{current_price:.2}</code>\n\
                         SL: <code>{pos_sl:.2}</code> | TP: <code>{pos_tp1:.2}</code>\n\
                         {pnl_emoji} PnL: <code>{pos_pnl:+.2} USDT</code> | {h}h abierta",
                    ));
                    continue;
                }
                // position_closed == true → fall through to entry detection below
            }

            // ── Look for new entry signals ────────────────────────────────────
            if !metrics.trading_enabled {
                status_lines.push(format!(
                    "⛔ <b>{symbol}</b> | <code>{:.2}</code> | trading deshabilitado",
                    current_price
                ));
                continue;
            }

            if positions.len() >= MAX_OPEN_POSITIONS {
                status_lines.push(format!(
                    "⏸ <b>{symbol}</b> | <code>{:.2}</code> | máx posiciones ({}/{})",
                    current_price, positions.len(), MAX_OPEN_POSITIONS
                ));
                continue;
            }

            // ── Filter 1: 4H bias via SMA(20) ────────────────────────────────
            let bias = fvg_detector::detect_bias(candles_4h);
            if bias == BiasDirection::Neutral {
                log::info!("[{}] 4H bias Neutral — skip", symbol);
                status_lines.push(format!(
                    "⚪ <b>{symbol}</b> | <code>{current_price:.2}</code> | 4H bias neutro"
                ));
                continue;
            }

            // ── Filter 2: 1H Break of Structure ──────────────────────────────
            let candles_1h = match all_candles.get(&key_1h) {
                Some(c) if c.len() >= 21 => c,
                _ => {
                    let bias_label = if bias == BiasDirection::Bullish { "alcista" } else { "bajista" };
                    status_lines.push(format!(
                        "⏳ <b>{symbol}</b> | <code>{current_price:.2}</code> | 1H sin datos suficientes (bias {bias_label})"
                    ));
                    continue;
                }
            };
            let structure_ok = fvg_detector::detect_structure_break(candles_1h, &bias);
            if !structure_ok {
                let bias_label = if bias == BiasDirection::Bullish { "4H↑" } else { "4H↓" };
                log::info!("[{}] {} — 1H sin BOS aún", symbol, bias_label);
                status_lines.push(format!(
                    "⏳ <b>{symbol}</b> | <code>{current_price:.2}</code> | {bias_label} — 1H sin BOS aún"
                ));
                continue;
            }

            // ── Filter 3: 15M FVG in bias direction ──────────────────────────
            let avg_volume_15m =
                candles_15m.iter().rev().take(20).map(|c| c.volume).sum::<f64>() / 20.0;
            let last_15m = candles_15m.last().unwrap();

            // Bollinger Bands (20, 2σ) sobre 15M — la banda media filtra la entrada
            let bb_15m = fvg_detector::bollinger_bands(candles_15m, 20);

            let (fvg_opt, signal_type, side_str) = match bias {
                BiasDirection::Bullish => (
                    fvg_detector::detect_bullish_fvg(candles_15m, &p),
                    SignalType::BuyBreakout,
                    "Buy",
                ),
                BiasDirection::Bearish => (
                    fvg_detector::detect_bearish_fvg(candles_15m, &p),
                    SignalType::SellBreakout,
                    "Sell",
                ),
                BiasDirection::Neutral => unreachable!(),
            };

            let fvg_direction = if bias == BiasDirection::Bullish { "bullish" } else { "bearish" };
            let bias_label    = if bias == BiasDirection::Bullish { "4H↑ BOS✓" } else { "4H↓ BOS✓" };

            let entry_signal: Option<(TradeSignal, &str)> = if let Some(fvg) = fvg_opt {
                if fvg_detector::check_fvg_breakout(&fvg, last_15m, avg_volume_15m, &p) {
                    let mut sig = build_signal(signal_type, fvg, current_price);
                    position_manager::set_stop_loss(&mut sig, atr, &p, bb_4h.as_ref());
                    position_manager::calculate_take_profits(&mut sig, &p, bb_4h.as_ref());

                    // Round SL/TP to tick_size BEFORE sizing so position qty
                    // matches the actual SL distance the exchange will use.
                    let tick = p.tick_size;
                    if tick > 0.0 {
                        sig.stop_loss     = (sig.stop_loss / tick).round() * tick;
                        sig.take_profit_1 = (sig.take_profit_1 / tick).round() * tick;
                        sig.take_profit_2 = (sig.take_profit_2 / tick).round() * tick;
                    }

                    sig.position_size =
                        position_manager::calculate_position_size(&sig, &metrics, &p);

                    // Recalculate risk_amount with the final position_size
                    sig.risk_amount =
                        (sig.entry_price - sig.stop_loss).abs() * sig.position_size;

                    Some((sig, side_str))
                } else {
                    None
                }
            } else {
                None
            };

            let has_entry = entry_signal.is_some();

            // Log state every cycle for diagnostics
            let bb_log = bb_15m.as_ref().map(|b| format!(
                "BB[{:.2}/{:.2}/{:.2}]", b.lower, b.middle, b.upper
            )).unwrap_or_else(|| "BB[n/a]".into());
            match (fvg_direction, has_entry) {
                (dir, true) => log::info!(
                    "[{}] {} {} FVG 15M → breakout confirmado | precio={:.2} ATR={:.2} {}",
                    symbol, bias_label, dir, current_price, atr, bb_log
                ),
                (dir, false) => log::info!(
                    "[{}] {} {} FVG 15M detectado/pendiente | precio={:.2} ATR={:.2} {}",
                    symbol, bias_label, dir, current_price, atr, bb_log
                ),
            }

            if let Some((sig, side)) = entry_signal {
                // Hard guard: TP/SL must be directionally consistent with trade side.
                let tp_ok = if side == "Buy" {
                    sig.take_profit_1 > sig.entry_price && sig.stop_loss < sig.entry_price
                } else {
                    sig.take_profit_1 < sig.entry_price && sig.stop_loss > sig.entry_price
                };
                if !tp_ok {
                    log::error!(
                        "[{}] TP/SL direction mismatch! side={} entry={:.6} sl={:.6} tp={:.6} fvg_type={:?}",
                        symbol, side, sig.entry_price, sig.stop_loss, sig.take_profit_1, sig.fvg_zone.fvg_type
                    );
                    continue;
                }
                match position_manager::validate_trade(&sig, &metrics) {
                    Err(e) => {
                        log::warn!("[{}] Trade skipped: {}", symbol, e);
                    }
                    Ok(_) => {
                        let pd = tick_decimals(p.tick_size);
                        pending_orders.push((symbol.clone(), sig, side.to_string(), pd));
                    }
                }
            }

            // Bollinger Bands compact string for status display
            let bb_status = bb_15m.as_ref().map(|b| format!(
                "BB20 ▲<code>{:.2}</code> ─<code>{:.2}</code> ▼<code>{:.2}</code>",
                b.upper, b.middle, b.lower
            )).unwrap_or_else(|| "BB20 n/a".into());

            // Status line
            let status_line = match has_entry {
                true if bias == BiasDirection::Bullish => format!(
                    "🟢 <b>{symbol}</b> | <code>{current_price:.2}</code> | {bias_label} — Bullish FVG 15M → <b>señal activada</b>\n    {bb_status}"
                ),
                true => format!(
                    "🔴 <b>{symbol}</b> | <code>{current_price:.2}</code> | {bias_label} — Bearish FVG 15M → <b>señal activada</b>\n    {bb_status}"
                ),
                false => {
                    if let Some(pend) = fvg_detector::scan_pending_fvg(candles_15m, &p) {
                        let dir_emoji = if pend.direction == "bullish" { "🔼" } else { "🔽" };
                        let dir_label = if pend.direction == "bullish" { "Bullish" } else { "Bearish" };
                        format!(
                            "{dir_emoji} <b>{symbol}</b> | <code>{current_price:.2}</code> | {bias_label} — {dir_label} FVG 15M [<code>{:.2}</code>–<code>{:.2}</code>]\n    ⏳ Falta: {}\n    {bb_status}",
                            pend.zone_low, pend.zone_high, pend.missing
                        )
                    } else {
                        format!(
                            "⚪ <b>{symbol}</b> | <code>{current_price:.2}</code> | {bias_label} — Sin FVG 15M en ventana ({}v)\n    {bb_status}",
                            p.fvg_lookback
                        )
                    }
                }
            };
            status_lines.push(status_line);
        } // end symbol loop

        // ── Execute all pending entry orders in parallel ───────────────────────
        if !pending_orders.is_empty() {
            // Verify live exchange position count before placing any order.
            // This guards against state drift (e.g. manual trades, restart races).
            let exchange_open = bybit
                .count_open_exchange_positions(&pair_refs)
                .await;
            if exchange_open >= MAX_OPEN_POSITIONS {
                log::warn!(
                    "Exchange already has {} open positions (max {}). Skipping {} pending order(s).",
                    exchange_open, MAX_OPEN_POSITIONS, pending_orders.len()
                );
                pending_orders.clear();
            }

            // Respect the global position cap even if multiple signals fired this cycle
            let slots_available = MAX_OPEN_POSITIONS.saturating_sub(positions.len());
            let order_handles: Vec<_> = pending_orders
                .into_iter()
                .take(slots_available)
                .map(|(symbol, sig, side, price_dec)| {
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
                                price_dec,
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

            // Release unused memory pages back to the OS
            #[cfg(feature = "jemalloc")]
            jemalloc_purge();
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
/// Uses a single REST call to fetch all open positions (no per-symbol loop).
/// - Orphan (exchange open, no local state) → imports into local state.
/// - Stale (local state, exchange size=0) → clears local state.
/// - Size mismatch → updates local qty to match exchange.
async fn reconcile_positions(
    bybit: &bybit_api::BybitClient,
    local_positions: &mut HashMap<String, OpenPosition>,
    _symbols: &[&str],
) {
    log::info!("Reconciling positions with exchange (single call)…");
    let exchange_positions = match bybit.get_all_open_positions().await {
        Ok(map) => map,
        Err(e) => {
            log::warn!("Reconcile failed to fetch positions: {} — skipping.", e);
            return;
        }
    };

    // Stale locals: in local state but size=0 on exchange
    let stale: Vec<String> = local_positions
        .keys()
        .filter(|sym| !exchange_positions.contains_key(*sym))
        .cloned()
        .collect();
    for sym in stale {
        log::warn!("[{}] Local position exists but exchange size=0. Clearing.", sym);
        local_positions.remove(&sym);
    }

    // Size mismatch or orphan: positions on exchange
    for (symbol, info) in exchange_positions {
        match local_positions.get_mut(&symbol) {
            Some(local) => {
                if (local.data.position_size - info.size).abs() > 0.001 {
                    log::warn!(
                        "[{}] Size mismatch: local={:.4}, exchange={:.4}. Using exchange.",
                        symbol, local.data.position_size, info.size
                    );
                    local.data.position_size = info.size;
                }
            }
            None => {
                log::warn!(
                    "[{}] Orphan position imported: {} size={:.4} @ {:.2}",
                    symbol, info.side, info.size, info.avg_price
                );
                local_positions.insert(
                    symbol.clone(),
                    orphan_to_open_position(&symbol, info),
                );
            }
        }
    }

    log::info!("Position reconciliation complete ({} open).", local_positions.len());
}

/// Build an OpenPosition from exchange data when no local state exists.
fn orphan_to_open_position(symbol: &str, info: ExchangePositionInfo) -> OpenPosition {
    let sl = if info.stop_loss > 0.0 { info.stop_loss } else {
        // Fallback: SL 5% away from entry in the opposite direction
        if info.side == "Buy" {
            info.avg_price * 0.95
        } else {
            info.avg_price * 1.05
        }
    };
    let tp = if info.take_profit > 0.0 { info.take_profit } else {
        if info.side == "Buy" {
            info.avg_price * 1.10
        } else {
            info.avg_price * 0.90
        }
    };
    log::info!(
        "[{}] Imported {} @ {:.2} | sl={:.2} tp={:.2} qty={:.4}",
        symbol, info.side, info.avg_price, sl, tp, info.size
    );
    OpenPosition {
        side: info.side,
        data: PositionData {
            is_open:                   true,
            entry_price:               info.avg_price,
            actual_entry:              Some(info.avg_price),
            entry_time:                info.created_time,
            position_size:             info.size,
            stop_loss:                 sl,
            take_profit_1:             tp,
            take_profit_2:             tp,
            unrealized_pnl:            0.0,
            risk_amount:               0.0,
            max_favorable_excursion:   info.avg_price,
            order_id:                  String::new(),
            actual_exit:               None,
        },
    }
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
