use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use tokio::time::{interval, Duration};
use tokio_tungstenite::{connect_async, tungstenite::Message};

use crate::types::Candle;

const PING_INTERVAL_SECS: u64 = 20;

const WS_URL: &str = "wss://stream.bybit.com/v5/public/linear";
const BUFFER_SIZE: usize = 50;

/// Shared candle buffers keyed by `"SYMBOL_INTERVAL"` (e.g. `"BTCUSDT_240"`).
pub type CandleMap = Arc<Mutex<HashMap<String, VecDeque<Candle>>>>;

pub struct BybitWsClient {
    symbols: Vec<String>,
    intervals: Vec<String>,
    pub candle_map: CandleMap,
}

impl BybitWsClient {
    /// Pass all symbols and all timeframe intervals to subscribe to.
    /// Buffer keys are `"SYMBOL_INTERVAL"` (e.g. `"BTCUSDT_240"`).
    pub fn new(symbols: &[&str], intervals: &[&str]) -> Self {
        let mut map = HashMap::new();
        for &s in symbols {
            for &tf in intervals {
                map.insert(format!("{}_{}", s, tf), VecDeque::with_capacity(BUFFER_SIZE));
            }
        }
        BybitWsClient {
            symbols: symbols.iter().map(|s| s.to_string()).collect(),
            intervals: intervals.iter().map(|i| i.to_string()).collect(),
            candle_map: Arc::new(Mutex::new(map)),
        }
    }

    pub async fn connect(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let (ws_stream, _) = connect_async(WS_URL).await?;
        log::info!("WebSocket connected to Bybit ({})", WS_URL);

        let (mut write, mut read) = ws_stream.split();

        // Subscribe to all symbol × interval combinations.
        // Bybit limits 10 args per subscribe message — send in chunks.
        let args: Vec<String> = self
            .symbols
            .iter()
            .flat_map(|s| self.intervals.iter().map(move |tf| format!("kline.{}.{}", tf, s)))
            .collect();

        log::info!("Subscribing to {} topics across {} symbols…", args.len(), self.symbols.len());
        for chunk in args.chunks(10) {
            let sub_msg = json!({ "op": "subscribe", "args": chunk });
            write.send(Message::Text(sub_msg.to_string())).await?;
        }
        log::info!("Subscriptions sent ({} topics)", args.len());

        let candle_map = Arc::clone(&self.candle_map);
        let mut ping_timer = interval(Duration::from_secs(PING_INTERVAL_SECS));
        ping_timer.tick().await; // consume the immediate first tick

        let mut drop_reason: Option<String> = None;

        loop {
            tokio::select! {
                _ = ping_timer.tick() => {
                    let ping = json!({"op": "ping"}).to_string();
                    if let Err(e) = write.send(Message::Text(ping)).await {
                        log::error!("WebSocket ping error: {}", e);
                        drop_reason = Some(format!("ping failed: {e}"));
                        break;
                    }
                    log::debug!("WebSocket ping sent");
                }
                msg = read.next() => {
                    match msg {
                        Some(Ok(Message::Text(text))) => {
                            if let Ok(data) = serde_json::from_str::<serde_json::Value>(&text) {
                                // Ignore pong / op responses
                                if data["op"].as_str() == Some("pong") {
                                    log::debug!("WebSocket pong received");
                                    continue;
                                }
                                // Bybit topic format: "kline.240.BTCUSDT"
                                // Buffer key = "BTCUSDT_240"
                                if let Some(topic) = data["topic"].as_str() {
                                    let parts: Vec<&str> = topic.splitn(3, '.').collect();
                                    if parts.len() == 3 {
                                        let interval = parts[1]; // e.g. "240"
                                        let symbol   = parts[2]; // e.g. "BTCUSDT"
                                        let key = format!("{}_{}", symbol, interval);

                                        if let Some(kline_arr) = data["data"].as_array() {
                                            let mut map = candle_map.lock().unwrap();
                                            if let Some(buf) = map.get_mut(&key) {
                                                for k in kline_arr {
                                                    if let Ok(candle) = Self::parse_candle(k) {
                                                        if candle.timestamp == 0 { continue; }
                                                        // Deduplicate: replace last candle if same timestamp (live update)
                                                        if buf.back().map(|c| c.timestamp) == Some(candle.timestamp) {
                                                            *buf.back_mut().unwrap() = candle;
                                                        } else {
                                                            buf.push_back(candle);
                                                            if buf.len() > BUFFER_SIZE {
                                                                buf.pop_front();
                                                            }
                                                            log::debug!("[{} {}] candles in buffer: {}", symbol, interval, buf.len());
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        Some(Ok(Message::Close(_))) => {
                            log::warn!("WebSocket closed by server");
                            drop_reason = Some("closed by server".into());
                            break;
                        }
                        Some(Err(e)) => {
                            log::error!("WebSocket error: {}", e);
                            drop_reason = Some(format!("{e}"));
                            break;
                        }
                        None => {
                            log::warn!("WebSocket stream ended");
                            drop_reason = Some("stream ended".into());
                            break;
                        }
                        _ => {}
                    }
                }
            }
        }

        // Always return Err so reconnect_with_backoff actually reconnects
        Err(drop_reason.unwrap_or_else(|| "connection dropped".into()).into())
    }

    fn parse_candle(
        data: &serde_json::Value,
    ) -> Result<Candle, Box<dyn std::error::Error + Send + Sync>> {
        // Bybit V5 kline format uses named fields: start, open, high, low, close, volume
        Ok(Candle {
            timestamp: data["start"].as_i64().unwrap_or(0),
            open:   data["open"].as_str().unwrap_or("0").parse().unwrap_or(0.0),
            high:   data["high"].as_str().unwrap_or("0").parse().unwrap_or(0.0),
            low:    data["low"].as_str().unwrap_or("0").parse().unwrap_or(0.0),
            close:  data["close"].as_str().unwrap_or("0").parse().unwrap_or(0.0),
            volume: data["volume"].as_str().unwrap_or("0").parse().unwrap_or(0.0),
        })
    }

    /// Snapshot of candles for a specific symbol + interval.
    /// Key format: `"SYMBOL_INTERVAL"` (e.g. `"BTCUSDT_240"`).
    pub fn get_candles(&self, symbol: &str, interval: &str) -> Vec<Candle> {
        let key = format!("{}_{}", symbol, interval);
        self.candle_map
            .lock()
            .unwrap()
            .get(&key)
            .map(|buf| buf.iter().cloned().collect())
            .unwrap_or_default()
    }
}

pub async fn reconnect_with_backoff(
    client: &BybitWsClient,
    max_retries: u32,
    initial_delay_secs: u64,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut retries = 0;
    let mut delay = initial_delay_secs;

    loop {
        match client.connect().await {
            Ok(_) => return Ok(()),
            Err(e) => {
                retries += 1;
                if retries >= max_retries {
                    return Err(
                        format!("WS failed after {} retries: {}", retries, e).into()
                    );
                }
                log::warn!(
                    "WS error: {}. Reconnect in {}s… ({}/{})",
                    e,
                    delay,
                    retries,
                    max_retries
                );
                tokio::time::sleep(tokio::time::Duration::from_secs(delay)).await;
                delay = (delay * 2).min(300);
            }
        }
    }
}
