/// FVG Backtester — lee data/*.csv, simula la estrategia vela a vela
/// Run: cargo run --bin backtest --release
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;

// ── Constantes ────────────────────────────────────────────────────────────────
const SYMBOLS:            &[&str] = &["BTCUSDT","ETHUSDT","BNBUSDT","XRPUSDT","SOLUSDT"];
const INITIAL_BALANCE:    f64   = 10_000.0;
const MAX_RISK_PCT:       f64   = 0.03;
const MAX_DAILY_LOSS_PCT: f64   = 0.05;
const EQUITY_FLOOR_PCT:   f64   = 0.90;
const ATR_PERIOD:         usize = 14;
const VOL_AVG_PERIOD:     usize = 20;

// ── Parámetros optimizados por símbolo (resultado del grid search) ─────────────
struct SymbolParams {
    min_gap_pct:  f64,
    min_vol_mult: f64,
    fvg_lookback: usize,
    sl_atr_mult:  f64,
    tp_mult:      f64,
    time_stop:    usize, // candles (4H c/u)
}

fn symbol_params(symbol: &str) -> SymbolParams {
    match symbol {
        "BTCUSDT" => SymbolParams { min_gap_pct: 0.001, min_vol_mult: 1.5, fvg_lookback: 12, sl_atr_mult: 0.5, tp_mult: 5.0, time_stop:  7 },
        "ETHUSDT" => SymbolParams { min_gap_pct: 0.001, min_vol_mult: 1.5, fvg_lookback:  8, sl_atr_mult: 1.0, tp_mult: 3.0, time_stop:  7 },
        "BNBUSDT" => SymbolParams { min_gap_pct: 0.003, min_vol_mult: 1.0, fvg_lookback: 12, sl_atr_mult: 2.0, tp_mult: 2.5, time_stop: 35 },
        "XRPUSDT" => SymbolParams { min_gap_pct: 0.008, min_vol_mult: 1.0, fvg_lookback:  8, sl_atr_mult: 2.0, tp_mult: 1.5, time_stop: 14 },
        "SOLUSDT" => SymbolParams { min_gap_pct: 0.008, min_vol_mult: 1.2, fvg_lookback: 12, sl_atr_mult: 1.5, tp_mult: 4.0, time_stop:  7 },
        _         => SymbolParams { min_gap_pct: 0.003, min_vol_mult: 1.2, fvg_lookback:  8, sl_atr_mult: 1.0, tp_mult: 2.0, time_stop:  7 },
    }
}

// ── Tipos ─────────────────────────────────────────────────────────────────────
#[derive(Clone, Debug)]
struct Candle { ts_ms: i64, open: f64, high: f64, low: f64, close: f64, volume: f64 }

#[derive(Clone, Debug, PartialEq)]
enum Side { Long, Short }

#[derive(Clone, Debug)]
struct Trade {
    symbol: String, side: Side,
    entry_ts: i64, exit_ts: i64,
    entry: f64, exit: f64, qty: f64, sl: f64, tp1: f64,
    pnl: f64, pnl_pct: f64, reason: String,
}

struct Position {
    side: Side, entry: f64, sl: f64, tp1: f64,
    qty: f64, entry_candle: usize,
}

// ── CSV loader ────────────────────────────────────────────────────────────────
fn load_csv(path: &Path) -> Vec<Candle> {
    let mut out = Vec::with_capacity(9000);
    for (i, line) in BufReader::new(File::open(path).expect("CSV not found")).lines().enumerate() {
        let line = line.unwrap();
        if i == 0 { continue; }
        let f: Vec<&str> = line.split(',').collect();
        if f.len() < 7 { continue; }
        out.push(Candle {
            ts_ms:  f[0].parse().unwrap_or(0),
            open:   f[2].parse().unwrap_or(0.0),
            high:   f[3].parse().unwrap_or(0.0),
            low:    f[4].parse().unwrap_or(0.0),
            close:  f[5].parse().unwrap_or(0.0),
            volume: f[6].parse().unwrap_or(0.0),
        });
    }
    out.sort_by_key(|c| c.ts_ms);
    out
}

