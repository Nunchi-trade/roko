use std::collections::{HashSet, VecDeque};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Graph {
    adjacency_list: Vec<Vec<usize>>,
}

impl Graph {
    pub fn new(nodes: usize) -> Self {
        Self {
            adjacency_list: vec![Vec::new(); nodes],
        }
    }

    pub fn add_edge(&mut self, from: usize, to: usize) {
        self.adjacency_list[from].push(to);
    }

    pub fn bfs(&self, start: usize) -> Vec<usize> {
        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();
        let mut order = Vec::new();

        visited.insert(start);
        queue.push_back(start);

        while let Some(node) = queue.pop_front() {
            order.push(node);
            if let Some(neighbors) = self.adjacency_list.get(node) {
                for &neighbor in neighbors {
                    if visited.insert(neighbor) {
                        queue.push_back(neighbor);
                    }
                }
            }
        }

        order
    }
}

#[cfg(test)]
mod tests {
    use super::Graph;

    #[test]
    fn bfs_traverses_six_node_graph() {
        let mut graph = Graph::new(6);
        graph.add_edge(0, 1);
        graph.add_edge(0, 2);
        graph.add_edge(1, 3);
        graph.add_edge(1, 4);
        graph.add_edge(2, 5);

        assert_eq!(graph.bfs(0), vec![0, 1, 2, 3, 4, 5]);
    }

    #[test]
    fn bfs_handles_disconnected_nodes() {
        let mut graph = Graph::new(6);
        graph.add_edge(0, 1);
        graph.add_edge(1, 2);
        graph.add_edge(3, 4);

        assert_eq!(graph.bfs(3), vec![3, 4]);
    }
}
