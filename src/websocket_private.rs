/// Bybit V5 private WebSocket client.
///
/// Streams: `order`, `execution`, `position`
/// Only available on **live** Bybit (NOT demo).
/// Enable with: `cargo build --release --features private-ws,jemalloc`
///
/// Provides real fill prices (actual_entry / actual_exit) which are more
/// accurate than the candle-close fallback used in demo mode.

use futures_util::{SinkExt, StreamExt};
use hmac::{Hmac, Mac};
use serde_json::json;
use sha2::Sha256;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::Message};


type HmacSha256 = Hmac<Sha256>;

const PRIVATE_WS_URL: &str = "wss://stream.bybit.com/v5/private";
const PING_INTERVAL_SECS: u64 = 20;

#[derive(Debug, Clone)]
pub struct PositionState {
    pub symbol: String,
    pub side: String,
    pub size: f64,
    pub entry_price: f64,
    pub unrealized_pnl: f64,
    pub last_update: i64,
}

#[derive(Debug, Clone)]
pub struct Execution {
    pub order_id: String,
    pub symbol: String,
    pub exec_price: f64,
    pub exec_qty: f64,
    pub exec_time: i64,
    pub exec_fee: f64,
}

pub struct BybitPrivateWs {
    api_key: String,
    api_secret: String,
    pub position_state: Arc<Mutex<HashMap<String, PositionState>>>,
    execution_tx: mpsc::Sender<Execution>,
}

impl BybitPrivateWs {
    pub fn new() -> (Self, mpsc::Receiver<Execution>) {
        let (tx, rx) = mpsc::channel(64);
        let api_key = std::env::var("BYBIT_API_KEY").expect("BYBIT_API_KEY env var not set");
        let api_secret = std::env::var("BYBIT_SECRET").expect("BYBIT_SECRET env var not set");
        let ws = BybitPrivateWs {
            api_key,
            api_secret,
            position_state: Arc::new(Mutex::new(HashMap::new())),
            execution_tx: tx,
        };
        (ws, rx)
    }

    fn sign_auth(&self) -> (String, String, String) {
        let expires = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64
            + 5000; // expires in 5 seconds

        let payload = format!("GET/realtime{}", expires);
        let mut mac =
            HmacSha256::new_from_slice(self.api_secret.as_bytes()).expect("HMAC init");
        mac.update(payload.as_bytes());
        let signature = hex::encode(mac.finalize().into_bytes());

        (self.api_key.clone(), expires.to_string(), signature)
    }

    pub async fn connect(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let (ws_stream, _) = connect_async(PRIVATE_WS_URL).await?;
        log::info!("Private WebSocket connected to Bybit");

        let (mut write, mut read) = ws_stream.split();

        // Authenticate
        let (api_key, expires, signature) = self.sign_auth();
        let auth_msg = json!({
            "op": "auth",
            "args": [api_key, expires, signature]
        });
        write.send(Message::Text(auth_msg.to_string())).await?;

        // Subscribe to execution, order, and position streams
        let sub_msg = json!({
            "op": "subscribe",
            "args": ["execution", "order", "position"]
        });

        let position_state = Arc::clone(&self.position_state);
        let execution_tx = self.execution_tx.clone();
        let mut ping_timer =
            tokio::time::interval(Duration::from_secs(PING_INTERVAL_SECS));
        ping_timer.tick().await; // consume immediate first tick

        let mut authed = false;
        let mut drop_reason: Option<String> = None;

        loop {
            tokio::select! {
                _ = ping_timer.tick() => {
                    let ping = json!({"op": "ping"}).to_string();
                    if let Err(e) = write.send(Message::Text(ping)).await {
                        drop_reason = Some(format!("ping failed: {e}"));
                        break;
                    }
                }
                msg = read.next() => {
                    match msg {
                        Some(Ok(Message::Text(text))) => {
                            if let Ok(data) = serde_json::from_str::<serde_json::Value>(&text) {
                                // Handle auth response
                                if data["op"].as_str() == Some("auth") {
                                    if data["success"].as_bool() == Some(true) {
                                        log::info!("Private WS authenticated");
                                        authed = true;
                                        write.send(Message::Text(sub_msg.to_string())).await?;
                                    } else {
                                        drop_reason = Some("auth failed".into());
                                        break;
                                    }
                                    continue;
                                }

                                if !authed { continue; }

                                let topic = data["topic"].as_str().unwrap_or("");

                                if topic == "execution" {
                                    if let Some(list) = data["data"].as_array() {
                                        for item in list {
                                            let exec = Execution {
                                                order_id: item["orderId"].as_str().unwrap_or("").to_string(),
                                                symbol: item["symbol"].as_str().unwrap_or("").to_string(),
                                                exec_price: item["execPrice"].as_str()
                                                    .unwrap_or("0").parse().unwrap_or(0.0),
                                                exec_qty: item["execQty"].as_str()
                                                    .unwrap_or("0").parse().unwrap_or(0.0),
                                                exec_time: item["execTime"].as_i64().unwrap_or(0),
                                                exec_fee: item["execFee"].as_str()
                                                    .unwrap_or("0").parse().unwrap_or(0.0),
                                            };
                                            log::info!(
                                                "[{}] Execution: orderId={} price={:.2} qty={:.4} fee={:.4}",
                                                exec.symbol, exec.order_id, exec.exec_price,
                                                exec.exec_qty, exec.exec_fee
                                            );
                                            let _ = execution_tx.try_send(exec);
                                        }
                                    }
                                } else if topic == "position" {
                                    if let Some(list) = data["data"].as_array() {
                                        let mut state = position_state.lock().unwrap();
                                        for item in list {
                                            let symbol = item["symbol"].as_str().unwrap_or("").to_string();
                                            let size: f64 = item["size"].as_str()
                                                .unwrap_or("0").parse().unwrap_or(0.0);
                                            if size == 0.0 {
                                                state.remove(&symbol);
                                            } else {
                                                state.insert(symbol.clone(), PositionState {
                                                    symbol: symbol.clone(),
                                                    side: item["side"].as_str().unwrap_or("").to_string(),
                                                    size,
                                                    entry_price: item["entryPrice"].as_str()
                                                        .unwrap_or("0").parse().unwrap_or(0.0),
                                                    unrealized_pnl: item["unrealisedPnl"].as_str()
                                                        .unwrap_or("0").parse().unwrap_or(0.0),
                                                    last_update: chrono::Utc::now().timestamp(),
                                                });
                                                log::debug!("[{}] Position update: size={:.4}", symbol, size);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        Some(Ok(Message::Close(_))) => {
                            drop_reason = Some("closed by server".into());
                            break;
                        }
                        Some(Err(e)) => {
                            drop_reason = Some(format!("{e}"));
                            break;
                        }
                        None => {
                            drop_reason = Some("stream ended".into());
                            break;
                        }
                        _ => {}
                    }
                }
            }
        }

        // Always return Err so reconnect logic can restart
        Err(drop_reason.unwrap_or_else(|| "connection dropped".into()).into())
    }
}