// ── Indicadores ───────────────────────────────────────────────────────────────
fn calc_atr(candles: &[Candle], period: usize) -> f64 {
    if candles.len() < period + 1 { return 0.0; }
    let start = candles.len() - period - 1;
    ((start + 1)..candles.len()).map(|i| {
        let c = &candles[i]; let p = &candles[i-1];
        (c.high - c.low).max((c.high - p.close).abs()).max((c.low - p.close).abs())
    }).sum::<f64>() / period as f64
}

fn calc_avg_vol(candles: &[Candle], period: usize) -> f64 {
    let n = candles.len().min(period);
    candles.iter().rev().take(n).map(|c| c.volume).sum::<f64>() / n as f64
}

// ── Detección FVG 3 velas (válida para futuros perpetuos sin gaps) ─────────────
//
// Bullish FVG:  c1.high < c3.low   →  zona = [c1.high, c3.low]
//               c2 es la vela impulso (body grande hacia arriba)
//
// Bearish FVG:  c3.high < c1.low   →  zona = [c3.high, c1.low]
//               c2 es la vela impulso (body grande hacia abajo)
//
struct Fvg { side: Side, zone_low: f64, zone_high: f64 }

fn find_signal(candles: &[Candle], i: usize, p: &SymbolParams) -> Option<Fvg> {
    let current  = &candles[i];
    let avg_vol  = calc_avg_vol(&candles[..=i], VOL_AVG_PERIOD);

    let search_start = i.saturating_sub(p.fvg_lookback + 2);

    for j in search_start..i.saturating_sub(2) {
        let c1 = &candles[j];
        let c2 = &candles[j + 1]; // impulso
        let c3 = &candles[j + 2];
        let imp_vol = calc_avg_vol(&candles[..=j + 1], VOL_AVG_PERIOD);

        // ── Bullish FVG ───────────────────────────────────────────────────────
        if c3.low > c1.high {
            let gap = c3.low - c1.high;
            if gap > c2.close * p.min_gap_pct
                && c2.close > c2.open
                && c2.volume > imp_vol * p.min_vol_mult
            {
                let zl = c1.high;
                let zh = c3.low;
                let retested = candles[j + 3..=i].iter()
                    .any(|c| c.low <= zh + (zh - zl) * 0.5);
                if retested
                    && current.close > zh
                    && current.volume > avg_vol * p.min_vol_mult
                {
                    return Some(Fvg { side: Side::Long, zone_low: zl, zone_high: zh });
                }
            }
        }

        // ── Bearish FVG ───────────────────────────────────────────────────────
        if c1.low > c3.high {
            let gap = c1.low - c3.high;
            if gap > c2.close * p.min_gap_pct
                && c2.close < c2.open
                && c2.volume > imp_vol * p.min_vol_mult
            {
                let zl = c3.high;
                let zh = c1.low;
                let retested = candles[j + 3..=i].iter()
                    .any(|c| c.high >= zl - (zh - zl) * 0.5);
                if retested
                    && current.close < zl
                    && current.volume > avg_vol * p.min_vol_mult
                {
                    return Some(Fvg { side: Side::Short, zone_low: zl, zone_high: zh });
                }
            }
        }
    }
    None
}

