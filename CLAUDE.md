# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Rust async trading bot implementing a Fair Value Gap (FVG) strategy for Bybit perpetual futures.
Targets HyroTrader prop-firm compliance. Currently wired to the **Bybit Demo** environment.

## Build & Run

```sh
cargo check          # type-check without building
RUST_LOG=info cargo run   # run with logging
cargo build --release --features jemalloc   # production binary (siempre usar)
systemctl restart fvg_trader                # aplicar nuevo binario
```

> **Regla de oro**: Siempre compilar con `--release --features jemalloc` para producción.
> Las flags `lto="fat"`, `codegen-units=1`, `panic="abort"`, `target-cpu=native` en `Cargo.toml`
> garantizan la mínima latencia posible. Nunca usar el binario `debug` en el servidor.

## Module Structure

```
src/
├── main.rs              # Entry point: initialises all modules, 60-second main loop
├── config.rs            # All constants: API keys, Telegram token, strategy params
├── types.rs             # Shared structs (Candle, FVGZone, TradeSignal, RiskMetrics…)
├── fvg_detector.rs      # Bullish/bearish FVG detection + breakout + fill checks
├── position_manager.rs  # Position sizing, SL/TP calculation, HyroTrader validation
├── bybit_api.rs         # Bybit REST client (HMAC-SHA256): place_order / close_position
├── websocket_handler.rs # Bybit public WS feed (wss://stream.bybit.com) with reconnect
└── telegram.rs          # Telegram Bot notifications (start / open / close / daily PnL)
```

## Data Flow

```
Bybit Public WS  →  VecDeque<Candle> (Arc<Mutex>)
                         │
          main loop (60 s poll)
                         │
            ┌────────────┴────────────┐
       FVG detector            Position manager
       (bullish / bearish)     (SL, TP, sizing)
            └────────────┬────────────┘
                  bybit_api (Demo REST)
                  telegram  (Telegram Bot)
```

## Key Endpoints

| Purpose | Value |
|---|---|
| REST (Demo) | `https://api-demo.bybit.com` |
| Public WS   | `wss://stream.bybit.com/v5/public/linear` |
| Order route | `POST /v5/order/create` |
| Position    | `GET /v5/position/list` |

Bybit V5 auth: `X-BAPI-API-KEY`, `X-BAPI-TIMESTAMP`, `X-BAPI-SIGN`, `X-BAPI-RECV-WINDOW`
Signature payload: `timestamp + apiKey + recvWindow + body`

## Strategy Parameters (config.rs)

| Param | Value |
|---|---|
| Capital | $10,000 USDT |
| Max risk/trade | 3 % ($300) |
| Daily max loss | 5 % ($500) |
| Equity floor | 90 % |
| Pair / TF | BTCUSDT / 4H |
| FVG min gap | 0.5 % of price |
| Volume filter | ≥ 120 % of 20-candle avg |

## Telegram Notifications

Events sent automatically: bot start, trade opened, trade closed (with PnL), daily summary at UTC midnight, risk alerts.

## Reference

Full strategy theory, compliance rules, and backtesting guidance: `FVG_TRADING_STRATEGY_RUST_HYROTRADER.md`
