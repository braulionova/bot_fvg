/// Optimizador de parámetros FVG — grid search por símbolo
/// Run: cargo run --bin optimize --release
use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;

// ── Constantes fijas (no optimizables) ────────────────────────────────────────
const SYMBOLS: &[&str]  = &["BTCUSDT","ETHUSDT","BNBUSDT","XRPUSDT","SOLUSDT"];
const INITIAL_BALANCE: f64 = 10_000.0;
const MAX_RISK_PCT: f64    = 0.03;
const MAX_DAILY_LOSS_PCT: f64 = 0.05;
const EQUITY_FLOOR_PCT: f64  = 0.90;
const ATR_PERIOD: usize  = 14;
const VOL_AVG_PERIOD: usize = 20;
const MIN_TRADES: usize  = 15; // mínimo para ser estadísticamente relevante

// ── Grid de búsqueda ──────────────────────────────────────────────────────────
const GRID_GAP:      &[f64]   = &[0.001, 0.002, 0.003, 0.005, 0.008];
const GRID_VOL:      &[f64]   = &[1.0, 1.2, 1.5, 2.0];
const GRID_LOOKBACK: &[usize] = &[3, 5, 8, 12];
const GRID_SL_ATR:   &[f64]   = &[0.5, 1.0, 1.5, 2.0];
const GRID_TP:       &[f64]   = &[1.5, 2.0, 2.5, 3.0, 4.0, 5.0];
const GRID_TSTOP:    &[usize] = &[5, 7, 10, 14, 20, 28, 35];

// ── Datos ─────────────────────────────────────────────────────────────────────
#[derive(Clone)]
struct Candle { ts_ms: i64, open: f64, high: f64, low: f64, close: f64, volume: f64 }

#[derive(Clone)]
struct Params {
    min_gap:      f64,
    min_vol_mult: f64,
    lookback:     usize,
    sl_atr_mult:  f64,
    tp_mult:      f64,
    time_stop:    usize,
}

#[derive(Clone)]
struct Result {
    params:        Params,
    trades:        usize,
    win_rate:      f64,
    profit_factor: f64,
    total_pnl:     f64,
    max_drawdown:  f64,
    score:         f64,
}

