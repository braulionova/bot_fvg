// ─── Bybit Demo Account ───────────────────────────────────────────────────────
pub const BYBIT_REST_URL: &str = "https://api-demo.bybit.com";
pub const BYBIT_WS_URL: &str = "wss://stream.bybit.com/v5/public/linear";
// BYBIT_API_KEY, BYBIT_SECRET, TELEGRAM_TOKEN, TELEGRAM_CHAT_ID
// are read from environment variables at runtime (see .env.example)

// ─── Strategy ─────────────────────────────────────────────────────────────────
pub const ACCOUNT_BALANCE: f64 = 10_000.0;
pub const MAX_DAILY_LOSS_PCT: f64 = 0.05;   // 5 %
pub const MAX_RISK_PER_TRADE_PCT: f64 = 0.01; // 1 %  (~$100 USDT)
pub const EQUITY_FLOOR_PCT: f64 = 0.90;      // 90 %

pub const TRADING_PAIRS: &[&str] = &["BTCUSDT", "ETHUSDT", "BNBUSDT", "XRPUSDT", "SOLUSDT"];
pub const MAX_OPEN_POSITIONS: usize = 2;
pub const KLINE_INTERVAL: &str = "240"; // 4-hour candles (Bybit V5: minutes)

// ─── Parámetros optimizados por símbolo (resultado del grid search) ───────────
// Generados por: cargo run --bin optimize --release
// Criterio: maximizar win_rate × profit_factor × (1 − max_drawdown)
//
//            Symbol  WR%    PF    LB  Gap%  Vol×  TP×  SL×  Stop
//           BTCUSDT  55.6  3.06   12  0.1%  1.5   5.0  0.5    7
//           ETHUSDT  52.9  1.47    8  0.1%  1.5   3.0  1.0    7
//           BNBUSDT  54.7  1.46   12  0.3%  1.0   2.5  2.0   35
//           XRPUSDT  56.8  1.49    8  0.8%  1.0   1.5  2.0   14
//           SOLUSDT  57.1  1.83   12  0.8%  1.2   4.0  1.5    7

pub struct SymbolParams {
    pub min_gap_pct:   f64,   // mínimo tamaño del gap FVG como % del precio
    pub min_vol_mult:  f64,   // multiplicador de volumen mínimo
    pub fvg_lookback:  usize, // velas hacia atrás para buscar FVG
    pub sl_atr_mult:   f64,   // multiplicador ATR para el stop-loss
    pub tp_mult:       f64,   // ratio riesgo:recompensa
    pub time_stop:     usize, // velas máximas en posición
}

pub const fn params(
    min_gap_pct: f64, min_vol_mult: f64, fvg_lookback: usize,
    sl_atr_mult: f64, tp_mult: f64, time_stop: usize,
) -> SymbolParams {
    SymbolParams { min_gap_pct, min_vol_mult, fvg_lookback, sl_atr_mult, tp_mult, time_stop }
}

pub fn symbol_params(symbol: &str) -> SymbolParams {
    match symbol {
        "BTCUSDT" => params(0.001, 1.5, 12, 0.5, 5.0,  7),
        "ETHUSDT" => params(0.001, 1.5,  8, 1.0, 3.0,  7),
        "BNBUSDT" => params(0.003, 1.0, 12, 2.0, 2.5, 35),
        "XRPUSDT" => params(0.008, 1.0,  8, 2.0, 1.5, 14),
        "SOLUSDT" => params(0.008, 1.2, 12, 1.5, 4.0,  7),
        _         => params(0.003, 1.2,  8, 1.0, 2.0,  7), // fallback
    }
}
