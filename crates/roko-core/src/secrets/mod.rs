//! Secret management: trait + env + file backends.
//!
//! Secrets are namespaced (`llm.anthropic`, `rpc.alchemy`, etc.) and resolved
//! through a pluggable [`SecretStore`]. Built-in backends:
//! - [`EnvVarStore`]: reads `ROKO_SECRET_<ns_upper_with_underscores>`
//! - [`FileStore`]: reads `.roko/secrets.toml` (0600 perms required on unix)
//!
//! Additional backends (`OnePasswordStore`, `VaultStore`, `AwsSecretsManagerStore`)
//! are later waves (§43.4–43.6) and live behind opt-in features.

pub mod audit;
pub mod env;
pub mod file;
pub mod namespace;
pub mod resolve;

pub use audit::{AuditAction, AuditEntry, SecretAuditLog};
pub use env::EnvVarStore;
pub use file::FileStore;
pub use namespace::{Namespace, WellKnownNamespaces};
pub use resolve::{
    EnvProvider, FileProvider, ResolverConfig, SecretProvider, SecretResolver, SecretSource,
    SecretValue,
};

use crate::error::Result;

/// Abstraction over secret backends: env, file, vault, 1Password, AWS.
///
/// Implementations must be thread-safe (`Send + Sync`); a single store may be
/// shared across agents, gates, and backends via `Arc<dyn SecretStore>`.
pub trait SecretStore: Send + Sync {
    /// Retrieve the secret for a namespace. Returns `Ok(None)` if not set.
    fn get(&self, ns: &Namespace) -> Result<Option<String>>;

    /// Store a secret. Read-only backends (e.g. [`EnvVarStore`]) return an
    /// error.
    fn set(&self, ns: &Namespace, value: String) -> Result<()>;

    /// Rotate a secret. Default impl is equivalent to [`SecretStore::set`];
    /// backends that track versions override this.
    fn rotate(&self, ns: &Namespace, new_value: String) -> Result<()> {
        self.set(ns, new_value)
    }

    /// Human-readable backend name (`"env"`, `"file"`, `"vault"`, ...).
    fn name(&self) -> &'static str;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn secret_store_is_object_safe() {
        // Compile-time check: trait is usable as `dyn SecretStore`.
        let _: Arc<dyn SecretStore> = Arc::new(EnvVarStore::new());
    }

    #[test]
    fn rotate_delegates_to_set_by_default() {
        // FileStore uses default rotate impl; exercise it via the trait.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secrets.toml");
        let store: Arc<dyn SecretStore> = Arc::new(FileStore::open(&path).unwrap());
        let ns = Namespace::new("llm", "anthropic");
        store.set(&ns, "v1".into()).unwrap();
        store.rotate(&ns, "v2".into()).unwrap();
        assert_eq!(store.get(&ns).unwrap(), Some("v2".into()));
    }
}
