mod http;
mod cache;
mod math;
mod macros;
mod counter;
mod json;
pub mod shapes;

use std::io;

pub fn add(a: i32, b: i32) -> i32 {
    a + b
}

pub fn broken() -> i32 {
    42
}

pub fn binary_search(haystack: &[i32], needle: i32) -> Option<usize> {
    let mut low = 0usize;
    let mut high = haystack.len();

    while low < high {
        let mid = low + (high - low) / 2;
        match haystack[mid].cmp(&needle) {
            std::cmp::Ordering::Less => low = mid + 1,
            std::cmp::Ordering::Greater => high = mid,
            std::cmp::Ordering::Equal => return Some(mid),
        }
    }

    None
}

pub async fn read_non_empty_trimmed_lines(path: &str) -> io::Result<Vec<String>> {
    let contents = tokio::fs::read_to_string(path).await?;
    Ok(contents
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| line.to_string())
        .collect())
}

pub struct FibIter {
    curr: u64,
    next: u64,
}

impl FibIter {
    pub fn new() -> Self {
        Self { curr: 0, next: 1 }
    }
}

impl Iterator for FibIter {
    type Item = u64;

    fn next(&mut self) -> Option<Self::Item> {
        let value = self.curr;
        self.curr = self.next;
        self.next = self.next.wrapping_add(value);
        Some(value)
    }
}

pub struct LinkedList<T> {
    head: Link<T>,
    len: usize,
}

type Link<T> = Option<Box<Node<T>>>;

struct Node<T> {
    elem: T,
    next: Link<T>,
}

impl<T> LinkedList<T> {
    pub fn new() -> Self {
        Self { head: None, len: 0 }
    }

    pub fn push_front(&mut self, elem: T) {
        let new_node = Box::new(Node {
            elem,
            next: self.head.take(),
        });
        self.head = Some(new_node);
        self.len += 1;
    }

    pub fn pop_front(&mut self) -> Option<T> {
        self.head.take().map(|node| {
            let node = *node;
            self.head = node.next;
            self.len -= 1;
            node.elem
        })
    }

    pub fn len(&self) -> usize {
        self.len
    }
}

impl<T> Default for LinkedList<T> {
    fn default() -> Self {
        Self::new()
    }
}
