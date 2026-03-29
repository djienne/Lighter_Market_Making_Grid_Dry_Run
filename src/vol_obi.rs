use crate::orderbook::BookSide;
use crate::rolling_stats::RollingStats;

/// Volatility + OBI spread calculator.
///
/// Port of vol_obi.py:VolObiCalculator. All scaling (dollars, ticks,
/// vol_scale) matches the Python/Rust reference exactly.
pub struct VolObiCalculator {
    mid_stats: RollingStats,
    imb_stats: RollingStats,
    prev_mid: Option<f64>,
    volatility: f64,
    alpha: f64,
    alpha_override: Option<f64>,
    warmed_up: bool,
    total_samples: u64,
    // config
    tick_size: f64,
    vol_scale: f64,
    vol_to_half_spread: f64,
    min_half_spread_bps: f64,
    c1: f64,
    skew: f64,
    looking_depth: f64,
    min_warmup_samples: u64,
    max_position_dollar: f64,
}

impl VolObiCalculator {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        tick_size: f64,
        window_steps: usize,
        step_ns: u64,
        vol_to_half_spread: f64,
        min_half_spread_bps: f64,
        c1_ticks: f64,
        c1: f64,
        skew: f64,
        looking_depth: f64,
        min_warmup_samples: u64,
        max_position_dollar: f64,
    ) -> Self {
        assert!(tick_size > 0.0, "tick_size must be positive");
        let vol_scale = (1_000_000_000.0 / step_ns as f64).sqrt();
        let c1_val = if c1 > 0.0 { c1 } else { c1_ticks * tick_size };
        Self {
            mid_stats: RollingStats::new(window_steps),
            imb_stats: RollingStats::new(window_steps),
            prev_mid: None,
            volatility: 0.0,
            alpha: 0.0,
            alpha_override: None,
            warmed_up: false,
            total_samples: 0,
            tick_size,
            vol_scale,
            vol_to_half_spread,
            min_half_spread_bps,
            c1: c1_val,
            skew,
            looking_depth,
            min_warmup_samples,
            max_position_dollar,
        }
    }

    /// Feed a new mid-price and orderbook sides. Called from WS callback (hot path).
    pub fn on_book_update(&mut self, mid_price: f64, bids: &BookSide, asks: &BookSide) {
        // 1. Mid-price change → volatility [DOLLARS]
        if let Some(prev) = self.prev_mid {
            let change = mid_price - prev;
            self.mid_stats.push(change);
            self.total_samples += 1;
        }
        self.prev_mid = Some(mid_price);

        // 2. Order book imbalance → alpha [quantity units]
        let imbalance = self.compute_imbalance(mid_price, bids, asks);
        self.imb_stats.push(imbalance);

        // 3. Update cached volatility & alpha once warmed up
        if self.total_samples >= self.min_warmup_samples {
            if !self.warmed_up {
                self.warmed_up = true;
                tracing::debug!(
                    "Vol+OBI warmed up after {} samples | vol_scale={:.3}",
                    self.total_samples,
                    self.vol_scale,
                );
            }
            let vol_raw = self.mid_stats.std();
            self.volatility = vol_raw * self.vol_scale;
            if let Some(override_val) = self.alpha_override {
                self.alpha = override_val;
            } else {
                self.alpha = self.imb_stats.zscore(imbalance);
            }
        }
    }

    fn compute_imbalance(&self, mid_price: f64, bids: &BookSide, asks: &BookSide) -> f64 {
        let depth = self.looking_depth;
        let lower = mid_price * (1.0 - depth);
        let upper = mid_price * (1.0 + depth);

        // Sum bid sizes within looking depth (iterate from highest to lowest)
        let mut sum_bid = 0.0;
        for (&price, &size) in bids.iter().rev() {
            if price.into_inner() < lower {
                break;
            }
            sum_bid += size;
        }

        // Sum ask sizes within looking depth (iterate from lowest to highest)
        let mut sum_ask = 0.0;
        for (&price, &size) in asks.iter() {
            if price.into_inner() > upper {
                break;
            }
            sum_ask += size;
        }

        sum_bid - sum_ask
    }

    /// Calculate bid/ask prices. Returns None if not warmed up or crossed.
    pub fn quote(&self, mid_price: f64, position_size: f64) -> Option<(f64, f64)> {
        if !self.warmed_up {
            return None;
        }

        let tick = self.tick_size;

        // STEP 2: half-spread in dollars → ticks
        let half_spread_price = self.volatility * self.vol_to_half_spread;
        let half_spread_tick = half_spread_price / tick;

        // STEP 4: fair price
        let fair_price = mid_price + self.c1 * self.alpha;

        // STEP 5: position skew in ticks
        let norm_pos = if self.max_position_dollar > 0.0 {
            ((position_size * mid_price) / self.max_position_dollar).clamp(-1.0, 1.0)
        } else {
            0.0
        };

        let bid_depth_tick = (half_spread_tick * (1.0 + self.skew * norm_pos)).max(0.0);
        let ask_depth_tick = (half_spread_tick * (1.0 - self.skew * norm_pos)).max(0.0);

        // Convert depths back to dollars
        let mut raw_bid = fair_price - bid_depth_tick * tick;
        let mut raw_ask = fair_price + ask_depth_tick * tick;

        // STEP 6: min spread floor in bps
        if self.min_half_spread_bps > 0.0 {
            let min_bid = mid_price * (1.0 - self.min_half_spread_bps / 10000.0);
            if raw_bid > min_bid {
                raw_bid = min_bid;
            }
            let min_ask = mid_price * (1.0 + self.min_half_spread_bps / 10000.0);
            if raw_ask < min_ask {
                raw_ask = min_ask;
            }
        }

        // Snap to tick grid
        let bid_price = (raw_bid / tick).floor() * tick;
        let ask_price = (raw_ask / tick).ceil() * tick;

        // Guard: never return crossed quotes
        if bid_price >= ask_price {
            return None;
        }

        Some((bid_price, ask_price))
    }

    pub fn set_alpha_override(&mut self, alpha: Option<f64>) {
        self.alpha_override = alpha;
    }

    pub fn warmed_up(&self) -> bool {
        self.warmed_up
    }

    pub fn volatility(&self) -> f64 {
        self.volatility
    }

    pub fn alpha(&self) -> f64 {
        self.alpha
    }

    pub fn total_samples(&self) -> u64 {
        self.total_samples
    }

    pub fn reset(&mut self) {
        self.mid_stats.clear();
        self.imb_stats.clear();
        self.prev_mid = None;
        self.volatility = 0.0;
        self.alpha = 0.0;
        self.alpha_override = None;
        self.warmed_up = false;
        self.total_samples = 0;
    }

    pub fn set_max_position_dollar(&mut self, value: f64) {
        self.max_position_dollar = value.max(0.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ordered_float::OrderedFloat;

    fn make_calc() -> VolObiCalculator {
        VolObiCalculator::new(
            0.1,   // tick_size
            6000,  // window_steps
            100_000_000, // step_ns
            48.0,  // vol_to_half_spread
            8.0,   // min_half_spread_bps
            20.0,  // c1_ticks
            0.0,   // c1 (auto from c1_ticks)
            3.0,   // skew
            0.025, // looking_depth
            100,   // min_warmup_samples
            500.0, // max_position_dollar
        )
    }

    #[test]
    fn test_not_warmed_up() {
        let calc = make_calc();
        assert!(!calc.warmed_up());
        assert!(calc.quote(87000.0, 0.0).is_none());
    }

    #[test]
    fn test_warmup_and_quote() {
        let mut calc = make_calc();
        let mut bids = BookSide::new();
        let mut asks = BookSide::new();

        // Build a simple book
        for i in 0..50 {
            bids.insert(OrderedFloat(87000.0 - i as f64 * 0.1), 1.0);
            asks.insert(OrderedFloat(87000.1 + i as f64 * 0.1), 1.0);
        }

        // Feed enough samples to warm up
        for i in 0..110 {
            let mid = 87000.0 + (i as f64 * 0.01).sin() * 0.5;
            calc.on_book_update(mid, &bids, &asks);
        }

        assert!(calc.warmed_up());
        let result = calc.quote(87000.0, 0.0);
        assert!(result.is_some());

        let (bid, ask) = result.unwrap();
        assert!(bid < 87000.0);
        assert!(ask > 87000.0);
        assert!(bid < ask);
    }

    #[test]
    fn test_alpha_override() {
        let mut calc = make_calc();
        calc.set_alpha_override(Some(1.5));
        assert_eq!(calc.alpha_override, Some(1.5));
        calc.set_alpha_override(None);
        assert_eq!(calc.alpha_override, None);
    }
}
