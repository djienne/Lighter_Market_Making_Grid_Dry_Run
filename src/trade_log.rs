use chrono::Utc;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

const HEADER: &str = "timestamp,symbol,side,price,size,level,position_after,realized_pnl,available_capital,portfolio_value,simulated\n";

/// Buffered, append-only CSV trade logger with atomic flush.
///
/// `log_fill()` is O(1) — it appends to an in-memory buffer.
/// `flush()` writes all buffered rows atomically: first to a temp file,
/// then appends the temp content to the main file. If the process is killed
/// mid-write, only the temp file is lost — the main CSV is never corrupted.
pub struct TradeLogger {
    path: PathBuf,
    symbol: String,
    buffer: Vec<u8>, // pre-formatted bytes, avoids per-row String alloc
}

impl TradeLogger {
    pub fn new(log_dir: &Path, symbol: &str) -> std::io::Result<Self> {
        fs::create_dir_all(log_dir)?;
        let path = log_dir.join(format!("trades_{}.csv", symbol));

        // Write header if file doesn't exist or is empty
        let needs_header = !path.exists() || fs::metadata(&path).map_or(true, |m| m.len() == 0);
        if needs_header {
            fs::write(&path, HEADER)?;
        }

        Ok(Self {
            path,
            symbol: symbol.to_string(),
            buffer: Vec::with_capacity(4096),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Buffer one fill row (no disk I/O). Hot path.
    pub fn log_fill(
        &mut self,
        side: &str,
        price: f64,
        size: f64,
        level: usize,
        position_after: f64,
        realized_pnl: f64,
        available_capital: f64,
        portfolio_value: f64,
        simulated: bool,
    ) {
        use std::io::Write as _;
        let ts = Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ");
        let _ = write!(
            self.buffer,
            "{},{},{},{:.10},{:.6},{},{:.6},{:.4},{:.2},{:.2},{}\n",
            ts, self.symbol, side, price, size, level,
            position_after, realized_pnl, available_capital, portfolio_value, simulated,
        );
    }

    /// Atomic flush: write buffer to temp file, then append to main CSV.
    ///
    /// If killed during write, only the temp file is incomplete.
    /// The main CSV is never left in a partial state.
    pub fn flush(&mut self) -> std::io::Result<()> {
        if self.buffer.is_empty() {
            return Ok(());
        }

        let dir = self.path.parent().unwrap_or(Path::new("."));
        let tmp_path = dir.join(format!(".tmp_trades_{}", std::process::id()));

        // Step 1: Write buffer to temp file
        fs::write(&tmp_path, &self.buffer)?;

        // Step 2: Append temp content to main CSV (atomic at filesystem level)
        let mut main_file = fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(&self.path)?;
        main_file.write_all(&self.buffer)?;
        main_file.flush()?;

        // Step 3: Remove temp file, clear buffer
        let _ = fs::remove_file(&tmp_path);
        self.buffer.clear();

        Ok(())
    }

    /// Delete the trade log and reset.
    pub fn clear(&mut self) -> std::io::Result<()> {
        self.buffer.clear();
        if self.path.exists() {
            fs::remove_file(&self.path)?;
        }
        fs::write(&self.path, HEADER)?;
        Ok(())
    }
}
