/// Fixed-capacity ring buffer with O(1) incremental mean / std / zscore.
///
/// Uses Welford's online algorithm for numerically stable variance.
/// Caches mean and std on every push() so zscore() is a single division.
pub struct RollingStats {
    buffer: Vec<f64>,
    capacity: usize,
    write_pos: usize,
    count: usize,
    sum: f64,
    m2: f64, // sum of squared differences from mean (Welford)
    cached_mean: f64,
    cached_std: f64,
}

impl RollingStats {
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0);
        Self {
            buffer: vec![0.0; capacity],
            capacity,
            write_pos: 0,
            count: 0,
            sum: 0.0,
            m2: 0.0,
            cached_mean: 0.0,
            cached_std: 0.0,
        }
    }

    pub fn push(&mut self, value: f64) {
        let idx = self.write_pos % self.capacity;

        // Evict oldest value if buffer is full (reverse Welford update)
        if self.count >= self.capacity {
            let old = self.buffer[idx];
            let n = self.count;
            let old_mean = self.cached_mean;
            let new_mean = if n > 1 {
                old_mean + (old_mean - old) / (n as f64 - 1.0)
            } else {
                0.0
            };
            self.m2 -= (old - old_mean) * (old - new_mean);
            self.m2 = self.m2.max(0.0);
            self.sum -= old;
            self.count -= 1;
            self.cached_mean = new_mean;
        }

        // Add new value (forward Welford update)
        self.buffer[idx] = value;
        self.sum += value;
        self.count += 1;
        self.write_pos += 1;
        let n = self.count as f64;
        let old_mean = self.cached_mean;
        let new_mean = self.sum / n;
        self.m2 += (value - old_mean) * (value - new_mean);
        self.m2 = self.m2.max(0.0);
        self.cached_mean = new_mean;
        self.cached_std = if self.count >= 2 {
            (self.m2 / n).sqrt()
        } else {
            0.0
        };
    }

    pub fn clear(&mut self) {
        self.write_pos = 0;
        self.count = 0;
        self.sum = 0.0;
        self.m2 = 0.0;
        self.cached_mean = 0.0;
        self.cached_std = 0.0;
    }

    pub fn count(&self) -> usize {
        self.count
    }

    pub fn mean(&self) -> f64 {
        self.cached_mean
    }

    pub fn std(&self) -> f64 {
        self.cached_std
    }

    pub fn zscore(&self, value: f64) -> f64 {
        if self.cached_std < 1e-10 {
            return 0.0;
        }
        (value - self.cached_mean) / self.cached_std
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_push_and_stats() {
        let mut rs = RollingStats::new(5);
        for v in [1.0, 2.0, 3.0, 4.0, 5.0] {
            rs.push(v);
        }
        assert_eq!(rs.count(), 5);
        assert!((rs.mean() - 3.0).abs() < 1e-10);
        // Population std of [1,2,3,4,5] = sqrt(2)
        assert!((rs.std() - 2.0_f64.sqrt()).abs() < 1e-10);
    }

    #[test]
    fn test_eviction() {
        let mut rs = RollingStats::new(3);
        for v in [1.0, 2.0, 3.0] {
            rs.push(v);
        }
        assert_eq!(rs.count(), 3);
        assert!((rs.mean() - 2.0).abs() < 1e-10);

        // Push 4.0, evicts 1.0 → window is [2, 3, 4]
        rs.push(4.0);
        assert_eq!(rs.count(), 3);
        assert!((rs.mean() - 3.0).abs() < 1e-10);
    }

    #[test]
    fn test_zscore() {
        let mut rs = RollingStats::new(100);
        for v in [10.0, 20.0, 30.0, 40.0, 50.0] {
            rs.push(v);
        }
        // zscore of mean should be 0
        assert!((rs.zscore(rs.mean())).abs() < 1e-10);
        // zscore of value above mean should be positive
        assert!(rs.zscore(60.0) > 0.0);
    }

    #[test]
    fn test_clear() {
        let mut rs = RollingStats::new(5);
        rs.push(1.0);
        rs.push(2.0);
        rs.clear();
        assert_eq!(rs.count(), 0);
        assert_eq!(rs.mean(), 0.0);
        assert_eq!(rs.std(), 0.0);
    }

    #[test]
    fn test_single_value() {
        let mut rs = RollingStats::new(5);
        rs.push(42.0);
        assert_eq!(rs.count(), 1);
        assert!((rs.mean() - 42.0).abs() < 1e-10);
        assert_eq!(rs.std(), 0.0); // need >= 2 samples
    }
}
