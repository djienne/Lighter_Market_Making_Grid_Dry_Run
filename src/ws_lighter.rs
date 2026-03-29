use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tokio_tungstenite::connect_async_with_config;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;

const LIGHTER_WS_URL: &str = "wss://mainnet.zklighter.elliot.ai/stream";
const PONG_MSG: &str = r#"{"type":"pong"}"#;

#[derive(Debug)]
pub enum LighterWsMsg {
    OrderbookUpdate {
        bids: Vec<(f64, f64)>,
        asks: Vec<(f64, f64)>,
    },
    TickerUpdate {
        best_bid: f64,
        best_ask: f64,
    },
    Disconnected,
}

/// Parse price/size levels from JSON array of objects.
fn parse_levels(arr: &[serde_json::Value]) -> Vec<(f64, f64)> {
    arr.iter()
        .filter_map(|item| {
            let price: f64 = item.get("price")?.as_str()?.parse().ok()?;
            let size: f64 = item.get("size")?.as_str()?.parse().ok()?;
            Some((price, size))
        })
        .collect()
}

/// Connect to Lighter WS and subscribe to orderbook + ticker channels.
/// Sends parsed messages through the channel.
///
/// Keepalive strategy: use `tokio::select!` to race incoming messages against
/// a periodic ping timer.  Every `ping_interval_secs` we send a WebSocket
/// protocol-level Ping frame (opcode 0x9).  The Lighter server requires at
/// least one frame every 120 seconds or it closes the connection; sending a
/// Ping every 20 s (the default) mirrors the Python `websockets` library's
/// `ping_interval=20` behaviour.
///
/// We also reply to application-level `{"type":"ping"}` JSON messages with
/// `{"type":"pong"}` in case the server sends those.
pub async fn run_lighter_ws(
    market_id: i64,
    tx: mpsc::Sender<LighterWsMsg>,
    ping_interval_secs: u64,
    recv_timeout_secs: f64,
    reconnect_base: u64,
    reconnect_max: u64,
) {
    let ob_channel = format!("order_book/{}", market_id);
    let ticker_channel = format!("ticker/{}", market_id);
    let mut backoff = reconnect_base;

    loop {
        let ws_config = WebSocketConfig::default();

        match connect_async_with_config(LIGHTER_WS_URL, Some(ws_config), false).await {
            Ok((ws_stream, _)) => {
                tracing::info!("Lighter WS connected");
                let (mut write, mut read) = ws_stream.split();
                backoff = reconnect_base;

                // Subscribe
                let mut sub_ok = true;
                for ch in [&ob_channel, &ticker_channel] {
                    let sub = serde_json::json!({"type": "subscribe", "channel": ch});
                    if let Err(e) = write
                        .send(tokio_tungstenite::tungstenite::Message::Text(sub.to_string().into()))
                        .await
                    {
                        tracing::error!("Failed to subscribe to {}: {}", ch, e);
                        sub_ok = false;
                        break;
                    }
                }
                if !sub_ok {
                    let _ = tx.send(LighterWsMsg::Disconnected).await;
                    tracing::info!("Lighter WS reconnecting in {}s...", backoff);
                    tokio::time::sleep(tokio::time::Duration::from_secs(backoff)).await;
                    backoff = (backoff * 2).min(reconnect_max);
                    continue;
                }

                let recv_timeout = tokio::time::Duration::from_secs_f64(recv_timeout_secs);
                // Periodic ping timer -- fires every `ping_interval_secs`.
                // Using `tokio::time::interval` inside `tokio::select!` ensures
                // the ping is sent on schedule even while we are blocked waiting
                // for the next incoming message.
                let mut ping_ticker = tokio::time::interval(
                    tokio::time::Duration::from_secs(ping_interval_secs),
                );
                // The first tick completes immediately; consume it so we don't
                // send a spurious ping right after connecting.
                ping_ticker.tick().await;

                // Deadline for the recv-timeout watchdog.  Reset every time we
                // receive any frame from the server.
                let mut deadline = tokio::time::Instant::now() + recv_timeout;

                loop {
                    tokio::select! {
                        // --- Branch 1: incoming WebSocket message ---
                        frame = read.next() => {
                            match frame {
                                Some(Ok(tokio_tungstenite::tungstenite::Message::Text(text))) => {
                                    deadline = tokio::time::Instant::now() + recv_timeout;

                                    let data: serde_json::Value = match serde_json::from_str(&text) {
                                        Ok(d) => d,
                                        Err(_) => continue,
                                    };

                                    let msg_type = data
                                        .get("type")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("");

                                    // Application-level ping from the server
                                    if msg_type == "ping" {
                                        let _ = write
                                            .send(tokio_tungstenite::tungstenite::Message::Text(
                                                PONG_MSG.to_string().into(),
                                            ))
                                            .await;
                                        continue;
                                    }

                                    if msg_type == "subscribed" {
                                        tracing::info!(
                                            "Lighter WS subscribed to: {}",
                                            data.get("channel")
                                                .and_then(|v| v.as_str())
                                                .unwrap_or("?")
                                        );
                                        continue;
                                    }

                                    if msg_type.contains("order_book") {
                                        if let Some(ob) = data.get("order_book") {
                                            let bids = ob
                                                .get("bids")
                                                .and_then(|v| v.as_array())
                                                .map(|a| parse_levels(a))
                                                .unwrap_or_default();
                                            let asks = ob
                                                .get("asks")
                                                .and_then(|v| v.as_array())
                                                .map(|a| parse_levels(a))
                                                .unwrap_or_default();
                                            let _ = tx
                                                .send(LighterWsMsg::OrderbookUpdate { bids, asks })
                                                .await;
                                        }
                                    } else if msg_type.contains("ticker") {
                                        if let Some(ticker) = data.get("ticker") {
                                            let bb = ticker
                                                .get("best_bid_price")
                                                .and_then(|v| v.as_str())
                                                .and_then(|s| s.parse().ok())
                                                .unwrap_or(0.0);
                                            let ba = ticker
                                                .get("best_ask_price")
                                                .and_then(|v| v.as_str())
                                                .and_then(|s| s.parse().ok())
                                                .unwrap_or(0.0);
                                            if bb > 0.0 || ba > 0.0 {
                                                let _ = tx
                                                    .send(LighterWsMsg::TickerUpdate {
                                                        best_bid: bb,
                                                        best_ask: ba,
                                                    })
                                                    .await;
                                            }
                                        }
                                    }
                                }
                                // tungstenite surfaces Pong frames here when the
                                // server replies to our protocol-level Ping.
                                // Just reset the watchdog; no action needed.
                                Some(Ok(tokio_tungstenite::tungstenite::Message::Pong(_))) => {
                                    deadline = tokio::time::Instant::now() + recv_timeout;
                                }
                                Some(Ok(tokio_tungstenite::tungstenite::Message::Close(_))) => {
                                    tracing::info!("Lighter WS closed by server");
                                    break;
                                }
                                Some(Err(e)) => {
                                    tracing::warn!("Lighter WS error: {}", e);
                                    break;
                                }
                                None => {
                                    tracing::info!("Lighter WS stream ended");
                                    break;
                                }
                                // Ping frames from the server are handled
                                // automatically by tungstenite (it queues a Pong
                                // reply which is flushed on the next write/read).
                                _ => {
                                    deadline = tokio::time::Instant::now() + recv_timeout;
                                }
                            }
                        }

                        // --- Branch 2: periodic ping timer ---
                        _ = ping_ticker.tick() => {
                            if let Err(e) = write
                                .send(tokio_tungstenite::tungstenite::Message::Ping(vec![].into()))
                                .await
                            {
                                tracing::warn!("Lighter WS failed to send ping: {}", e);
                                break;
                            }
                            tracing::trace!("Lighter WS sent protocol ping");
                        }

                        // --- Branch 3: recv-timeout watchdog ---
                        _ = tokio::time::sleep_until(deadline) => {
                            tracing::warn!(
                                "Lighter WS watchdog: no data for {:.0}s",
                                recv_timeout_secs
                            );
                            break;
                        }
                    }
                }

                let _ = tx.send(LighterWsMsg::Disconnected).await;
            }
            Err(e) => {
                tracing::warn!("Lighter WS connect failed: {}", e);
            }
        }

        tracing::info!("Lighter WS reconnecting in {}s...", backoff);
        tokio::time::sleep(tokio::time::Duration::from_secs(backoff)).await;
        backoff = (backoff * 2).min(reconnect_max);
    }
}
