//! End-to-end integration tests for the Roko primitive stack.
//!
//! The real tests live in `tests/*.rs`; this lib exists only to satisfy
//! the crate root requirement.

/// Placeholder sanity-check helper used by integration tests.
pub fn add(a: i32, b: i32) -> i32 {
    a + b
}