// ── CSV loader ────────────────────────────────────────────────────────────────
fn load_csv(path: &Path) -> Vec<Candle> {
    let mut out = Vec::with_capacity(9000);
    for (i, line) in BufReader::new(File::open(path).unwrap()).lines().enumerate() {
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
#[inline]
fn calc_atr(candles: &[Candle], period: usize) -> f64 {
    if candles.len() < period + 1 { return 0.0; }
    let start = candles.len() - period - 1;
    ((start + 1)..candles.len()).map(|i| {
        let c = &candles[i]; let p = &candles[i-1];
        (c.high - c.low).max((c.high - p.close).abs()).max((c.low - p.close).abs())
    }).sum::<f64>() / period as f64
}

#[inline]
fn calc_avg_vol(candles: &[Candle], period: usize) -> f64 {
    let n = candles.len().min(period);
    candles.iter().rev().take(n).map(|c| c.volume).sum::<f64>() / n as f64
}

// ── Detección FVG con parámetros ──────────────────────────────────────────────
fn find_signal(candles: &[Candle], i: usize, p: &Params) -> Option<(bool, f64, f64)> {
    // returns (is_long, zone_low, zone_high)
    let current = &candles[i];
    let avg_vol = calc_avg_vol(&candles[..=i], VOL_AVG_PERIOD);

    let search_start = i.saturating_sub(p.lookback + 2);

    for j in search_start..i.saturating_sub(2) {
        let c1 = &candles[j];
        let c2 = &candles[j + 1];
        let c3 = &candles[j + 2];
        let imp_vol = calc_avg_vol(&candles[..=j + 1], VOL_AVG_PERIOD);

        // ── Bullish FVG ───────────────────────────────────────────────────────
        if c3.low > c1.high {
            let gap = c3.low - c1.high;
            if gap > c2.close * p.min_gap
                && c2.close > c2.open
                && c2.volume > imp_vol * p.min_vol_mult
            {
                let zl = c1.high; let zh = c3.low;
                let retested = candles[j+3..=i].iter()
                    .any(|c| c.low <= zh + (zh - zl) * 0.5);
                if retested && current.close > zh
                    && current.volume > avg_vol * p.min_vol_mult {
                    return Some((true, zl, zh));
                }
            }
        }

        // ── Bearish FVG ───────────────────────────────────────────────────────
        if c1.low > c3.high {
            let gap = c1.low - c3.high;
            if gap > c2.close * p.min_gap
                && c2.close < c2.open
                && c2.volume > imp_vol * p.min_vol_mult
            {
                let zl = c3.high; let zh = c1.low;
                let retested = candles[j+3..=i].iter()
                    .any(|c| c.high >= zl - (zh - zl) * 0.5);
                if retested && current.close < zl
                    && current.volume > avg_vol * p.min_vol_mult {
                    return Some((false, zl, zh));
                }
            }
        }
    }
    None
}

// ── Backtest parametrizado ────────────────────────────────────────────────────
fn run_backtest(candles: &[Candle], p: &Params) -> (usize, f64, f64, f64, f64) {
    // returns (trades, win_rate, profit_factor, total_pnl, max_drawdown)
    let mut balance = INITIAL_BALANCE;
    let mut open: Option<(bool, f64, f64, f64, f64, usize)> = None;
    // (is_long, entry, sl, tp1, qty, entry_idx)

    let mut wins = 0usize; let mut losses = 0usize;
    let mut gross_win = 0.0f64; let mut gross_loss = 0.0f64;
    let mut current_day = -1i64; let mut daily_pnl = 0.0f64;
    let mut trading_on = true;
    let mut peak = INITIAL_BALANCE; let mut max_dd = 0.0f64;

    let min_i = ATR_PERIOD + VOL_AVG_PERIOD + 3;

    for i in min_i..candles.len() {
        let c = &candles[i];
        let day = c.ts_ms / 86_400_000;
        if day != current_day {
            current_day = day; daily_pnl = 0.0;
            trading_on = balance >= INITIAL_BALANCE * EQUITY_FLOOR_PCT;
        }

        let atr = calc_atr(&candles[..=i], ATR_PERIOD);

        if let Some((is_long, entry, sl, tp1, qty, entry_idx)) = open {
            let sl_hit  = if is_long { c.low  <= sl  } else { c.high >= sl  };
            let tp_hit  = if is_long { c.high >= tp1 } else { c.low  <= tp1 };
            let time_ok = (i - entry_idx) >= p.time_stop;

            let close_p = if sl_hit { sl } else if tp_hit { tp1 } else if time_ok { c.close } else { continue; };
            let mult = if is_long { 1.0 } else { -1.0 };
            let pnl  = (close_p - entry) * qty * mult;
            balance   += pnl; daily_pnl += pnl;
            if pnl > 0.0 { wins += 1; gross_win  += pnl; }
            else         { losses += 1; gross_loss += pnl.abs(); }
            if balance > peak { peak = balance; }
            let dd = (peak - balance) / peak * 100.0;
            if dd > max_dd { max_dd = dd; }
            open = None;
            if daily_pnl < -(balance.max(INITIAL_BALANCE) * MAX_DAILY_LOSS_PCT) { trading_on = false; }
            continue;
        }

        if !trading_on || atr == 0.0 { continue; }

        if let Some((is_long, _zl, _zh)) = find_signal(candles, i, p) {
            let entry = c.close;
            let sl = if is_long { _zl - atr * p.sl_atr_mult } else { _zh + atr * p.sl_atr_mult };
            let risk_unit = (entry - sl).abs();
            if risk_unit <= 0.0 || risk_unit > entry * 0.12 { continue; }

            let tp1 = if is_long { entry + risk_unit * p.tp_mult } else { entry - risk_unit * p.tp_mult };
            let max_risk = balance * MAX_RISK_PCT;
            let budget   = (balance * MAX_DAILY_LOSS_PCT + daily_pnl).max(0.0);
            let risk     = max_risk.min(budget);
            if risk <= 0.0 { continue; }
            let qty = (risk / risk_unit).floor();
            if qty <= 0.0 { continue; }

            open = Some((is_long, entry, sl, tp1, qty, i));
        }
    }

    let n = wins + losses;
    if n == 0 { return (0, 0.0, 0.0, 0.0, 0.0); }
    let wr = wins as f64 / n as f64 * 100.0;
    let pf = if gross_loss == 0.0 { 99.0 } else { gross_win / gross_loss };
    let total = balance - INITIAL_BALANCE;
    (n, wr, pf, total, max_dd)
}

// ── Función de puntuación ─────────────────────────────────────────────────────
fn score(wr: f64, pf: f64, dd: f64, trades: usize) -> f64 {
    if trades < MIN_TRADES || wr < 40.0 || pf < 0.5 { return 0.0; }
    let wr_norm = (wr / 100.0).powf(2.0);        // premia win rate alto
    let pf_norm  = (pf / 3.0).min(1.0);          // premia profit factor
    let dd_pen   = 1.0 - (dd / 100.0).min(0.99); // penaliza drawdown
    let freq     = (trades as f64 / 200.0).min(1.0); // premia más trades
    wr_norm * pf_norm * dd_pen * freq * 1000.0
}

// ── Optimización por símbolo ──────────────────────────────────────────────────
fn optimize_symbol(symbol: &str, candles: &[Candle]) -> Vec<Result> {
    let total = GRID_GAP.len() * GRID_VOL.len() * GRID_LOOKBACK.len()
              * GRID_SL_ATR.len() * GRID_TP.len() * GRID_TSTOP.len();
    let mut results: Vec<Result> = Vec::with_capacity(total / 5);
    let mut done = 0usize;

    for &gap in GRID_GAP {
    for &vol in GRID_VOL {
    for &lb in GRID_LOOKBACK {
    for &sl_a in GRID_SL_ATR {
    for &tp in GRID_TP {
    for &ts in GRID_TSTOP {
        let p = Params { min_gap: gap, min_vol_mult: vol, lookback: lb,
                         sl_atr_mult: sl_a, tp_mult: tp, time_stop: ts };
        let (n, wr, pf, pnl, dd) = run_backtest(candles, &p);
        let sc = score(wr, pf, dd, n);
        if sc > 0.0 {
            results.push(Result { params: p, trades: n, win_rate: wr,
                profit_factor: pf, total_pnl: pnl, max_drawdown: dd, score: sc });
        }
        done += 1;
        if done % 500 == 0 {
            eprint!("\r    {}/{} combinaciones ({:.0}%)   ", done, total,
                    done as f64 / total as f64 * 100.0);
        }
    }}}}}}
    eprintln!("\r    {} combinaciones probadas               ", total);
    results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());
    results
}

// ── Output CSV con mejores parámetros ─────────────────────────────────────────
fn save_best(results_per_sym: &[(&str, &Result)], path: &Path) {
    let mut f = File::create(path).unwrap();
    writeln!(f, "symbol,min_gap,min_vol_mult,lookback,sl_atr_mult,tp_mult,time_stop,\
                 trades,win_rate,profit_factor,total_pnl,max_drawdown,score").unwrap();
    for (sym, r) in results_per_sym {
        let p = &r.params;
        writeln!(f, "{},{},{},{},{},{},{},{},{:.1},{:.3},{:.2},{:.1},{:.2}",
            sym, p.min_gap, p.min_vol_mult, p.lookback, p.sl_atr_mult,
            p.tp_mult, p.time_stop, r.trades, r.win_rate, r.profit_factor,
            r.total_pnl, r.max_drawdown, r.score).unwrap();
    }
}

// ── Main ──────────────────────────────────────────────────────────────────────
fn main() {
    let data_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("data");
    let total_combos = GRID_GAP.len() * GRID_VOL.len() * GRID_LOOKBACK.len()
                     * GRID_SL_ATR.len() * GRID_TP.len() * GRID_TSTOP.len();

    println!("\n╔══════════════════════════════════════════════════════════════╗");
    println!("║        FVG OPTIMIZADOR DE PARÁMETROS — Grid Search          ║");
    println!("║  {} combinaciones × {} símbolos = {} backtests     ║",
             total_combos, SYMBOLS.len(), total_combos * SYMBOLS.len());
    println!("╚══════════════════════════════════════════════════════════════╝\n");

    let mut best_per_sym: Vec<(&str, Result)> = Vec::new();

    for &symbol in SYMBOLS {
        let csv = data_dir.join(format!("{}_4H.csv", symbol));
        if !csv.exists() { eprintln!("  ⚠  No existe: {:?}", csv); continue; }

        let candles = load_csv(&csv);
        println!("  ── {} ({} velas) ──────────────────────────────────────", symbol, candles.len());
        print!("    buscando…");

        let results = optimize_symbol(symbol, &candles);

        if results.is_empty() {
            println!("    Sin resultados válidos.");
            continue;
        }

        // Top 3 para este símbolo
        println!("    Top 3 configuraciones:\n");
        println!("    {:>4}  {:>6}  {:>6}  {:>4}  {:>5}  {:>5}  {:>6}  {:>7}  {:>5}  {:>7}  {:>8}",
                 "Rank","WR%","PF","LB","Gap%","Vol×","TP×","SL×ATR","Stop","PnL","Score");
        println!("    {}", "─".repeat(75));

        for (rank, r) in results.iter().take(3).enumerate() {
            let p = &r.params;
            println!(
                "    {:>4}  {:>5.1}%  {:>5.2}  {:>4}  {:>4.1}%  {:>4.1}×  {:>4.1}×  {:>6.1}  {:>5}  {:>+7.0}  {:>8.1}",
                rank + 1, r.win_rate, r.profit_factor, p.lookback,
                p.min_gap * 100.0, p.min_vol_mult, p.tp_mult, p.sl_atr_mult,
                p.time_stop, r.total_pnl, r.score
            );
        }

        let best = results.into_iter().next().unwrap();
        println!();
        println!("    ✅ MEJOR: WR={:.1}%  PF={:.2}  DD={:.1}%  {} trades  PnL={:+.0}",
                 best.win_rate, best.profit_factor, best.max_drawdown,
                 best.trades, best.total_pnl);
        println!();

        best_per_sym.push((symbol, best));
    }

    // ── Resumen final ─────────────────────────────────────────────────────────
    println!("╔══════════════════════════════════════════════════════════════╗");
    println!("║  PARÁMETROS ÓPTIMOS POR SÍMBOLO                              ║");
    println!("╠══════════════════════════════════════════════════════════════╣");
    println!("  {:12}  {:>5}  {:>5}  {:>4}  {:>5}  {:>5}  {:>5}  {:>6}  {:>4}  {:>8}",
             "Symbol","WR%","PF","LB","Gap%","Vol×","TP×","SL×","Stop","PnL");
    println!("  {}", "─".repeat(72));
    for (sym, r) in &best_per_sym {
        let p = &r.params;
        println!("  {:12}  {:>4.1}%  {:>4.2}  {:>4}  {:>4.1}%  {:>4.1}  {:>4.1}  {:>5.1}  {:>4}  {:>+8.0}",
                 sym, r.win_rate, r.profit_factor, p.lookback,
                 p.min_gap * 100.0, p.min_vol_mult, p.tp_mult, p.sl_atr_mult,
                 p.time_stop, r.total_pnl);
    }
    println!("╚══════════════════════════════════════════════════════════════╝");

    // Guardar CSV con mejores parámetros
    let refs: Vec<(&str, &Result)> = best_per_sym.iter().map(|(s, r)| (*s, r)).collect();
    let out = data_dir.join("optimized_params.csv");
    save_best(&refs, &out);
    println!("\n  📄 Parámetros guardados: {:?}", out);

    // ── Instrucciones para aplicar ────────────────────────────────────────────
    println!("\n  Para aplicar los parámetros óptimos por símbolo, actualiza");
    println!("  config.rs con los valores del símbolo de mayor puntuación,");
    println!("  o edita src/bin/backtest.rs para usarlos individualmente.\n");
}
