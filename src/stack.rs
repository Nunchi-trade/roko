#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Stack<T> {
    items: Vec<T>,
}

impl<T> Stack<T> {
    pub fn new() -> Self {
        Self { items: Vec::new() }
    }

    pub fn push(&mut self, value: T) {
        self.items.push(value);
    }

    pub fn pop(&mut self) -> Option<T> {
        self.items.pop()
    }

    pub fn peek(&self) -> Option<&T> {
        self.items.last()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }
}

#[cfg(test)]
mod tests {
    use super::Stack;

    #[test]
    fn new_stack_is_empty() {
        let stack: Stack<i32> = Stack::new();
        assert!(stack.is_empty());
        assert_eq!(stack.len(), 0);
        assert_eq!(stack.peek(), None);
        assert_eq!(stack.clone(), Stack::default());
    }

    #[test]
    fn push_updates_len_and_peek() {
        let mut stack = Stack::new();
        stack.push(10);
        stack.push(20);
        assert!(!stack.is_empty());
        assert_eq!(stack.len(), 2);
        assert_eq!(stack.peek(), Some(&20));
    }

    #[test]
    fn pop_returns_elements_in_lifo_order() {
        let mut stack = Stack::new();
        stack.push(1);
        stack.push(2);
        stack.push(3);

        assert_eq!(stack.pop(), Some(3));
        assert_eq!(stack.pop(), Some(2));
        assert_eq!(stack.pop(), Some(1));
        assert_eq!(stack.pop(), None);
        assert!(stack.is_empty());
        assert_eq!(stack.len(), 0);
    }

    #[test]
    fn peek_does_not_remove_item() {
        let mut stack = Stack::new();
        stack.push("top");

        assert_eq!(stack.peek(), Some(&"top"));
        assert_eq!(stack.len(), 1);
        assert_eq!(stack.pop(), Some("top"));
        assert!(stack.is_empty());
    }

    #[test]
    fn handles_non_copy_types() {
        let mut stack = Stack::new();
        stack.push(String::from("hello"));
        stack.push(String::from("world"));

        assert_eq!(stack.peek().map(String::as_str), Some("world"));
        assert_eq!(stack.pop(), Some(String::from("world")));
        assert_eq!(stack.pop(), Some(String::from("hello")));
    }
}
