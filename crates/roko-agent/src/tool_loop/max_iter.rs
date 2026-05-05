//! Maximum-iteration guard (§36.54).
//!
//! Prevents the tool loop from running indefinitely when the backend
//! keeps emitting tool calls without converging on a final answer.

pub use roko_core::defaults::DEFAULT_MAX_TOOL_ITERATIONS as DEFAULT_MAX_ITERATIONS;

/// Returns `true` when the loop has exhausted its iteration budget.
#[inline]
#[must_use]
pub const fn is_exhausted(iterations: usize, max: usize) -> bool {
    iterations >= max
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_iterations_not_exhausted() {
        assert!(!is_exhausted(0, 25));
    }

    #[test]
    fn at_limit_is_exhausted() {
        assert!(is_exhausted(25, 25));
    }

    #[test]
    fn past_limit_is_exhausted() {
        assert!(is_exhausted(30, 25));
    }

    #[test]
    fn zero_limit_always_exhausted() {
        assert!(is_exhausted(0, 0));
    }
}
