//! Robust statistics primitives resistant to outliers and adversarial
//! perturbation.
//!
//! These replace standard estimators with breakdown-resistant alternatives
//! (Huber 1964, Hampel 1974):
//!
//! - [`trimmed_mean`] — discard extremes before averaging (breakdown = trim_pct)
//! - [`mad`] — Median Absolute Deviation, robust scale estimator (breakdown 50%)
//! - [`hodges_lehmann`] — median of pairwise averages (breakdown 29%)

/// Compute the trimmed mean, discarding the top and bottom `trim_pct`
/// fraction of values before averaging.
///
/// `trim_pct` is in `[0.0, 0.5)` — e.g. 0.1 trims the lowest 10% and
/// highest 10%. The breakdown point equals `trim_pct`.
///
/// Returns `None` if the remaining set after trimming is empty.
pub fn trimmed_mean(values: &[f64], trim_pct: f64) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let trim_pct = trim_pct.clamp(0.0, 0.499);
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    let n = sorted.len();
    let trim_count = (n as f64 * trim_pct).floor() as usize;
    let lo = trim_count;
    let hi = n.saturating_sub(trim_count);
    if lo >= hi {
        return None;
    }
    let trimmed = &sorted[lo..hi];
    let sum: f64 = trimmed.iter().sum();
    Some(sum / trimmed.len() as f64)
}

/// Compute the Median Absolute Deviation (MAD).
///
/// `MAD = median(|x_i - median(x)|) * 1.4826`
///
/// The 1.4826 consistency factor makes MAD an unbiased estimator of the
/// standard deviation for normal distributions. Breakdown point is 50%.
///
/// Returns `None` for empty input.
pub fn mad(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let med = median(values)?;
    let abs_devs: Vec<f64> = values.iter().map(|x| (x - med).abs()).collect();
    let med_dev = median(&abs_devs)?;
    Some(med_dev * 1.4826)
}

/// Compute the Hodges-Lehmann estimator.
///
/// `HL = median((x_i + x_j) / 2)` over all pairs `i <= j`.
///
/// Highly robust (breakdown point 29.3%). For n values, computes
/// `n*(n+1)/2` pairwise averages. Returns `None` for empty input.
pub fn hodges_lehmann(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let n = values.len();
    let mut pairwise: Vec<f64> = Vec::with_capacity(n * (n + 1) / 2);
    for i in 0..n {
        for j in i..n {
            pairwise.push((values[i] + values[j]) / 2.0);
        }
    }
    median(&pairwise)
}

/// Helper: compute the median of a slice.
fn median(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = sorted.len();
    if n % 2 == 0 {
        Some((sorted[n / 2 - 1] + sorted[n / 2]) / 2.0)
    } else {
        Some(sorted[n / 2])
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trimmed_mean_no_trim() {
        let vals = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let m = trimmed_mean(&vals, 0.0).unwrap();
        assert!((m - 3.0).abs() < 1e-9);
    }

    #[test]
    fn trimmed_mean_removes_outliers() {
        // With outliers at both ends, 20% trim removes them.
        let vals = vec![-1000.0, 2.0, 3.0, 4.0, 1000.0];
        let m = trimmed_mean(&vals, 0.2).unwrap();
        // Trims 1 from each side: mean(2,3,4) = 3.0
        assert!((m - 3.0).abs() < 1e-9);
    }

    #[test]
    fn trimmed_mean_empty() {
        assert!(trimmed_mean(&[], 0.1).is_none());
    }

    #[test]
    fn mad_normal_data() {
        // For [1,2,3,4,5]: median=3, abs_devs=[2,1,0,1,2], median_dev=1.
        // MAD = 1 * 1.4826 = 1.4826
        let vals = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let m = mad(&vals).unwrap();
        assert!((m - 1.4826).abs() < 1e-4);
    }

    #[test]
    fn mad_single_value() {
        let m = mad(&[42.0]).unwrap();
        assert!((m - 0.0).abs() < 1e-9);
    }

    #[test]
    fn mad_empty() {
        assert!(mad(&[]).is_none());
    }

    #[test]
    fn hodges_lehmann_symmetric() {
        // For [1,2,3]: pairs = (1,1.5,2, 2,2.5, 3) -> median = 2.0
        let vals = vec![1.0, 2.0, 3.0];
        let hl = hodges_lehmann(&vals).unwrap();
        assert!((hl - 2.0).abs() < 1e-9);
    }

    #[test]
    fn hodges_lehmann_robust_to_outlier() {
        // Without outlier: [1,2,3,4,5] -> HL = 3.0
        // With outlier: [1,2,3,4,1000] -> HL should stay near 3
        let clean = hodges_lehmann(&[1.0, 2.0, 3.0, 4.0, 5.0]).unwrap();
        let dirty = hodges_lehmann(&[1.0, 2.0, 3.0, 4.0, 1000.0]).unwrap();
        assert!((clean - 3.0).abs() < 1e-9);
        // HL with one outlier should still be reasonably close to the center.
        assert!(dirty > 2.0 && dirty < 10.0, "HL with outlier = {dirty}");
    }

    #[test]
    fn hodges_lehmann_empty() {
        assert!(hodges_lehmann(&[]).is_none());
    }

    #[test]
    fn median_even_odd() {
        assert!((median(&[1.0, 3.0]).unwrap() - 2.0).abs() < 1e-9);
        assert!((median(&[1.0, 2.0, 3.0]).unwrap() - 2.0).abs() < 1e-9);
    }
}
