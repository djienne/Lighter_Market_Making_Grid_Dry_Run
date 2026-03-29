use ordered_float::OrderedFloat;
use std::collections::BTreeMap;

pub type PriceLevel = OrderedFloat<f64>;
pub type BookSide = BTreeMap<PriceLevel, f64>;

pub struct Orderbook {
    pub bids: BookSide,
    pub asks: BookSide,
    pub initialized: bool,
}

impl Orderbook {
    pub fn new() -> Self {
        Self {
            bids: BTreeMap::new(),
            asks: BTreeMap::new(),
            initialized: false,
        }
    }

    /// Apply an orderbook update (snapshot or delta).
    /// Returns true if treated as a snapshot.
    pub fn apply_update(
        &mut self,
        bids_in: &[(f64, f64)],
        asks_in: &[(f64, f64)],
        snapshot_threshold: usize,
    ) -> bool {
        let is_snapshot = if !self.initialized {
            true
        } else {
            bids_in.len() > snapshot_threshold || asks_in.len() > snapshot_threshold
        };

        if is_snapshot {
            self.bids.clear();
            self.asks.clear();
            for &(price, size) in bids_in {
                if size > 0.0 {
                    self.bids.insert(OrderedFloat(price), size);
                }
            }
            for &(price, size) in asks_in {
                if size > 0.0 {
                    self.asks.insert(OrderedFloat(price), size);
                }
            }
            self.initialized = true;
        } else {
            for &(price, size) in bids_in {
                let key = OrderedFloat(price);
                if size == 0.0 {
                    self.bids.remove(&key);
                } else {
                    self.bids.insert(key, size);
                }
            }
            for &(price, size) in asks_in {
                let key = OrderedFloat(price);
                if size == 0.0 {
                    self.asks.remove(&key);
                } else {
                    self.asks.insert(key, size);
                }
            }
        }

        is_snapshot
    }

    pub fn best_bid(&self) -> Option<f64> {
        self.bids.keys().next_back().map(|k| k.into_inner())
    }

    pub fn best_ask(&self) -> Option<f64> {
        self.asks.keys().next().map(|k| k.into_inner())
    }

    pub fn mid(&self) -> Option<f64> {
        match (self.best_bid(), self.best_ask()) {
            (Some(b), Some(a)) => Some((b + a) / 2.0),
            _ => None,
        }
    }

    pub fn clear(&mut self) {
        self.bids.clear();
        self.asks.clear();
        self.initialized = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_snapshot() {
        let mut ob = Orderbook::new();
        let bids = vec![(100.0, 1.0), (99.0, 2.0)];
        let asks = vec![(101.0, 1.5), (102.0, 3.0)];
        let is_snap = ob.apply_update(&bids, &asks, 100);
        assert!(is_snap);
        assert!(ob.initialized);
        assert_eq!(ob.best_bid(), Some(100.0));
        assert_eq!(ob.best_ask(), Some(101.0));
        assert!((ob.mid().unwrap() - 100.5).abs() < 1e-10);
    }

    #[test]
    fn test_delta() {
        let mut ob = Orderbook::new();
        // First update = snapshot
        ob.apply_update(&[(100.0, 1.0)], &[(101.0, 1.0)], 100);
        // Delta: update bid, remove ask, add new ask
        ob.apply_update(&[(100.0, 2.0)], &[(101.0, 0.0), (101.5, 1.0)], 100);
        assert_eq!(*ob.bids.get(&OrderedFloat(100.0)).unwrap(), 2.0);
        assert!(!ob.asks.contains_key(&OrderedFloat(101.0)));
        assert_eq!(ob.best_ask(), Some(101.5));
    }

    #[test]
    fn test_zero_size_ignored_on_snapshot() {
        let mut ob = Orderbook::new();
        ob.apply_update(&[(100.0, 0.0), (99.0, 1.0)], &[], 100);
        assert!(!ob.bids.contains_key(&OrderedFloat(100.0)));
        assert_eq!(ob.best_bid(), Some(99.0));
    }
}
