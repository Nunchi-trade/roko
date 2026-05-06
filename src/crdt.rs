use std::collections::HashMap;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GCounter {
    node_id: String,
    counts: HashMap<String, u64>,
}

impl GCounter {
    pub fn new(node_id: impl Into<String>) -> Self {
        Self {
            node_id: node_id.into(),
            counts: HashMap::new(),
        }
    }

    pub fn increment(&mut self) {
        *self.counts.entry(self.node_id.clone()).or_insert(0) += 1;
    }

    pub fn value(&self) -> u64 {
        self.counts.values().copied().sum()
    }

    pub fn merge(&mut self, remote: &Self) {
        for (node_id, remote_count) in &remote.counts {
            let local_count = self.counts.entry(node_id.clone()).or_insert(0);
            *local_count = (*local_count).max(*remote_count);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::GCounter;

    #[test]
    fn increments_are_local_to_each_node() {
        let mut a = GCounter::new("a");
        let mut b = GCounter::new("b");

        a.increment();
        a.increment();
        b.increment();

        assert_eq!(a.value(), 2);
        assert_eq!(b.value(), 1);
    }

    #[test]
    fn merge_uses_element_wise_max() {
        let mut a = GCounter::new("a");
        let mut b = GCounter::new("b");

        a.increment();
        a.increment();
        b.increment();
        b.increment();
        b.increment();

        a.merge(&b);
        b.merge(&a);

        assert_eq!(a.value(), 5);
        assert_eq!(b.value(), 5);
    }

    #[test]
    fn three_nodes_converge_after_merges() {
        let mut a = GCounter::new("a");
        let mut b = GCounter::new("b");
        let mut c = GCounter::new("c");

        a.increment();
        a.increment();
        b.increment();
        c.increment();
        c.increment();
        c.increment();

        a.merge(&b);
        a.merge(&c);
        b.merge(&a);
        c.merge(&b);

        assert_eq!(a.value(), 6);
        assert_eq!(b.value(), 6);
        assert_eq!(c.value(), 6);
    }
}