// ── Backtest por símbolo ──────────────────────────────────────────────────────
fn backtest_symbol(symbol: &str, candles: &[Candle]) -> Vec<Trade> {
    let p = symbol_params(symbol);
    let mut trades: Vec<Trade> = Vec::new();
    let mut balance  = INITIAL_BALANCE;
    let mut position: Option<Position> = None;

    let mut current_day: i64 = -1;
    let mut daily_pnl   = 0.0_f64;
    let mut trading_on  = true;

    let min_i = ATR_PERIOD + VOL_AVG_PERIOD + 3;

    for i in min_i..candles.len() {
        let candle = &candles[i];

        // ── Reset diario ──────────────────────────────────────────────────────
        let day = candle.ts_ms / 86_400_000;
        if day != current_day {
            current_day = day;
            daily_pnl = 0.0;
            trading_on = balance >= INITIAL_BALANCE * EQUITY_FLOOR_PCT;
        }

        let cur_atr = calc_atr(&candles[..=i], ATR_PERIOD);

        // ── Gestión de posición abierta ───────────────────────────────────────
        if let Some(ref pos) = position {
            let sl_hit = match pos.side {
                Side::Long  => candle.low  <= pos.sl,
                Side::Short => candle.high >= pos.sl,
            };
            let tp_hit = match pos.side {
                Side::Long  => candle.high >= pos.tp1,
                Side::Short => candle.low  <= pos.tp1,
            };
            let time_stop = (i - pos.entry_candle) >= p.time_stop;

            let (close_price, reason) = if sl_hit {
                (pos.sl, "SL")
            } else if tp_hit {
                (pos.tp1, "TP1")
            } else if time_stop {
                (candle.close, "TimeStop")
            } else {
                continue;
            };

            let mult = match pos.side { Side::Long => 1.0, Side::Short => -1.0 };
            let pnl  = (close_price - pos.entry) * pos.qty * mult;
            let pnl_pct = pnl / balance * 100.0;
            balance   += pnl;
            daily_pnl += pnl;

            trades.push(Trade {
                symbol: symbol.to_string(), side: pos.side.clone(),
                entry_ts: candles[pos.entry_candle].ts_ms, exit_ts: candle.ts_ms,
                entry: pos.entry, exit: close_price, qty: pos.qty,
                sl: pos.sl, tp1: pos.tp1, pnl, pnl_pct, reason: reason.to_string(),
            });
            position = None;

            if daily_pnl < -(balance.max(INITIAL_BALANCE) * MAX_DAILY_LOSS_PCT) {
                trading_on = false;
            }
            continue;
        }

        if !trading_on || cur_atr == 0.0 { continue; }

        // ── Búsqueda de señal ─────────────────────────────────────────────────
        if let Some(fvg) = find_signal(candles, i, &p) {
            let entry = candle.close;

            let sl = match fvg.side {
                Side::Long  => fvg.zone_low  - cur_atr * p.sl_atr_mult,
                Side::Short => fvg.zone_high + cur_atr * p.sl_atr_mult,
            };

            let risk_unit = (entry - sl).abs();
            if risk_unit <= 0.0 || risk_unit > entry * 0.10 { continue; }

            let tp1 = match fvg.side {
                Side::Long  => entry + risk_unit * p.tp_mult,
                Side::Short => entry - risk_unit * p.tp_mult,
            };

            let max_risk = balance * MAX_RISK_PCT;
            let budget   = (balance * MAX_DAILY_LOSS_PCT + daily_pnl).max(0.0);
            let risk     = max_risk.min(budget);
            if risk <= 0.0 { continue; }

            let qty = (risk / risk_unit).floor();
            if qty <= 0.0 { continue; }

            position = Some(Position {
                side: fvg.side, entry, sl, tp1, qty, entry_candle: i,
            });
        }
    }

    trades
}

// ── Estadísticas ──────────────────────────────────────────────────────────────
struct Stats {
    symbol: String, trades: usize, wins: usize, losses: usize,
    win_rate: f64, total_pnl: f64, total_pnl_pct: f64,
    avg_win: f64, avg_loss: f64, profit_factor: f64,
    max_drawdown: f64, best: f64, worst: f64,
}

