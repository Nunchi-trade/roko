//! Rolling log-return vol estimator. Ports
//! `quoting_engine/vol_estimator.py`.

use std::collections::VecDeque;

#[derive(Debug, Clone)]
pub struct RollingVolEstimator {
    window: usize,
    min_samples: usize,
    prices: VecDeque<f64>,
    log_returns: VecDeque<f64>,
}

impl RollingVolEstimator {
    #[must_use]
    pub fn new(window: usize) -> Self {
        Self {
            window,
            min_samples: 3,
            prices: VecDeque::with_capacity(window),
            log_returns: VecDeque::with_capacity(window),
        }
    }

    /// Returns `(sigma_price, sigma_log)`. `sigma_price = sigma_log * mid`.
    pub fn update(&mut self, mid: f64) -> (f64, f64) {
        if let Some(&prev) = self.prices.back() {
            if prev > 0.0 && mid > 0.0 {
                if self.log_returns.len() == self.window {
                    self.log_returns.pop_front();
                }
                self.log_returns.push_back((mid / prev).ln());
            }
        }
        if self.prices.len() == self.window {
            self.prices.pop_front();
        }
        self.prices.push_back(mid);

        if self.log_returns.len() < self.min_samples {
            let fallback = 3.0 / 10_000.0;
            return (mid * fallback, fallback);
        }
        let n = self.log_returns.len() as f64;
        let mean = self.log_returns.iter().sum::<f64>() / n;
        let var = self.log_returns.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / n;
        let log_std = var.max(1e-12).sqrt();
        (log_std * mid, log_std)
    }

    #[must_use]
    pub fn ready(&self) -> bool {
        self.log_returns.len() >= self.min_samples
    }
}
