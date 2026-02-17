use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use tokio::time::{interval, Duration};
use tokio_tungstenite::{connect_async, tungstenite::Message};

use crate::config::KLINE_INTERVAL;
use crate::types::Candle;

const PING_INTERVAL_SECS: u64 = 20;

const WS_URL: &str = "wss://stream.bybit.com/v5/public/linear";
const BUFFER_SIZE: usize = 50;

/// Shared candle buffers keyed by symbol (e.g. "BTCUSDT").
pub type CandleMap = Arc<Mutex<HashMap<String, VecDeque<Candle>>>>;

pub struct BybitWsClient {
    symbols: Vec<String>,
    pub candle_map: CandleMap,
}

impl BybitWsClient {
    /// Pass all symbols to subscribe to (e.g. &["BTCUSDT", "ETHUSDT", …]).
    pub fn new(symbols: &[&str]) -> Self {
        let mut map = HashMap::new();
        for &s in symbols {
            map.insert(s.to_string(), VecDeque::with_capacity(BUFFER_SIZE));
        }
        BybitWsClient {
            symbols: symbols.iter().map(|s| s.to_string()).collect(),
            candle_map: Arc::new(Mutex::new(map)),
        }
    }

    pub async fn connect(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let (ws_stream, _) = connect_async(WS_URL).await?;
        log::info!("WebSocket connected to Bybit ({})", WS_URL);

        let (mut write, mut read) = ws_stream.split();

        // Subscribe to all symbols in one message
        let args: Vec<String> = self
            .symbols
            .iter()
            .map(|s| format!("kline.{}.{}", KLINE_INTERVAL, s))
            .collect();

        let sub_msg = json!({ "op": "subscribe", "args": args });
        write
            .send(Message::Text(sub_msg.to_string()))
            .await?;
        log::info!("Subscribed to: {:?}", args);

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
                                // Bybit topic format: "kline.4.BTCUSDT"
                                if let Some(topic) = data["topic"].as_str() {
                                    let symbol = topic
                                        .splitn(3, '.')
                                        .nth(2)
                                        .unwrap_or("")
                                        .to_string();

                                    if !symbol.is_empty() {
                                        if let Some(kline_arr) = data["data"].as_array() {
                                            let mut map = candle_map.lock().unwrap();
                                            if let Some(buf) = map.get_mut(&symbol) {
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
                                                            log::debug!("[{}] candles in buffer: {}", symbol, buf.len());
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

    /// Snapshot of candles for a specific symbol.
    pub fn get_candles(&self, symbol: &str) -> Vec<Candle> {
        self.candle_map
            .lock()
            .unwrap()
            .get(symbol)
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
