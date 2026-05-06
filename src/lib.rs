mod http;
mod cache;
mod math;
mod macros;
mod counter;
mod json;
mod hashmap;
mod stack;
pub mod shapes;

use std::io;
use unicode_segmentation::UnicodeSegmentation;

pub fn add(a: i32, b: i32) -> i32 {
    a + b
}

pub fn broken() -> i32 {
    42
}

pub fn reverse_string(s: &str) -> String {
    UnicodeSegmentation::graphemes(s, true).rev().collect()
}

pub fn is_palindrome(s: &str) -> bool {
    let filtered: String = s
        .chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect();

    filtered.chars().eq(filtered.chars().rev())
}

pub fn fib(n: u64) -> u64 {
    if n == 0 {
        return 0;
    }

    let mut prev = 0u64;
    let mut curr = 1u64;

    for _ in 1..n {
        let next = prev + curr;
        prev = curr;
        curr = next;
    }

    curr
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

pub fn rotate_left<T: Clone>(slice: &mut [T], k: usize) {
    let len = slice.len();
    if len == 0 {
        return;
    }

    let k = k % len;
    if k == 0 {
        return;
    }

    slice.rotate_left(k);
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

pub use stack::Stack;

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

#[cfg(test)]
mod tests {
    use super::{fib, is_palindrome, reverse_string, rotate_left};

    #[test]
    fn fib_zero_is_zero() {
        assert_eq!(fib(0), 0);
    }

    #[test]
    fn fib_one_is_one() {
        assert_eq!(fib(1), 1);
    }

    #[test]
    fn fib_ten_is_fifty_five() {
        assert_eq!(fib(10), 55);
    }

    #[test]
    fn reverse_string_ascii() {
        assert_eq!(reverse_string("stressed"), "desserts");
    }

    #[test]
    fn reverse_string_unicode_graphemes() {
        assert_eq!(reverse_string("a🇺🇳b👨‍👩‍👧‍👦"), "👨‍👩‍👧‍👦b🇺🇳a");
    }

    #[test]
    fn palindrome_ignores_case_and_punctuation() {
        assert!(is_palindrome("A man, a plan, a canal: Panama"));
    }

    #[test]
    fn palindrome_handles_empty_and_single_character_strings() {
        assert!(is_palindrome(""));
        assert!(is_palindrome("x"));
    }

    #[test]
    fn palindrome_rejects_non_palindromes() {
        assert!(!is_palindrome("hello"));
    }

    #[test]
    fn rotate_left_by_zero_keeps_slice_unchanged() {
        let mut values = vec![1, 2, 3, 4];
        rotate_left(&mut values, 0);
        assert_eq!(values, vec![1, 2, 3, 4]);
    }

    #[test]
    fn rotate_left_by_less_than_len() {
        let mut values = vec![1, 2, 3, 4, 5];
        rotate_left(&mut values, 2);
        assert_eq!(values, vec![3, 4, 5, 1, 2]);
    }

    #[test]
    fn rotate_left_handles_k_greater_than_len() {
        let mut values = vec![1, 2, 3, 4, 5];
        rotate_left(&mut values, 7);
        assert_eq!(values, vec![3, 4, 5, 1, 2]);
    }

    #[test]
    fn rotate_left_handles_empty_slice() {
        let mut values: Vec<i32> = vec![];
        rotate_left(&mut values, 3);
        assert!(values.is_empty());
    }
}