fn compute_stats(symbol: &str, trades: &[Trade]) -> Stats {
    if trades.is_empty() {
        return Stats { symbol: symbol.to_string(), trades: 0, wins: 0, losses: 0,
            win_rate: 0.0, total_pnl: 0.0, total_pnl_pct: 0.0,
            avg_win: 0.0, avg_loss: 0.0, profit_factor: 0.0,
            max_drawdown: 0.0, best: 0.0, worst: 0.0 };
    }
    let wins: Vec<f64>  = trades.iter().filter(|t| t.pnl > 0.0).map(|t| t.pnl).collect();
    let losses: Vec<f64> = trades.iter().filter(|t| t.pnl <= 0.0).map(|t| t.pnl.abs()).collect();
    let gross_win: f64  = wins.iter().sum();
    let gross_loss: f64 = losses.iter().sum();
    let total_pnl: f64  = trades.iter().map(|t| t.pnl).sum();

    let mut bal = INITIAL_BALANCE;
    let mut peak = INITIAL_BALANCE;
    let mut max_dd = 0.0_f64;
    for t in trades {
        bal += t.pnl;
        if bal > peak { peak = bal; }
        let dd = (peak - bal) / peak * 100.0;
        if dd > max_dd { max_dd = dd; }
    }

    Stats {
        symbol: symbol.to_string(),
        trades: trades.len(), wins: wins.len(), losses: losses.len(),
        win_rate: wins.len() as f64 / trades.len() as f64 * 100.0,
        total_pnl, total_pnl_pct: total_pnl / INITIAL_BALANCE * 100.0,
        avg_win:  if wins.is_empty()   { 0.0 } else { gross_win  / wins.len() as f64 },
        avg_loss: if losses.is_empty() { 0.0 } else { gross_loss / losses.len() as f64 },
        profit_factor: if gross_loss == 0.0 { f64::INFINITY } else { gross_win / gross_loss },
        max_drawdown: max_dd,
        best:  trades.iter().map(|t| t.pnl).fold(f64::NEG_INFINITY, f64::max),
        worst: trades.iter().map(|t| t.pnl).fold(f64::INFINITY,     f64::min),
    }
}

fn print_stats(s: &Stats) {
    let verdict = if s.win_rate >= 55.0 && s.profit_factor >= 1.5 { "✅ APTO" }
                  else if s.win_rate >= 50.0 { "⚠️  MARGINAL" }
                  else { "❌ NO APTO" };
    println!();
    println!("  ┌─────────────────────────────────────────────┐");
    println!("  │  {:12}                    {}  │", s.symbol, verdict);
    println!("  ├─────────────────────────────────────────────┤");
    println!("  │  Trades         {:>6}   ({} W / {} L)", s.trades, s.wins, s.losses);
    println!("  │  Win Rate       {:>6.1}%", s.win_rate);
    println!("  │  Total PnL      {:>+9.2} USDT  ({:+.1}%)", s.total_pnl, s.total_pnl_pct);
    println!("  │  Avg Win        {:>+9.2} USDT", s.avg_win);
    println!("  │  Avg Loss       {:>+9.2} USDT", -s.avg_loss);
    println!("  │  Profit Factor  {:>9.2}", s.profit_factor);
    println!("  │  Max Drawdown   {:>6.1}%", s.max_drawdown);
    println!("  │  Best Trade     {:>+9.2} USDT", s.best);
    println!("  │  Worst Trade    {:>+9.2} USDT", s.worst);
    println!("  └─────────────────────────────────────────────┘");
}

