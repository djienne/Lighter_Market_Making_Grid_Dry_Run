use std::time::Instant;

// ---------------------------------------------------------------------------
// Side / Action enums
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Buy,
    Sell,
}

impl Side {
    pub fn as_str(&self) -> &'static str {
        match self {
            Side::Buy => "buy",
            Side::Sell => "sell",
        }
    }
}

impl std::fmt::Display for Side {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Create,
    Modify,
    Cancel,
}

// ---------------------------------------------------------------------------
// BatchOp — a single order operation
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct BatchOp {
    pub side: Side,
    pub level: usize,
    pub action: Action,
    pub price: f64,
    pub size: f64,
    pub order_id: i64,
    pub exchange_id: i64,
}

// ---------------------------------------------------------------------------
// SideStatus / SideOrderLifecycle
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SideStatus {
    Idle,
    Placing,
    Live,
    Modifying,
    Canceling,
}

#[derive(Clone)]
pub struct SideOrderLifecycle {
    pub status: SideStatus,
    pub pending_order_id: Option<i64>,
    pub target_price: Option<f64>,
    pub target_size: Option<f64>,
    pub updated_at: Instant,
}

impl Default for SideOrderLifecycle {
    fn default() -> Self {
        Self {
            status: SideStatus::Idle,
            pending_order_id: None,
            target_price: None,
            target_size: None,
            updated_at: Instant::now(),
        }
    }
}

// ---------------------------------------------------------------------------
// OrderState — canonical order positions per level
// ---------------------------------------------------------------------------

pub struct OrderState {
    pub bid_order_ids: Vec<Option<i64>>,
    pub ask_order_ids: Vec<Option<i64>>,
    pub bid_prices: Vec<Option<f64>>,
    pub ask_prices: Vec<Option<f64>>,
    pub bid_sizes: Vec<Option<f64>>,
    pub ask_sizes: Vec<Option<f64>>,
}

impl OrderState {
    pub fn new(num_levels: usize) -> Self {
        Self {
            bid_order_ids: vec![None; num_levels],
            ask_order_ids: vec![None; num_levels],
            bid_prices: vec![None; num_levels],
            ask_prices: vec![None; num_levels],
            bid_sizes: vec![None; num_levels],
            ask_sizes: vec![None; num_levels],
        }
    }
}

// ---------------------------------------------------------------------------
// OrderManager — lifecycle tracking
// ---------------------------------------------------------------------------

pub struct OrderManager {
    pub bids: Vec<SideOrderLifecycle>,
    pub asks: Vec<SideOrderLifecycle>,
    num_levels: usize,
}

impl OrderManager {
    pub fn new(num_levels: usize) -> Self {
        Self {
            bids: vec![SideOrderLifecycle::default(); num_levels],
            asks: vec![SideOrderLifecycle::default(); num_levels],
            num_levels,
        }
    }

    pub fn lifecycle(&self, side: Side, level: usize) -> &SideOrderLifecycle {
        match side {
            Side::Buy => &self.bids[level],
            Side::Sell => &self.asks[level],
        }
    }

    pub fn mark_status(
        &mut self,
        side: Side,
        status: SideStatus,
        level: usize,
        target_price: Option<f64>,
        target_size: Option<f64>,
    ) {
        let lc = match side {
            Side::Buy => &mut self.bids[level],
            Side::Sell => &mut self.asks[level],
        };
        lc.status = status;
        lc.target_price = target_price;
        lc.target_size = target_size;
        lc.updated_at = Instant::now();
    }

    /// Bind an order as LIVE in the order state and lifecycle.
    pub fn bind_live(
        &mut self,
        orders: &mut OrderState,
        side: Side,
        order_id: i64,
        price: f64,
        size: f64,
        level: usize,
    ) {
        match side {
            Side::Buy => {
                orders.bid_order_ids[level] = Some(order_id);
                orders.bid_prices[level] = Some(price);
                orders.bid_sizes[level] = Some(size);
            }
            Side::Sell => {
                orders.ask_order_ids[level] = Some(order_id);
                orders.ask_prices[level] = Some(price);
                orders.ask_sizes[level] = Some(size);
            }
        }
        self.mark_status(side, SideStatus::Live, level, Some(price), Some(size));
    }

    /// Clear an order from the order state and set lifecycle to IDLE.
    pub fn clear_live(&mut self, orders: &mut OrderState, side: Side, level: usize) {
        match side {
            Side::Buy => {
                orders.bid_order_ids[level] = None;
                orders.bid_prices[level] = None;
                orders.bid_sizes[level] = None;
            }
            Side::Sell => {
                orders.ask_order_ids[level] = None;
                orders.ask_prices[level] = None;
                orders.ask_sizes[level] = None;
            }
        }
        self.mark_status(side, SideStatus::Idle, level, None, None);
    }

    /// Clear all levels for both sides.
    pub fn clear_all(&mut self, orders: &mut OrderState) {
        for level in 0..self.num_levels {
            self.clear_live(orders, Side::Buy, level);
            self.clear_live(orders, Side::Sell, level);
        }
    }
}
