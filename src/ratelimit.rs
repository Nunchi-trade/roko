use std::time::Instant;

pub struct TokenBucket {
    rate: f64,
    capacity: u64,
    tokens: f64,
    last_refill: Instant,
}

impl TokenBucket {
    pub fn new(rate: f64, capacity: u64) -> Self {
        let rate = rate.max(0.0);
        Self {
            rate,
            capacity,
            tokens: capacity as f64,
            last_refill: Instant::now(),
        }
    }

    fn refill(&mut self) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_refill).as_secs_f64();
        if elapsed > 0.0 && self.rate > 0.0 {
            self.tokens = (self.tokens + elapsed * self.rate).min(self.capacity as f64);
        }
        self.last_refill = now;
    }

    pub fn try_acquire(&mut self) -> bool {
        self.try_acquire_n(1)
    }

    pub fn try_acquire_n(&mut self, n: u64) -> bool {
        if n == 0 {
            return true;
        }
        if n > self.capacity {
            return false;
        }

        self.refill();

        if self.tokens >= n as f64 {
            self.tokens -= n as f64;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::TokenBucket;
    use std::thread::sleep;
    use std::time::Duration;

    #[test]
    fn starts_full() {
        let mut bucket = TokenBucket::new(10.0, 3);
        assert!(bucket.try_acquire_n(3));
        assert!(!bucket.try_acquire());
    }

    #[test]
    fn rejects_more_than_capacity() {
        let mut bucket = TokenBucket::new(10.0, 3);
        assert!(!bucket.try_acquire_n(4));
    }

    #[test]
    fn refills_over_time() {
        let mut bucket = TokenBucket::new(10.0, 1);
        assert!(bucket.try_acquire());
        assert!(!bucket.try_acquire());
        sleep(Duration::from_millis(120));
        assert!(bucket.try_acquire());
    }

    #[test]
    fn zero_request_always_succeeds() {
        let mut bucket = TokenBucket::new(0.0, 0);
        assert!(bucket.try_acquire_n(0));
    }
}
