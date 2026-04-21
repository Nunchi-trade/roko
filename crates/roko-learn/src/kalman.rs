//! Kalman filter for online signal smoothing in oracle predictions.
//!
//! Implements the standard scalar (1D) Kalman filter for smoothing noisy
//! time-series observations. Useful for:
//!
//! - Oracle prediction smoothing (removing noise from rate observations)
//! - ISFR rate tracking (smoothing submitted rates over epochs)
//! - Performance metric denoising (gate pass rates, latency, cost)
//! - Adaptive threshold tracking (smoothing gate threshold adjustments)
//!
//! ## Model
//!
//! State-space model for a random walk with noise:
//! ```text
//! x_t = x_{t-1} + w_t    (state transition: random walk)
//! z_t = x_t + v_t         (observation: state + measurement noise)
//!
//! w_t ~ N(0, Q)  (process noise)
//! v_t ~ N(0, R)  (measurement noise)
//! ```
//!
//! ## Example
//!
//! ```rust
//! use roko_learn::kalman::KalmanFilter;
//!
//! let mut kf = KalmanFilter::new(0.0, 1.0, 0.01, 0.1);
//! // Feed noisy observations of a signal near 5.0
//! for obs in [4.8, 5.2, 4.9, 5.1, 5.0, 4.7, 5.3] {
//!     kf.update(obs);
//! }
//! // Filtered estimate should be close to 5.0
//! assert!((kf.state() - 5.0).abs() < 0.5);
//! ```

/// Scalar (1D) Kalman filter for online signal smoothing.
///
/// Tracks a single state variable through noisy observations using the
/// predict-update cycle. The filter automatically balances responsiveness
/// (tracking real changes) vs smoothness (rejecting noise) based on the
/// configured process noise (Q) and measurement noise (R) parameters.
#[derive(Debug, Clone, PartialEq)]
pub struct KalmanFilter {
    /// Current state estimate.
    x: f64,
    /// Current error covariance (uncertainty in the state estimate).
    p: f64,
    /// Process noise variance (how much the true state changes per step).
    /// Higher Q → filter trusts observations more (more responsive, less smooth).
    q: f64,
    /// Measurement noise variance (how noisy observations are).
    /// Higher R → filter trusts observations less (smoother, less responsive).
    r: f64,
    /// Number of observations processed.
    count: u64,
}

impl KalmanFilter {
    /// Create a new Kalman filter.
    ///
    /// # Arguments
    /// - `initial_state`: initial estimate of the state
    /// - `initial_covariance`: initial uncertainty (larger = less confident in initial estimate)
    /// - `process_noise`: Q — expected variance of state changes per step
    /// - `measurement_noise`: R — expected variance of observation noise
    pub fn new(
        initial_state: f64,
        initial_covariance: f64,
        process_noise: f64,
        measurement_noise: f64,
    ) -> Self {
        Self {
            x: initial_state,
            p: initial_covariance.max(1e-10),
            q: process_noise.max(0.0),
            r: measurement_noise.max(1e-10),
            count: 0,
        }
    }

    /// Create a filter tuned for smoothing oracle predictions.
    ///
    /// Low process noise, moderate measurement noise — assumes the true signal
    /// changes slowly and observations are moderately noisy.
    pub fn for_oracle_smoothing(initial_estimate: f64) -> Self {
        Self::new(initial_estimate, 1.0, 0.001, 0.1)
    }

    /// Create a filter tuned for tracking fast-changing signals.
    ///
    /// Higher process noise — the filter is more responsive to real changes
    /// at the cost of more noise in the output.
    pub fn for_tracking(initial_estimate: f64) -> Self {
        Self::new(initial_estimate, 1.0, 0.1, 0.1)
    }

    /// Predict step: propagate state and covariance forward.
    ///
    /// For a random-walk model: state doesn't change, but uncertainty grows by Q.
    pub fn predict(&mut self) {
        // x_predicted = x (random walk: state doesn't change in prediction)
        // p_predicted = p + Q (uncertainty grows)
        self.p += self.q;
    }

    /// Update step: incorporate a new observation.
    ///
    /// Combines the prediction with the observation using the Kalman gain.
    pub fn update(&mut self, observation: f64) {
        // Predict step (implicit).
        self.predict();

        // Kalman gain: K = P / (P + R)
        let k = self.p / (self.p + self.r);

        // State update: x = x + K * (z - x)
        let innovation = observation - self.x;
        self.x += k * innovation;

        // Covariance update: P = (1 - K) * P
        self.p *= 1.0 - k;

        self.count += 1;
    }

    /// Process a batch of observations, returning the final smoothed estimate.
    pub fn update_batch(&mut self, observations: &[f64]) {
        for &obs in observations {
            self.update(obs);
        }
    }

    /// Current state estimate (the smoothed value).
    pub fn state(&self) -> f64 {
        self.x
    }

    /// Current error covariance (uncertainty in the estimate).
    pub fn covariance(&self) -> f64 {
        self.p
    }

    /// Current Kalman gain (how much the filter trusts new observations).
    ///
    /// Gain near 1.0 → trusting observations (early or high Q).
    /// Gain near 0.0 → trusting state estimate (converged or high R).
    pub fn gain(&self) -> f64 {
        self.p / (self.p + self.r)
    }

    /// Number of observations processed.
    pub fn observation_count(&self) -> u64 {
        self.count
    }

    /// Innovation (residual) for a hypothetical observation without updating.
    ///
    /// Useful for anomaly detection: large innovations suggest the observation
    /// is far from the current estimate.
    pub fn innovation(&self, observation: f64) -> f64 {
        observation - self.x
    }

