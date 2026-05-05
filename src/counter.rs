use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

#[derive(Clone, Default)]
pub struct Counter {
    value: Arc<AtomicU64>,
}

impl Counter {
    pub fn new() -> Self {
        Self {
            value: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn increment(&self) -> u64 {
        self.value.fetch_add(1, Ordering::SeqCst) + 1
    }

    pub fn get(&self) -> u64 {
        self.value.load(Ordering::SeqCst)
    }
}
