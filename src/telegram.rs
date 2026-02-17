const BASE_URL: &str = "https://api.telegram.org";

#[derive(Clone)]
pub struct TelegramBot {
    client: reqwest::Client,
    url: String,
    chat_id: String,
}

impl TelegramBot {
    pub fn new() -> Self {
        let token = std::env::var("TELEGRAM_TOKEN").expect("TELEGRAM_TOKEN env var not set");
        let chat_id = std::env::var("TELEGRAM_CHAT_ID").expect("TELEGRAM_CHAT_ID env var not set");
        TelegramBot {
            client: reqwest::Client::new(),
            url: format!("{}/bot{}/sendMessage", BASE_URL, token),
            chat_id,
        }
    }

    pub async fn send(&self, text: &str) {
        let body = serde_json::json!({
            "chat_id": self.chat_id,
            "text": text,
            "parse_mode": "HTML"
        });

        match self.client.post(&self.url).json(&body).send().await {
            Ok(resp) if resp.status().is_success() => {
                let preview: String = text.chars().take(80).collect();
                log::info!("Telegram sent: {}", preview.replace('\n', " "));
            }
            Ok(resp) => {
                log::warn!("Telegram error status: {}", resp.status());
            }
            Err(e) => {
                log::warn!("Telegram send failed: {}", e);
            }
        }
    }

    // ── Convenience helpers ──────────────────────────────────────────────────

    pub async fn notify_start(&self) {
        self.send(
            "🤖 <b>FVG Trader started</b>\nPair: BTCUSDT | TF: 4H | Capital: $10,000",
        )
        .await;
    }

    pub async fn notify_trade_open(
        &self,
        symbol: &str,
        side: &str,
        qty: f64,
        entry: f64,
        sl: f64,
        tp1: f64,
        tp2: f64,
    ) {
        let emoji = if side == "Buy" { "🟢" } else { "🔴" };
        let msg = format!(
            "{emoji} <b>Trade Opened — {side} {symbol}</b>\n\
             Qty:    <code>{qty:.4}</code>\n\
             Entry:  <code>{entry:.2}</code>\n\
             SL:     <code>{sl:.2}</code>\n\
             TP1:    <code>{tp1:.2}</code>\n\
             TP2:    <code>{tp2:.2}</code>",
        );
        self.send(&msg).await;
    }

    pub async fn notify_trade_close(
        &self,
        symbol: &str,
        side: &str,
        entry: f64,
        exit: f64,
        pnl: f64,
        reason: &str,
    ) {
        let emoji = if pnl >= 0.0 { "✅" } else { "❌" };
        let msg = format!(
            "{emoji} <b>Trade Closed — {side} {symbol}</b>\n\
             Entry: <code>{entry:.2}</code>  Exit: <code>{exit:.2}</code>\n\
             PnL:   <code>{pnl:+.2} USDT</code>\n\
             Reason: {reason}",
        );
        self.send(&msg).await;
    }

    pub async fn notify_daily_summary(
        &self,
        daily_pnl: f64,
        trades: u32,
        wins: u32,
        equity: f64,
    ) {
        let win_rate = if trades > 0 {
            wins as f64 / trades as f64 * 100.0
        } else {
            0.0
        };
        let msg = format!(
            "📊 <b>Daily Summary</b>\n\
             PnL:      <code>{daily_pnl:+.2} USDT</code>\n\
             Trades:   <code>{trades}</code>  Wins: <code>{wins}</code>  WR: <code>{win_rate:.1}%</code>\n\
             Equity:   <code>{equity:.2} USDT</code>",
        );
        self.send(&msg).await;
    }

    pub async fn notify_risk_alert(&self, message: &str) {
        let msg = format!("⚠️ <b>Risk Alert</b>\n{message}");
        self.send(&msg).await;
    }

    pub async fn notify_status(
        &self,
        lines: &[String],
        equity: f64,
        daily_pnl: f64,
        trades_today: u32,
        trading_enabled: bool,
    ) {
        let status_flag = if trading_enabled { "✅ activo" } else { "⛔ detenido" };
        let pnl_emoji = if daily_pnl >= 0.0 { "📈" } else { "📉" };
        let header = format!(
            "📡 <b>Estado del bot</b> | {status_flag}\n\
             Equity: <code>${equity:.2}</code> | {pnl_emoji} PnL hoy: <code>{daily_pnl:+.2}</code> | Trades: <code>{trades_today}</code>\n\
             ─────────────────────"
        );
        let body = lines.join("\n");
        self.send(&format!("{header}\n{body}")).await;
    }
}
