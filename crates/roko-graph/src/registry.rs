//! `CellRegistry` — maps cell type names to factory functions that produce `Cell` instances.

use std::collections::HashMap;

use roko_core::Cell;

use crate::types::GraphError;

/// A factory function that takes a TOML config and produces a boxed Cell.
pub type CellFactory = Box<dyn Fn(toml::Value) -> Box<dyn Cell> + Send + Sync>;

/// Registry that maps cell type name strings to factory functions.
///
/// When the graph engine encounters a node with `cell_type = "gate.compile"`,
/// it looks up "gate.compile" in this registry to obtain a factory, then calls
/// it with the node's config to instantiate the cell.
pub struct CellRegistry {
    factories: HashMap<String, CellFactory>,
}

impl CellRegistry {
    /// Create a new empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            factories: HashMap::new(),
        }
    }

    /// Register a factory function for a cell type name.
    ///
    /// If a factory was already registered for this name, it is replaced.
    pub fn register<F>(&mut self, cell_type: &str, factory: F)
    where
        F: Fn(toml::Value) -> Box<dyn Cell> + Send + Sync + 'static,
    {
        self.factories
            .insert(cell_type.to_string(), Box::new(factory));
    }

    /// Look up a factory by cell type name and instantiate a Cell with the given config.
    ///
    /// # Errors
    /// Returns `GraphError::UnknownCellType` if no factory is registered for the name.
    pub fn create(&self, cell_type: &str, config: toml::Value) -> Result<Box<dyn Cell>, GraphError> {
        let factory = self
            .factories
            .get(cell_type)
            .ok_or_else(|| GraphError::UnknownCellType(cell_type.to_string()))?;
        Ok(factory(config))
    }

    /// Check if a cell type is registered.
    #[must_use]
    pub fn contains(&self, cell_type: &str) -> bool {
        self.factories.contains_key(cell_type)
    }

    /// Return the number of registered cell types.
    #[must_use]
    pub fn len(&self) -> usize {
        self.factories.len()
    }

    /// Check if the registry is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.factories.is_empty()
    }

    /// Return an iterator over registered cell type names.
    pub fn cell_types(&self) -> impl Iterator<Item = &str> {
        self.factories.keys().map(String::as_str)
    }
}

impl Default for CellRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for CellRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CellRegistry")
            .field("registered_types", &self.factories.keys().collect::<Vec<_>>())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use roko_core::cell::CellVersion;

    use super::*;

    /// A trivial Cell implementation for testing the registry.
    struct NoopCell {
        id: String,
    }

    impl Cell for NoopCell {
        fn cell_id(&self) -> &str {
            &self.id
        }
        fn cell_name(&self) -> &str {
            "noop"
        }
        fn cell_version(&self) -> CellVersion {
            (0, 1, 0)
        }
        fn protocols(&self) -> &[&str] {
            &[]
        }
        fn estimated_cost(&self) -> Option<f64> {
            None
        }
        fn estimated_duration(&self) -> Option<Duration> {
            None
        }
    }

    #[test]
    fn register_and_create() {
        let mut registry = CellRegistry::new();
        registry.register("noop", |_config| {
            Box::new(NoopCell {
                id: "noop-1".to_string(),
            })
        });

        assert!(registry.contains("noop"));
        assert_eq!(registry.len(), 1);

        let cell = registry
            .create("noop", toml::Value::Table(toml::map::Map::new()))
            .unwrap();
        assert_eq!(cell.cell_id(), "noop-1");
        assert_eq!(cell.cell_name(), "noop");
    }

    #[test]
    fn unknown_cell_type_errors() {
        let registry = CellRegistry::new();
        let result = registry.create("nonexistent", toml::Value::Table(toml::map::Map::new()));
        assert!(matches!(
            result,
            Err(GraphError::UnknownCellType(ref t)) if t == "nonexistent"
        ));
    }

    #[test]
    fn empty_registry() {
        let registry = CellRegistry::new();
        assert!(registry.is_empty());
        assert_eq!(registry.len(), 0);
        assert!(!registry.contains("anything"));
    }

    #[test]
    fn register_replaces_existing() {
        let mut registry = CellRegistry::new();
        registry.register("test", |_| {
            Box::new(NoopCell {
                id: "first".to_string(),
            })
        });
        registry.register("test", |_| {
            Box::new(NoopCell {
                id: "second".to_string(),
            })
        });

        let cell = registry
            .create("test", toml::Value::Table(toml::map::Map::new()))
            .unwrap();
        assert_eq!(cell.cell_id(), "second");
    }

    #[test]
    fn cell_types_iterator() {
        let mut registry = CellRegistry::new();
        registry.register("alpha", |_| {
            Box::new(NoopCell {
                id: "a".to_string(),
            })
        });
        registry.register("beta", |_| {
            Box::new(NoopCell {
                id: "b".to_string(),
            })
        });

        let mut types: Vec<&str> = registry.cell_types().collect();
        types.sort();
        assert_eq!(types, vec!["alpha", "beta"]);
    }
}