    /// Normalized innovation squared (Mahalanobis distance).
    ///
    /// Values > 4.0 (i.e., > 2 standard deviations) suggest an anomaly.
    pub fn normalized_innovation_sq(&self, observation: f64) -> f64 {
        let inn = self.innovation(observation);
        let s = self.p + self.r; // Innovation covariance
        if s > 0.0 { inn * inn / s } else { 0.0 }
    }

    /// Reset the filter to a new initial state.
    pub fn reset(&mut self, state: f64, covariance: f64) {
        self.x = state;
        self.p = covariance.max(1e-10);
        self.count = 0;
    }

    /// Process noise (Q).
    pub fn process_noise(&self) -> f64 {
        self.q
    }

    /// Measurement noise (R).
    pub fn measurement_noise(&self) -> f64 {
        self.r
    }

    /// Adjust process noise dynamically.
    ///
    /// Useful when the signal regime changes (e.g., increase Q during volatile periods).
    pub fn set_process_noise(&mut self, q: f64) {
        self.q = q.max(0.0);
    }

    /// Adjust measurement noise dynamically.
    pub fn set_measurement_noise(&mut self, r: f64) {
        self.r = r.max(1e-10);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converges_to_true_value() {
        let mut kf = KalmanFilter::new(0.0, 1.0, 0.001, 0.1);

        // Feed 100 observations of a constant signal at 5.0 with noise.
        for i in 0..100 {
            let noise = ((i * 7 + 3) % 11) as f64 / 11.0 - 0.5; // Deterministic pseudo-noise
            kf.update(5.0 + noise * 0.5);
        }

        assert!(
            (kf.state() - 5.0).abs() < 0.3,
            "should converge to ~5.0, got {}",
            kf.state()
        );
    }

    #[test]
    fn covariance_decreases_with_observations() {
        let mut kf = KalmanFilter::new(0.0, 10.0, 0.001, 0.1);
        let initial_cov = kf.covariance();

        for _ in 0..20 {
            kf.update(1.0);
        }

        assert!(
            kf.covariance() < initial_cov,
            "covariance should decrease: {} vs {}",
            kf.covariance(),
            initial_cov
        );
    }

    #[test]
    fn gain_decreases_as_filter_converges() {
        let mut kf = KalmanFilter::new(0.0, 10.0, 0.001, 0.1);
        let initial_gain = kf.gain();

        for _ in 0..50 {
            kf.update(3.0);
        }

        let converged_gain = kf.gain();
        assert!(
            converged_gain < initial_gain,
            "gain should decrease: {} vs {}",
            converged_gain,
            initial_gain
        );
    }

    #[test]
    fn high_process_noise_tracks_faster() {
        let mut slow = KalmanFilter::new(0.0, 1.0, 0.001, 0.1);
        let mut fast = KalmanFilter::new(0.0, 1.0, 1.0, 0.1);

        // Step change from 0 to 10.
        for _ in 0..5 {
            slow.update(10.0);
            fast.update(10.0);
        }

        assert!(
            fast.state() > slow.state(),
            "high Q should track faster: fast={}, slow={}",
            fast.state(),
            slow.state()
        );
    }

    #[test]
    fn high_measurement_noise_smooths_more() {
        let mut noisy = KalmanFilter::new(5.0, 1.0, 0.01, 10.0);
        let mut trusting = KalmanFilter::new(5.0, 1.0, 0.01, 0.01);

        // Single outlier observation.
        noisy.update(100.0);
        trusting.update(100.0);

        // High R should resist the outlier more.
        assert!(
            noisy.state() < trusting.state(),
            "high R should resist outlier: noisy={}, trusting={}",
            noisy.state(),
            trusting.state()
        );
    }

    #[test]
    fn anomaly_detection_via_innovation() {
        let mut kf = KalmanFilter::new(5.0, 0.1, 0.001, 0.1);

        // After convergence, a normal observation should have low NIS.
        for _ in 0..20 {
            kf.update(5.0);
        }

        let normal_nis = kf.normalized_innovation_sq(5.1);
        let anomaly_nis = kf.normalized_innovation_sq(50.0);

        assert!(
            normal_nis < 4.0,
            "normal obs should have NIS < 4: {normal_nis}"
        );
        assert!(
            anomaly_nis > 4.0,
            "anomaly should have NIS > 4: {anomaly_nis}"
        );
    }

    #[test]
    fn batch_update() {
        let mut kf = KalmanFilter::new(0.0, 1.0, 0.01, 0.1);
        kf.update_batch(&[3.0, 3.1, 2.9, 3.0, 3.2]);

        assert!(
            (kf.state() - 3.0).abs() < 0.5,
            "batch should converge to ~3.0, got {}",
            kf.state()
        );
        assert_eq!(kf.observation_count(), 5);
    }

    #[test]
    fn factory_methods() {
        let oracle = KalmanFilter::for_oracle_smoothing(10.0);
        assert!((oracle.state() - 10.0).abs() < f64::EPSILON);
        assert!(oracle.process_noise() < oracle.measurement_noise());

        let tracker = KalmanFilter::for_tracking(10.0);
        assert!((tracker.process_noise() - tracker.measurement_noise()).abs() < f64::EPSILON);
    }

    #[test]
    fn reset_clears_state() {
        let mut kf = KalmanFilter::new(0.0, 1.0, 0.01, 0.1);
        for _ in 0..50 {
            kf.update(10.0);
        }
        assert!(kf.observation_count() > 0);

        kf.reset(0.0, 5.0);
        assert_eq!(kf.observation_count(), 0);
        assert!((kf.state() - 0.0).abs() < f64::EPSILON);
        assert!((kf.covariance() - 5.0).abs() < f64::EPSILON);
    }
}
