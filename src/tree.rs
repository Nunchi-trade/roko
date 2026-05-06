pub enum Tree<T> {
    Leaf,
    Node(T, Box<Tree<T>>, Box<Tree<T>>),
}

impl<T> Tree<T> {
    pub fn inorder(&self) -> Vec<&T> {
        match self {
            Tree::Leaf => Vec::new(),
            Tree::Node(value, left, right) => {
                let mut result = left.inorder();
                result.push(value);
                result.extend(right.inorder());
                result
            }
        }
    }

    pub fn preorder(&self) -> Vec<&T> {
        match self {
            Tree::Leaf => Vec::new(),
            Tree::Node(value, left, right) => {
                let mut result = Vec::new();
                result.push(value);
                result.extend(left.preorder());
                result.extend(right.preorder());
                result
            }
        }
    }

    pub fn postorder(&self) -> Vec<&T> {
        match self {
            Tree::Leaf => Vec::new(),
            Tree::Node(value, left, right) => {
                let mut result = left.postorder();
                result.extend(right.postorder());
                result.push(value);
                result
            }
        }
    }
}
