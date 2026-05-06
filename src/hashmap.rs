use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

#[derive(Debug, Clone)]
pub struct HashMap {
    buckets: Vec<Entry>,
    len: usize,
}

#[derive(Debug, Clone)]
enum Entry {
    Empty,
    Deleted,
    Occupied(String, i64),
}

impl HashMap {
    pub fn new() -> Self {
        Self::with_capacity(16)
    }

    pub fn with_capacity(capacity: usize) -> Self {
        let capacity = capacity.max(1).next_power_of_two();
        Self {
            buckets: vec![Entry::Empty; capacity],
            len: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn insert(&mut self, key: String, value: i64) -> Option<i64> {
        if (self.len + 1) * 100 / self.buckets.len() >= 70 {
            self.resize();
        }

        self.insert_internal(key, value)
    }

    pub fn get(&self, key: &str) -> Option<i64> {
        let mut index = self.index_for(key);
        for _ in 0..self.buckets.len() {
            match &self.buckets[index] {
                Entry::Empty => return None,
                Entry::Deleted => {}
                Entry::Occupied(k, v) if k == key => return Some(*v),
                Entry::Occupied(_, _) => {}
            }
            index = (index + 1) % self.buckets.len();
        }
        None
    }

    pub fn remove(&mut self, key: &str) -> Option<i64> {
        let mut index = self.index_for(key);
        for _ in 0..self.buckets.len() {
            match &self.buckets[index] {
                Entry::Empty => return None,
                Entry::Deleted => {}
                Entry::Occupied(k, _) if k == key => {
                    let old = std::mem::replace(&mut self.buckets[index], Entry::Deleted);
                    if let Entry::Occupied(_, v) = old {
                        self.len -= 1;
                        return Some(v);
                    }
                }
                Entry::Occupied(_, _) => {}
            }
            index = (index + 1) % self.buckets.len();
        }
        None
    }

    fn insert_internal(&mut self, key: String, value: i64) -> Option<i64> {
        let mut index = self.index_for(&key);
        let mut first_deleted = None;

        for _ in 0..self.buckets.len() {
            match &mut self.buckets[index] {
                Entry::Empty => {
                    let target = first_deleted.unwrap_or(index);
                    self.buckets[target] = Entry::Occupied(key, value);
                    self.len += 1;
                    return None;
                }
                Entry::Deleted => {
                    if first_deleted.is_none() {
                        first_deleted = Some(index);
                    }
                }
                Entry::Occupied(k, v) if k == &key => {
                    return Some(std::mem::replace(v, value));
                }
                Entry::Occupied(_, _) => {}
            }
            index = (index + 1) % self.buckets.len();
        }

        if let Some(target) = first_deleted {
            self.buckets[target] = Entry::Occupied(key, value);
            self.len += 1;
            return None;
        }

        self.resize();
        self.insert_internal(key, value)
    }

    fn resize(&mut self) {
        let mut new_map = HashMap::with_capacity(self.buckets.len() * 2);
        for entry in self.buckets.drain(..) {
            if let Entry::Occupied(k, v) = entry {
                new_map.insert_internal(k, v);
            }
        }
        *self = new_map;
    }

    fn index_for<T: Hash>(&self, key: T) -> usize {
        let mut hasher = DefaultHasher::new();
        key.hash(&mut hasher);
        (hasher.finish() as usize) % self.buckets.len()
    }
}

impl Default for HashMap {
    fn default() -> Self {
        Self::new()
    }
}
