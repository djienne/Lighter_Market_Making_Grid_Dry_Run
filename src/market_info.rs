use serde::Deserialize;

const BASE_URL: &str = "https://mainnet.zklighter.elliot.ai";

#[derive(Deserialize)]
struct OrderBooksResponse {
    order_books: Vec<OrderBookInfo>,
}

#[derive(Deserialize)]
struct OrderBookInfo {
    market_id: i64,
    symbol: Option<String>,
    supported_price_decimals: Option<i64>,
    supported_size_decimals: Option<i64>,
    min_base_amount: Option<String>,
    min_quote_amount: Option<String>,
}

pub struct MarketDetails {
    pub market_id: i64,
    pub price_tick: f64,
    pub amount_tick: f64,
    pub min_base_amount: f64,
    pub min_quote_amount: f64,
}

/// Fallback tick sizes for known symbols.
fn fallback_tick_size(symbol: &str) -> Option<(f64, f64)> {
    match symbol.to_uppercase().as_str() {
        "BTC" => Some((0.1, 0.0001)),
        "ETH" => Some((0.01, 0.001)),
        "SOL" => Some((0.001, 0.01)),
        _ => None,
    }
}

/// Fetch market details from Lighter REST API.
pub async fn get_market_details(symbol: &str) -> anyhow::Result<MarketDetails> {
    let url = format!("{}/api/v1/orderBooks", BASE_URL);
    let resp: OrderBooksResponse = reqwest::get(&url).await?.json().await?;

    let symbol_upper = symbol.to_uppercase();
    for ob in &resp.order_books {
        let sym = ob.symbol.as_deref().unwrap_or("");
        if sym.eq_ignore_ascii_case(&symbol_upper) {
            let price_decimals = ob.supported_price_decimals.unwrap_or(1);
            let size_decimals = ob.supported_size_decimals.unwrap_or(4);

            let price_tick = 10.0_f64.powi(-(price_decimals as i32));
            let amount_tick = 10.0_f64.powi(-(size_decimals as i32));
            let min_base = ob
                .min_base_amount
                .as_deref()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0.0);
            let min_quote = ob
                .min_quote_amount
                .as_deref()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0.0);

            return Ok(MarketDetails {
                market_id: ob.market_id,
                price_tick,
                amount_tick,
                min_base_amount: min_base,
                min_quote_amount: min_quote,
            });
        }
    }

    // Fallback
    if let Some((pt, at)) = fallback_tick_size(symbol) {
        tracing::warn!(
            "Market {} not found via API, using fallback ticks: price={}, amount={}",
            symbol, pt, at
        );
        Ok(MarketDetails {
            market_id: 0,
            price_tick: pt,
            amount_tick: at,
            min_base_amount: 0.0,
            min_quote_amount: 0.0,
        })
    } else {
        anyhow::bail!("Market {} not found and no fallback available", symbol)
    }
}
