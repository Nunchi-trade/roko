//! Volatility-regime binning + drawdown-linked spread amplifiers.
//!
//! Ports `strategies/risk_multipliers.py`.

const T_ROUND_SEC: f64 = 20.0;

/// sqrt(seconds-per-year / round-duration).
#[inline]
fn annualize_factor() -> f64 {
    ((365.0 * 24.0 * 3600.0) / T_ROUND_SEC).sqrt()
}

/// `(upper_threshold_exclusive, multiplier, name)` — rows from
/// `strategies.risk_multipliers.VOL_BINS`.
pub const VOL_BINS: &[(f64, f64, &str)] = &[
    (0.15, 1.0, "I_low"),
    (0.40, 1.5, "II_normal"),
    (0.80, 2.5, "III_high"),
    (f64::INFINITY, 5.0, "IV_extreme"),
];

/// Drawdown bins: `(upper_threshold_exclusive_pct, multiplier, name)`.
pub const DD_BINS: &[(f64, f64, &str)] = &[
    (0.5, 1.0, "green"),
    (1.5, 1.5, "yellow"),
    (2.5, 2.0, "orange"),
    (f64::INFINITY, f64::INFINITY, "red"),
];

const HYSTERESIS_ROUNDS: u32 = 3;

/// Classified vol regime — mirrors the Python tuple `(m_vol, name)` but
/// typed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VolBin {
    pub multiplier: f64,
    pub name: &'static str,
}

/// Drawdown bin lookup result.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DdBin {
    pub multiplier: f64,
    pub name: &'static str,
}

/// Classifies realized vol into regime bins with hysteresis. Upward
/// transitions are immediate; downward transitions require
/// `HYSTERESIS_ROUNDS` consecutive rounds in a lower bin before dropping.
#[derive(Debug, Default)]
pub struct VolBinClassifier {
    current_idx: usize,
    downward_candidate: Option<usize>,
    downward_rounds: u32,
}

impl VolBinClassifier {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            current_idx: 0,
            downward_candidate: None,
            downward_rounds: 0,
        }
    }

    #[must_use]
    pub fn annualize(&self, sigma_log_std: f64) -> f64 {
        sigma_log_std * annualize_factor()
    }

    pub fn classify(&mut self, sigma_log_std: f64) -> VolBin {
        let sigma_ann = self.annualize(sigma_log_std);
        let target = Self::find_bin_idx(sigma_ann);

        match target.cmp(&self.current_idx) {
            std::cmp::Ordering::Greater => {
                self.current_idx = target;
                self.downward_candidate = None;
                self.downward_rounds = 0;
            }
            std::cmp::Ordering::Less => {
                if self.downward_candidate == Some(target) {
                    self.downward_rounds += 1;
                } else {
                    self.downward_candidate = Some(target);
                    self.downward_rounds = 1;
                }
                if self.downward_rounds >= HYSTERESIS_ROUNDS {
                    self.current_idx = target;
                    self.downward_candidate = None;
                    self.downward_rounds = 0;
                }
            }
            std::cmp::Ordering::Equal => {
                self.downward_candidate = None;
                self.downward_rounds = 0;
            }
        }

        let (_, mult, name) = VOL_BINS[self.current_idx];
        VolBin {
            multiplier: mult,
            name,
        }
    }

    fn find_bin_idx(sigma_ann: f64) -> usize {
        VOL_BINS
            .iter()
            .position(|(threshold, _, _)| sigma_ann < *threshold)
            .unwrap_or(VOL_BINS.len() - 1)
    }
}

/// Returns the drawdown multiplier bin for a given drawdown percentage.
#[must_use]
pub fn dd_multiplier(daily_drawdown_pct: f64) -> DdBin {
    for &(threshold, mult, name) in DD_BINS {
        if daily_drawdown_pct < threshold {
            return DdBin {
                multiplier: mult,
                name,
            };
        }
    }
    DdBin {
        multiplier: f64::INFINITY,
        name: "red",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vol_bins_monotone_thresholds() {
        for pair in VOL_BINS.windows(2) {
            assert!(pair[0].0 < pair[1].0);
        }
    }

    #[test]
    fn dd_multiplier_green_below_half() {
        let bin = dd_multiplier(0.25);
        assert_eq!(bin.name, "green");
        assert!((bin.multiplier - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn dd_multiplier_red_at_extreme() {
        let bin = dd_multiplier(10.0);
        assert_eq!(bin.name, "red");
        assert!(bin.multiplier.is_infinite());
    }

    #[test]
    fn vol_upward_transition_is_immediate() {
        let mut c = VolBinClassifier::new();
        // 0.2 un-annualized * 16.18 (annualize factor) ≈ 3.2 → IV_extreme
        let bin = c.classify(0.2);
        assert_eq!(bin.name, "IV_extreme");
    }

    #[test]
    fn vol_downward_requires_hysteresis() {
        let mut c = VolBinClassifier::new();
        assert_eq!(c.classify(0.2).name, "IV_extreme"); // push up
        // Round 1 + 2 at low: still latched high (HYSTERESIS_ROUNDS = 3).
        assert_eq!(c.classify(0.0).name, "IV_extreme");
        assert_eq!(c.classify(0.0).name, "IV_extreme");
        // Round 3 triggers the downward transition.
        assert_eq!(c.classify(0.0).name, "I_low");
    }
}