fn print_global(all_trades: &[Trade]) {
    let s = compute_stats("ALL", all_trades);
    let verdict = if s.win_rate >= 55.0 && s.profit_factor >= 1.5 { "✅ APTO PARA LIVE" }
                  else if s.win_rate >= 50.0 { "⚠️  REVISAR PARAMETROS" }
                  else { "❌ NO INICIAR LIVE" };
    println!();
    println!("  ╔══════════════════════════════════════════════════╗");
    println!("  ║  RESULTADO GLOBAL — {} PARES  {}  ║", SYMBOLS.len(), verdict);
    println!("  ╠══════════════════════════════════════════════════╣");
    println!("  ║  Trades         {:>6}   ({} W / {} L)", s.trades, s.wins, s.losses);
    println!("  ║  Win Rate       {:>6.1}%", s.win_rate);
    println!("  ║  Total PnL      {:>+9.2} USDT  ({:+.1}%)", s.total_pnl, s.total_pnl_pct);
    println!("  ║  Avg Win        {:>+9.2} USDT", s.avg_win);
    println!("  ║  Avg Loss       {:>+9.2} USDT", -s.avg_loss);
    println!("  ║  Profit Factor  {:>9.2}", s.profit_factor);
    println!("  ║  Max Drawdown   {:>6.1}%", s.max_drawdown);
    println!("  ║  Best Trade     {:>+9.2} USDT", s.best);
    println!("  ║  Worst Trade    {:>+9.2} USDT", s.worst);
    println!("  ╚══════════════════════════════════════════════════╝");

    let mut reasons: HashMap<&str, (usize, f64)> = HashMap::new();
    for t in all_trades {
        let e = reasons.entry(t.reason.as_str()).or_insert((0, 0.0));
        e.0 += 1; e.1 += t.pnl;
    }
    println!();
    println!("  Salidas por tipo:");
    let mut rv: Vec<_> = reasons.iter().collect();
    rv.sort_by_key(|(k,_)| *k);
    for (r, (n, pnl)) in &rv {
        let pct = *n as f64 / s.trades as f64 * 100.0;
        println!("    {:<12}  {:>5} trades ({:>4.1}%)   {:>+9.2} USDT", r, n, pct, pnl);
    }
}

// ── Trade log CSV ─────────────────────────────────────────────────────────────
fn save_trades(trades: &[Trade], path: &Path) {
    let mut f = File::create(path).expect("no se pudo crear trade log");
    writeln!(f, "symbol,side,entry_date,exit_date,entry,exit,qty,sl,tp1,pnl,pnl_pct,reason").unwrap();
    for t in trades {
        let side = match t.side { Side::Long => "Long", Side::Short => "Short" };
        writeln!(f, "{},{},{},{},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{}",
            t.symbol, side, ms_to_date(t.entry_ts), ms_to_date(t.exit_ts),
            t.entry, t.exit, t.qty, t.sl, t.tp1, t.pnl, t.pnl_pct, t.reason
        ).unwrap();
    }
}

fn ms_to_date(ms: i64) -> String {
    let s = ms / 1000;
    let (hh, mm) = ((s / 3600) % 24, (s / 60) % 60);
    let mut days = s / 86400;
    let mut y = 1970i32;
    loop {
        let dy = if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) { 366 } else { 365 };
        if days < dy { break; }
        days -= dy; y += 1;
    }
    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let dm = if leap { [31,29,31,30,31,30,31,31,30,31,30,31] }
             else    { [31,28,31,30,31,30,31,31,30,31,30,31] };
    let mut mo = 1;
    for d in &dm { if days < *d { break; } days -= *d; mo += 1; }
    format!("{:04}-{:02}-{:02} {:02}:{:02}", y, mo, days + 1, hh, mm)
}

// ── Main ──────────────────────────────────────────────────────────────────────
fn main() {
    let data_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("data");

    println!("\n╔═══════════════════════════════════════════════════════╗");
    println!("║          FVG BACKTESTER  —  4 años  —  velas 4H      ║");
    println!("║  Capital: ${}   Riesgo: {}%   Max DD diario: {}%   ║",
             INITIAL_BALANCE as u32, (MAX_RISK_PCT*100.0) as u32,
             (MAX_DAILY_LOSS_PCT*100.0) as u32);
    println!("╚═══════════════════════════════════════════════════════╝");

    let mut all_trades: Vec<Trade> = Vec::new();

    for &symbol in SYMBOLS {
        let csv = data_dir.join(format!("{}_4H.csv", symbol));
        if !csv.exists() { eprintln!("  ⚠  No existe: {:?}", csv); continue; }

        print!("  {} … cargando", symbol);
        let candles = load_csv(&csv);
        println!(" {} velas  →  ejecutando …", candles.len());

        let trades = backtest_symbol(symbol, &candles);
        let stats  = compute_stats(symbol, &trades);
        print_stats(&stats);
        all_trades.extend(trades);
    }

    print_global(&all_trades);

    let log = data_dir.join("backtest_trades.csv");
    save_trades(&all_trades, &log);
    println!("\n  📄 Trade log guardado: {:?}\n", log);
}
