use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::future::Future;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Data returned by `get_position_info` for a live exchange position.
#[derive(Debug, Clone)]
pub struct ExchangePositionInfo {
    pub side:         String,
    pub size:         f64,
    pub avg_price:    f64,
    pub stop_loss:    f64,
    pub take_profit:  f64,
    pub created_time: i64, // Unix seconds
}

use crate::config::BYBIT_REST_URL;

type HmacSha256 = Hmac<Sha256>;

// ── Error types ───────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum BybitError {
    /// Rate limited (retCode=10006 or HTTP 429). retry_after in seconds.
    RateLimit { retry_after: u64 },
    /// Transient error: network, timeout, HTTP 5xx, server overload (retCode=10016).
    Transient(String),
    /// Permanent error: invalid params, insufficient balance, HTTP 4xx.
    Permanent(String),
}

impl std::fmt::Display for BybitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BybitError::RateLimit { retry_after } => {
                write!(f, "rate limited (retry after {}s)", retry_after)
            }
            BybitError::Transient(msg) => write!(f, "transient error: {}", msg),
            BybitError::Permanent(msg) => write!(f, "permanent error: {}", msg),
        }
    }
}

impl std::error::Error for BybitError {}

/// Classify a Bybit retCode + HTTP status into a BybitError.
fn classify_error(ret_code: i64, http_status: u16, msg: &str) -> BybitError {
    match (ret_code, http_status) {
        (10006, _) | (_, 429) => BybitError::RateLimit { retry_after: 10 },
        (10016, _) | (_, 500..=599) => BybitError::Transient(msg.to_string()),
        _ => BybitError::Permanent(format!("retCode={} msg={}", ret_code, msg)),
    }
}

/// Generic retry wrapper with exponential backoff.
async fn with_retry<F, Fut, T>(operation: F, max_retries: u32) -> Result<T, BybitError>
where
    F: Fn() -> Fut,
    Fut: Future<Output = Result<T, BybitError>>,
{
    let mut retries = 0;
    let mut delay: u64 = 1;
    loop {
        match operation().await {
            Ok(r) => return Ok(r),
            Err(BybitError::RateLimit { retry_after }) => {
                if retries >= max_retries {
                    return Err(BybitError::RateLimit { retry_after });
                }
                log::warn!("Rate limited — sleeping {}s (attempt {}/{})", retry_after, retries + 1, max_retries);
                tokio::time::sleep(Duration::from_secs(retry_after)).await;
                retries += 1;
            }
            Err(BybitError::Transient(msg)) => {
                if retries >= max_retries {
                    return Err(BybitError::Transient(msg));
                }
                log::warn!("Transient error: {} — retry in {}s ({}/{})", msg, delay, retries + 1, max_retries);
                tokio::time::sleep(Duration::from_secs(delay)).await;
                delay = (delay * 2).min(60);
                retries += 1;
            }
            Err(e @ BybitError::Permanent(_)) => return Err(e),
        }
    }
}

// ── Client ────────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct BybitClient {
    client: reqwest::Client,
    base_url: String,
    api_key: String,
    api_secret: String,
}

impl BybitClient {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .pool_max_idle_per_host(10)
            .timeout(Duration::from_secs(10))
            .connect_timeout(Duration::from_secs(5))
            .build()
            .expect("HTTP client build failed");

        let api_key = std::env::var("BYBIT_API_KEY").expect("BYBIT_API_KEY env var not set");
        let api_secret = std::env::var("BYBIT_SECRET").expect("BYBIT_SECRET env var not set");

