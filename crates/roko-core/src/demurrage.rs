//! Time-decay tax on stored value (Gesellian demurrage).
//!
//! Implementors must be refreshed (validated, used) to maintain their balance.
//! Neglected items naturally fade, ensuring active validation of knowledge.
//!
//! This trait formalizes the demurrage pattern already present on
//! [`PlaybookRules::Rule`] and the `DemurrageConfig` in `roko.toml`.

/// Time-decay tax on stored value -- ensures active validation.
///
/// Implementors must be refreshed (validated, used) to maintain
/// their balance. Neglected items naturally fade.
///
/// The decay formula is: `balance *= (1 - rate)^elapsed_hours`
pub trait Demurrage {
    /// Current attention/value balance in `[0.0, 1.0]`.
    fn balance(&self) -> f64;

    /// Hourly decay rate in `[0.0, 1.0]`.
    fn demurrage_rate(&self) -> f64;

    /// Apply time-based decay for `elapsed_hours` of inactivity.
    fn tick(&mut self, elapsed_hours: f64);

    /// Replenish the balance (capped at 1.0) after active validation.
    fn replenish(&mut self, amount: f64);

    /// Returns `true` when the balance has fallen below the usable threshold.
    fn is_depleted(&self) -> bool {
        self.balance() < 0.1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestItem {
        balance: f64,
        rate: f64,
    }

    impl Demurrage for TestItem {
        fn balance(&self) -> f64 {
            self.balance
        }

        fn demurrage_rate(&self) -> f64 {
            self.rate
        }

        fn tick(&mut self, elapsed_hours: f64) {
            if elapsed_hours > 0.0 {
                self.balance *= (1.0 - self.rate).powf(elapsed_hours);
            }
        }

        fn replenish(&mut self, amount: f64) {
            self.balance = (self.balance + amount).min(1.0);
        }
    }

    #[test]
    fn tick_decays_balance() {
        let mut item = TestItem {
            balance: 1.0,
            rate: 0.01,
        };
        item.tick(100.0);
        assert!(item.balance() < 1.0);
        assert!(item.balance() > 0.0);
    }

    #[test]
    fn replenish_caps_at_one() {
        let mut item = TestItem {
            balance: 0.5,
            rate: 0.01,
        };
        item.replenish(0.8);
        assert!((item.balance() - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn is_depleted_when_low() {
        let item = TestItem {
            balance: 0.05,
            rate: 0.01,
        };
        assert!(item.is_depleted());
    }

    #[test]
    fn not_depleted_when_healthy() {
        let item = TestItem {
            balance: 0.5,
            rate: 0.01,
        };
        assert!(!item.is_depleted());
    }

    #[test]
    fn zero_elapsed_does_not_decay() {
        let mut item = TestItem {
            balance: 1.0,
            rate: 0.01,
        };
        item.tick(0.0);
        assert!((item.balance() - 1.0).abs() < f64::EPSILON);
    }
}