        BybitClient { client, base_url: BYBIT_REST_URL.to_string(), api_key, api_secret }
    }

    fn timestamp_ms() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64
    }

    fn sign(&self, payload: &str) -> String {
        let mut mac =
            HmacSha256::new_from_slice(self.api_secret.as_bytes()).expect("HMAC init failed");
        mac.update(payload.as_bytes());
        hex::encode(mac.finalize().into_bytes())
    }

    fn signed_headers(&self, body: &str) -> reqwest::header::HeaderMap {
        let ts = Self::timestamp_ms().to_string();
        let recv_window = "5000";
        let payload = format!("{}{}{}{}", ts, self.api_key, recv_window, body);
        let signature = self.sign(&payload);

        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("X-BAPI-API-KEY", self.api_key.parse().unwrap());
        headers.insert("X-BAPI-TIMESTAMP", ts.parse().unwrap());
        headers.insert("X-BAPI-SIGN", signature.parse().unwrap());
        headers.insert("X-BAPI-RECV-WINDOW", recv_window.parse().unwrap());
        headers.insert(
            reqwest::header::CONTENT_TYPE,
            "application/json".parse().unwrap(),
        );
        headers
    }

    // ── Internal raw methods (no retry) ──────────────────────────────────────

    async fn place_order_raw(
        &self,
        symbol: &str,
        side: &str,
        qty: f64,
        stop_loss: f64,
        take_profit: f64,
    ) -> Result<String, BybitError> {
        let body = serde_json::json!({
            "category":   "linear",
            "symbol":     symbol,
            "side":       side,
            "orderType":  "Market",
            "qty":        format!("{:.4}", qty),
            "stopLoss":   format!("{:.2}", stop_loss),
            "takeProfit": format!("{:.2}", take_profit),
            "tpslMode":   "Full",
            "timeInForce":"GTC"
        })
        .to_string();

        let url = format!("{}/v5/order/create", self.base_url);
        let headers = self.signed_headers(&body);

        let resp = self
            .client
            .post(&url)
            .headers(headers)
            .body(body)
            .send()
            .await
            .map_err(|e| BybitError::Transient(format!("HTTP error: {}", e)))?;

        let http_status = resp.status().as_u16();
        let json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| BybitError::Transient(format!("Parse error: {}", e)))?;

        let ret_code = json["retCode"].as_i64().unwrap_or(-1);
        if ret_code == 0 {
            let order_id = json["result"]["orderId"]
                .as_str()
                .unwrap_or("unknown")
                .to_string();
            log::info!("Order placed: {} {} {} qty={:.4}", side, symbol, order_id, qty);
            Ok(order_id)
        } else {
            let msg = json["retMsg"].as_str().unwrap_or("unknown");
            Err(classify_error(ret_code, http_status, msg))
        }
    }

    async fn close_position_raw(
        &self,
        symbol: &str,
        side: &str,
        qty: f64,
    ) -> Result<String, BybitError> {
        let close_side = if side == "Buy" { "Sell" } else { "Buy" };

        let body = serde_json::json!({
            "category":     "linear",
            "symbol":       symbol,
            "side":         close_side,
            "orderType":    "Market",
            "qty":          format!("{:.4}", qty),
            "reduceOnly":   true,
            "timeInForce":  "GTC"
        })
        .to_string();

        let url = format!("{}/v5/order/create", self.base_url);
        let headers = self.signed_headers(&body);

        let resp = self
            .client
            .post(&url)
            .headers(headers)
            .body(body)
            .send()
            .await
            .map_err(|e| BybitError::Transient(format!("HTTP error: {}", e)))?;

        let http_status = resp.status().as_u16();
        let json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| BybitError::Transient(format!("Parse error: {}", e)))?;

        let ret_code = json["retCode"].as_i64().unwrap_or(-1);
        if ret_code == 0 {
            let order_id = json["result"]["orderId"]
                .as_str()
                .unwrap_or("unknown")
                .to_string();
            log::info!("Position closed: {} {} orderId={}", symbol, close_side, order_id);
            Ok(order_id)
        } else {
            let msg = json["retMsg"].as_str().unwrap_or("unknown");
            Err(classify_error(ret_code, http_status, msg))
        }
    }

    async fn get_position_raw(
        &self,
        symbol: &str,
    ) -> Result<serde_json::Value, BybitError> {
        let ts = Self::timestamp_ms().to_string();
        let recv_window = "5000";
        let query = format!("category=linear&symbol={}", symbol);
        let payload = format!("{}{}{}{}", ts, self.api_key, recv_window, query);
        let signature = self.sign(&payload);

        let url = format!("{}/v5/position/list?{}", self.base_url, query);
        let resp = self
            .client
            .get(&url)
            .header("X-BAPI-API-KEY", &self.api_key)
            .header("X-BAPI-TIMESTAMP", &ts)
            .header("X-BAPI-SIGN", signature)
            .header("X-BAPI-RECV-WINDOW", recv_window)
            .send()
            .await
            .map_err(|e| BybitError::Transient(format!("HTTP error: {}", e)))?;

        let http_status = resp.status().as_u16();
        let json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| BybitError::Transient(format!("Parse error: {}", e)))?;

        let ret_code = json["retCode"].as_i64().unwrap_or(-1);
        if ret_code == 0 {
            Ok(json)
        } else {
            let msg = json["retMsg"].as_str().unwrap_or("unknown");
            Err(classify_error(ret_code, http_status, msg))
        }
    }

    async fn fetch_klines_raw(
        &self,
        symbol: &str,
        interval: &str,
        limit: usize,
    ) -> Result<Vec<crate::types::Candle>, BybitError> {
        let url = format!(
            "https://api.bybit.com/v5/market/kline?category=linear&symbol={}&interval={}&limit={}",
            symbol, interval, limit
        );
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| BybitError::Transient(format!("HTTP error: {}", e)))?;

        let http_status = resp.status().as_u16();
        let json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| BybitError::Transient(format!("Parse error: {}", e)))?;

        let ret_code = json["retCode"].as_i64().unwrap_or(-1);
        if ret_code != 0 {
            let msg = json["retMsg"].as_str().unwrap_or("unknown");
            return Err(classify_error(ret_code, http_status, msg));
        }

        let list = json["result"]["list"]
            .as_array()
            .ok_or_else(|| BybitError::Transient("missing result.list".into()))?;

        let mut candles: Vec<crate::types::Candle> = list
            .iter()
            .filter_map(|row| {
                let arr = row.as_array()?;
                let ts: i64 = arr[0].as_str()?.parse().ok()?;
                let open: f64 = arr[1].as_str()?.parse().ok()?;
                let high: f64 = arr[2].as_str()?.parse().ok()?;
                let low: f64 = arr[3].as_str()?.parse().ok()?;
                let close: f64 = arr[4].as_str()?.parse().ok()?;
                let volume: f64 = arr[5].as_str()?.parse().ok()?;
                Some(crate::types::Candle { timestamp: ts, open, high, low, close, volume })
            })
            .collect();
        candles.reverse(); // Bybit returns newest-first; reverse to oldest-first
        Ok(candles)
    }

    // ── Public methods with retry ─────────────────────────────────────────────

    /// Place a market order.  side = "Buy" | "Sell"
    pub async fn place_order(
        &self,
        symbol: &str,
        side: &str,
        qty: f64,
        stop_loss: f64,
        take_profit: f64,
    ) -> Result<String, BybitError> {
        let s = self.clone();
        let sym = symbol.to_string();
        let si = side.to_string();
        with_retry(|| {
            let s = s.clone();
            let sym = sym.clone();
            let si = si.clone();
            async move { s.place_order_raw(&sym, &si, qty, stop_loss, take_profit).await }
        }, 3).await
    }

    /// Close an open position with a market order (opposite side).
    pub async fn close_position(
        &self,
        symbol: &str,
        side: &str,
        qty: f64,
    ) -> Result<String, BybitError> {
        let s = self.clone();
        let sym = symbol.to_string();
        let si = side.to_string();
        with_retry(|| {
            let s = s.clone();
            let sym = sym.clone();
            let si = si.clone();
            async move { s.close_position_raw(&sym, &si, qty).await }
        }, 3).await
    }

    /// Fetch current position for a symbol.
    pub async fn get_position(&self, symbol: &str) -> Result<serde_json::Value, BybitError> {
        let s = self.clone();
        let sym = symbol.to_string();
        with_retry(|| {
            let s = s.clone();
            let sym = sym.clone();
            async move { s.get_position_raw(&sym).await }
        }, 5).await
    }

    /// Parse position data from exchange. Returns None if no open position (size == 0).
    pub async fn get_position_info(
        &self,
        symbol: &str,
    ) -> Result<Option<ExchangePositionInfo>, BybitError> {
        let json = self.get_position(symbol).await?;
        let entry = json["result"]["list"]
            .as_array()
            .and_then(|a| a.first())
            .cloned()
            .unwrap_or(serde_json::Value::Null);

        let size: f64 = entry["size"].as_str().and_then(|s| s.parse().ok()).unwrap_or(0.0);
        if size == 0.0 {
            return Ok(None);
        }

        Ok(Some(ExchangePositionInfo {
            side:         entry["side"].as_str().unwrap_or("Buy").to_string(),
            size,
            avg_price:    entry["avgPrice"].as_str().and_then(|s| s.parse().ok()).unwrap_or(0.0),
            stop_loss:    entry["stopLoss"].as_str().and_then(|s| s.parse().ok()).unwrap_or(0.0),
            take_profit:  entry["takeProfit"].as_str().and_then(|s| s.parse().ok()).unwrap_or(0.0),
            created_time: entry["createdTime"].as_str()
                .and_then(|s| s.parse::<i64>().ok())
                .map(|ms| ms / 1000)
                .unwrap_or_else(|| chrono::Utc::now().timestamp()),
        }))
    }

    /// Fetch ALL open linear positions in a single authenticated REST call.
    /// Returns a map of symbol → ExchangePositionInfo (only symbols with size > 0).
    pub async fn get_all_open_positions(
        &self,
    ) -> Result<std::collections::HashMap<String, ExchangePositionInfo>, BybitError> {
        let ts = Self::timestamp_ms().to_string();
        let recv_window = "5000";
        let query = "category=linear&settleCoin=USDT&limit=200";
        let payload = format!("{}{}{}{}", ts, self.api_key, recv_window, query);
        let signature = self.sign(&payload);

        let url = format!("{}/v5/position/list?{}", self.base_url, query);
        let resp = self
            .client
            .get(&url)
            .header("X-BAPI-API-KEY", &self.api_key)
            .header("X-BAPI-TIMESTAMP", &ts)
            .header("X-BAPI-SIGN", signature)
            .header("X-BAPI-RECV-WINDOW", recv_window)
            .send()
            .await
            .map_err(|e| BybitError::Transient(format!("HTTP error: {}", e)))?;

        let http_status = resp.status().as_u16();
        let json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| BybitError::Transient(format!("Parse error: {}", e)))?;

        let ret_code = json["retCode"].as_i64().unwrap_or(-1);
        if ret_code != 0 {
            let msg = json["retMsg"].as_str().unwrap_or("unknown");
            return Err(classify_error(ret_code, http_status, msg));
        }

        let list = match json["result"]["list"].as_array() {
            Some(l) => l,
            None => return Ok(std::collections::HashMap::new()),
        };

        let mut map = std::collections::HashMap::new();
        for entry in list {
            let size: f64 = entry["size"].as_str().and_then(|s| s.parse().ok()).unwrap_or(0.0);
            if size == 0.0 { continue; }
            let symbol = match entry["symbol"].as_str() {
                Some(s) => s.to_string(),
                None => continue,
            };
            map.insert(symbol, ExchangePositionInfo {
                side:         entry["side"].as_str().unwrap_or("Buy").to_string(),
                size,
                avg_price:    entry["avgPrice"].as_str().and_then(|s| s.parse().ok()).unwrap_or(0.0),
                stop_loss:    entry["stopLoss"].as_str().and_then(|s| s.parse().ok()).unwrap_or(0.0),
                take_profit:  entry["takeProfit"].as_str().and_then(|s| s.parse().ok()).unwrap_or(0.0),
                created_time: entry["createdTime"].as_str()
                    .and_then(|s| s.parse::<i64>().ok())
                    .map(|ms| ms / 1000)
                    .unwrap_or_else(|| chrono::Utc::now().timestamp()),
            });
        }
        Ok(map)
    }

    /// Count open positions on exchange (single REST call).
    pub async fn count_open_exchange_positions(&self, _symbols: &[&str]) -> usize {
        match self.get_all_open_positions().await {
            Ok(map) => map.len(),
            Err(e) => {
                log::warn!("count_open_exchange_positions failed: {}", e);
                0
            }
        }
    }

    /// Fetch the last `limit` closed klines for a symbol (public endpoint, no auth).
    /// Returns candles oldest-first.
    pub async fn fetch_klines(
        &self,
        symbol: &str,
        interval: &str,
        limit: usize,
    ) -> Result<Vec<crate::types::Candle>, BybitError> {
        let s = self.clone();
        let sym = symbol.to_string();
        let iv = interval.to_string();
        with_retry(|| {
            let s = s.clone();
            let sym = sym.clone();
            let iv = iv.clone();
            async move { s.fetch_klines_raw(&sym, &iv, limit).await }
        }, 3).await
    }

    /// Fetch all active USDT linear perpetual symbols from Bybit (public, no auth).
    /// Returns symbols sorted alphabetically.
    pub async fn fetch_linear_symbols(&self) -> Result<Vec<String>, BybitError> {
        let url = "https://api.bybit.com/v5/market/instruments-info\
                   ?category=linear&status=Trading&limit=1000";
        let resp = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|e| BybitError::Transient(format!("HTTP error: {}", e)))?;

        let http_status = resp.status().as_u16();
        let json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| BybitError::Transient(format!("Parse error: {}", e)))?;

        let ret_code = json["retCode"].as_i64().unwrap_or(-1);
        if ret_code != 0 {
            let msg = json["retMsg"].as_str().unwrap_or("unknown");
            return Err(classify_error(ret_code, http_status, msg));
        }

        let list = json["result"]["list"]
            .as_array()
            .ok_or_else(|| BybitError::Permanent("instruments-info: missing list".into()))?;

        let mut symbols: Vec<String> = list
            .iter()
            .filter_map(|item| {
                let symbol = item["symbol"].as_str()?;
                let quote  = item["quoteCoin"].as_str()?;
                if quote == "USDT" { Some(symbol.to_string()) } else { None }
            })
            .collect();
        symbols.sort();
        Ok(symbols)
    }

    /// Place a limit order (better fill, maker fees). side = "Buy" | "Sell"
    pub async fn place_limit_order(
        &self,
        symbol: &str,
        side: &str,
        qty: f64,
        price: f64,
        stop_loss: f64,
        take_profit: f64,
    ) -> Result<String, BybitError> {
        let body = serde_json::json!({
            "category":   "linear",
            "symbol":     symbol,
            "side":       side,
            "orderType":  "Limit",
            "qty":        format!("{:.4}", qty),
            "price":      format!("{:.2}", price),
            "stopLoss":   format!("{:.2}", stop_loss),
            "takeProfit": format!("{:.2}", take_profit),
            "tpslMode":   "Full",
            "timeInForce":"GTC"
        })
        .to_string();

        let url = format!("{}/v5/order/create", self.base_url);
        let headers = self.signed_headers(&body);

        let resp = self
            .client
            .post(&url)
            .headers(headers)
            .body(body)
            .send()
            .await
            .map_err(|e| BybitError::Transient(format!("HTTP error: {}", e)))?;

        let http_status = resp.status().as_u16();
        let json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| BybitError::Transient(format!("Parse error: {}", e)))?;

        let ret_code = json["retCode"].as_i64().unwrap_or(-1);
        if ret_code == 0 {
            let order_id = json["result"]["orderId"]
                .as_str()
                .unwrap_or("unknown")
                .to_string();
            log::info!("Limit order placed: {} {} {} qty={:.4} price={:.2}", side, symbol, order_id, qty, price);
            Ok(order_id)
        } else {
            let msg = json["retMsg"].as_str().unwrap_or("unknown");
            Err(classify_error(ret_code, http_status, msg))
        }
    }
}
