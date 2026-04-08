//! Resolution precedence for secrets (items 43.8--43.11).
//!
//! A [`SecretResolver`] walks a configurable chain of [`SecretProvider`]
//! implementations (env -> file -> vault -> prompt) and returns the first hit.
//! Every resolved value carries its [`SecretSource`] and a millisecond
//! timestamp so callers can reason about freshness and provenance.

use std::sync::Arc;

type EnvResolver = dyn Fn(&str) -> Option<String> + Send + Sync;

/// Where a secret was resolved from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub enum SecretSource {
    /// Read from an environment variable.
    Environment,
    /// Read from a secrets file on disk.
    File,
    /// Read from a vault backend (`HashiCorp` Vault, 1Password, etc.).
    Vault,
    /// Prompted interactively from the user.
    Prompt,
    /// Source is not known (e.g. injected programmatically).
    Unknown,
}

impl SecretSource {
    /// Stable label for metrics / logs.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Environment => "environment",
            Self::File => "file",
            Self::Vault => "vault",
            Self::Prompt => "prompt",
            Self::Unknown => "unknown",
        }
    }
}

impl std::fmt::Display for SecretSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A resolved secret value with metadata about its origin and when it was
/// resolved.
#[derive(Clone, Debug)]
pub struct SecretValue {
    /// The raw secret value.
    value: String,
    /// Which backend supplied the value.
    pub source: SecretSource,
    /// Unix epoch milliseconds at which the value was resolved.
    pub resolved_at_ms: u64,
}

impl SecretValue {
    /// Create a new resolved secret.
    #[must_use]
    pub const fn new(value: String, source: SecretSource, resolved_at_ms: u64) -> Self {
        Self {
            value,
            source,
            resolved_at_ms,
        }
    }

    /// Access the underlying secret string.
    #[must_use]
    pub fn value(&self) -> &str {
        &self.value
    }
}

/// A pluggable secret source. Each backend (env, file, vault, prompt)
/// implements this trait so the [`SecretResolver`] can walk the chain.
pub trait SecretProvider: Send + Sync {
    /// Try to get a secret for the given namespace + key.
    /// Returns `None` if this backend does not have the requested secret.
    fn get(&self, namespace: &str, key: &str) -> Option<String>;

    /// Which source category this provider represents.
    fn source(&self) -> SecretSource;
}

/// Reads secrets from environment variables.
///
/// Given namespace `llm` and key `anthropic`, looks up
/// `{prefix}LLM_ANTHROPIC` (uppercased, joined with `_`).
pub struct EnvProvider {
    prefix: String,
    resolver: Arc<EnvResolver>,
}

impl std::fmt::Debug for EnvProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EnvProvider")
            .field("prefix", &self.prefix)
            .finish_non_exhaustive()
    }
}

impl EnvProvider {
    /// Create a provider that reads env vars with the given prefix.
    /// Default prefix is `"ROKO_SECRET_"`.
    #[must_use]
    pub fn new() -> Self {
        Self::with_prefix("ROKO_SECRET_")
    }

    /// Create a provider with a custom prefix.
    #[must_use]
    pub fn with_prefix(prefix: impl Into<String>) -> Self {
        Self {
            prefix: prefix.into(),
            resolver: Arc::new(|name| std::env::var(name).ok()),
        }
    }

    /// Replace the env-var lookup function (useful for tests).
    #[must_use]
    pub fn with_resolver(mut self, resolver: Arc<EnvResolver>) -> Self {
        self.resolver = resolver;
        self
    }
}

impl Default for EnvProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl SecretProvider for EnvProvider {
    fn get(&self, namespace: &str, key: &str) -> Option<String> {
        let var_name = format!(
            "{}{}_{}",
            self.prefix,
            namespace.to_ascii_uppercase(),
            key.to_ascii_uppercase()
        );
        (self.resolver)(&var_name)
    }

    fn source(&self) -> SecretSource {
        SecretSource::Environment
    }
}

/// Reads secrets from an in-memory map (useful for file-backed or test stores).
pub struct FileProvider {
    secrets: parking_lot::RwLock<std::collections::HashMap<(String, String), String>>,
}

impl std::fmt::Debug for FileProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileProvider").finish_non_exhaustive()
    }
}

impl FileProvider {
    /// Create an empty file provider.
    #[must_use]
    pub fn new() -> Self {
        Self {
            secrets: parking_lot::RwLock::new(std::collections::HashMap::new()),
        }
    }

    /// Insert a secret into the in-memory map.
    pub fn insert(&self, namespace: &str, key: &str, value: String) {
        self.secrets
            .write()
            .insert((namespace.to_string(), key.to_string()), value);
    }
}

impl Default for FileProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl SecretProvider for FileProvider {
    fn get(&self, namespace: &str, key: &str) -> Option<String> {
        self.secrets
            .read()
            .get(&(namespace.to_string(), key.to_string()))
            .cloned()
    }

    fn source(&self) -> SecretSource {
        SecretSource::File
    }
}

/// Configuration for the [`SecretResolver`].
#[derive(Clone, Debug)]
pub struct ResolverConfig {
    /// Prefix for environment variable lookups (default: `"ROKO_SECRET_"`).
    pub env_prefix: String,
    /// Optional path to a secrets file.
    pub file_path: Option<String>,
    /// Optional URL for a vault backend.
    pub vault_url: Option<String>,
}

impl Default for ResolverConfig {
    fn default() -> Self {
        Self {
            env_prefix: "ROKO_SECRET_".to_string(),
            file_path: None,
            vault_url: None,
        }
    }
}

/// Walks a precedence chain of [`SecretProvider`]s and returns the first match.
///
/// Default resolution order: env var > file > vault > prompt.
/// The chain is caller-configurable; providers are tried in insertion order.
pub struct SecretResolver {
    providers: Vec<Box<dyn SecretProvider>>,
    config: ResolverConfig,
}

impl std::fmt::Debug for SecretResolver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SecretResolver")
            .field("config", &self.config)
            .field("provider_count", &self.providers.len())
            .finish_non_exhaustive()
    }
}

impl SecretResolver {
    /// Build a resolver from a config. Callers add providers via
    /// [`SecretResolver::add_provider`] after construction.
    #[must_use]
    pub fn new(config: ResolverConfig) -> Self {
        Self {
            providers: Vec::new(),
            config,
        }
    }

    /// Build a resolver with an `EnvProvider` already wired in.
    #[must_use]
    pub fn with_env(config: ResolverConfig) -> Self {
        let env = EnvProvider::with_prefix(&config.env_prefix);
        let mut resolver = Self::new(config);
        resolver.add_provider(Box::new(env));
        resolver
    }

    /// Append a provider to the end of the resolution chain.
    pub fn add_provider(&mut self, provider: Box<dyn SecretProvider>) {
        self.providers.push(provider);
    }

    /// Walk the provider chain and return the first match.
    #[must_use]
    pub fn resolve(&self, namespace: &str, key: &str) -> Option<SecretValue> {
        let now_ms = now_millis();
        for provider in &self.providers {
            if let Some(value) = provider.get(namespace, key) {
                return Some(SecretValue::new(value, provider.source(), now_ms));
            }
        }
        None
    }

    /// Access the resolver's configuration.
    #[must_use]
    pub const fn config(&self) -> &ResolverConfig {
        &self.config
    }

    /// Number of providers in the chain.
    #[must_use]
    pub fn provider_count(&self) -> usize {
        self.providers.len()
    }
}

/// Current time in epoch milliseconds (uses `chrono`).
fn now_millis() -> u64 {
    #[allow(clippy::cast_sign_loss)]
    let ms = chrono::Utc::now().timestamp_millis() as u64;
    ms
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Arc;

    fn make_env_provider(map: HashMap<String, String>) -> EnvProvider {
        let resolver = Arc::new(move |name: &str| map.get(name).cloned());
        EnvProvider::with_prefix("ROKO_SECRET_").with_resolver(resolver)
    }

    #[test]
    fn resolve_from_env_provider() {
        let mut map = HashMap::new();
        map.insert("ROKO_SECRET_LLM_ANTHROPIC".into(), "sk-ant-123".into());
        let env = make_env_provider(map);
        let mut resolver = SecretResolver::new(ResolverConfig::default());
        resolver.add_provider(Box::new(env));
        let val = resolver.resolve("llm", "anthropic").unwrap();
        assert_eq!(val.value(), "sk-ant-123");
        assert_eq!(val.source, SecretSource::Environment);
    }

    #[test]
    fn resolve_from_file_provider() {
        let file = FileProvider::new();
        file.insert("rpc", "alchemy", "alch-key".into());
        let mut resolver = SecretResolver::new(ResolverConfig::default());
        resolver.add_provider(Box::new(file));
        let val = resolver.resolve("rpc", "alchemy").unwrap();
        assert_eq!(val.value(), "alch-key");
        assert_eq!(val.source, SecretSource::File);
    }

    #[test]
    fn resolve_returns_none_when_no_provider_has_it() {
        let resolver = SecretResolver::new(ResolverConfig::default());
        assert!(resolver.resolve("llm", "anthropic").is_none());
    }

    #[test]
    fn env_takes_precedence_over_file() {
        let mut map = HashMap::new();
        map.insert("ROKO_SECRET_LLM_ANTHROPIC".into(), "from-env".into());
        let env = make_env_provider(map);
        let file = FileProvider::new();
        file.insert("llm", "anthropic", "from-file".into());

        let mut resolver = SecretResolver::new(ResolverConfig::default());
        resolver.add_provider(Box::new(env));
        resolver.add_provider(Box::new(file));
        let val = resolver.resolve("llm", "anthropic").unwrap();
        assert_eq!(val.value(), "from-env");
        assert_eq!(val.source, SecretSource::Environment);
    }

    #[test]
    fn file_used_when_env_misses() {
        let env = make_env_provider(HashMap::new());
        let file = FileProvider::new();
        file.insert("llm", "openai", "from-file".into());

        let mut resolver = SecretResolver::new(ResolverConfig::default());
        resolver.add_provider(Box::new(env));
        resolver.add_provider(Box::new(file));
        let val = resolver.resolve("llm", "openai").unwrap();
        assert_eq!(val.value(), "from-file");
        assert_eq!(val.source, SecretSource::File);
    }

    #[test]
    fn with_env_wires_env_provider() {
        let resolver = SecretResolver::with_env(ResolverConfig::default());
        assert_eq!(resolver.provider_count(), 1);
    }

    #[test]
    fn resolver_config_default_prefix() {
        let cfg = ResolverConfig::default();
        assert_eq!(cfg.env_prefix, "ROKO_SECRET_");
        assert!(cfg.file_path.is_none());
        assert!(cfg.vault_url.is_none());
    }

    #[test]
    fn secret_value_carries_timestamp() {
        let file = FileProvider::new();
        file.insert("llm", "cohere", "ck-123".into());
        let mut resolver = SecretResolver::new(ResolverConfig::default());
        resolver.add_provider(Box::new(file));
        let val = resolver.resolve("llm", "cohere").unwrap();
        assert!(val.resolved_at_ms > 0);
    }

    #[test]
    fn secret_source_display() {
        assert_eq!(SecretSource::Environment.to_string(), "environment");
        assert_eq!(SecretSource::File.to_string(), "file");
        assert_eq!(SecretSource::Vault.to_string(), "vault");
        assert_eq!(SecretSource::Prompt.to_string(), "prompt");
        assert_eq!(SecretSource::Unknown.to_string(), "unknown");
    }

    #[test]
    fn secret_source_as_str_stable() {
        assert_eq!(SecretSource::Environment.as_str(), "environment");
        assert_eq!(SecretSource::File.as_str(), "file");
        assert_eq!(SecretSource::Vault.as_str(), "vault");
        assert_eq!(SecretSource::Prompt.as_str(), "prompt");
        assert_eq!(SecretSource::Unknown.as_str(), "unknown");
    }

    #[test]
    fn custom_env_prefix() {
        let mut map: HashMap<String, String> = HashMap::new();
        map.insert("MY_APP_RPC_INFURA".into(), "infura-key".into());
        let resolver_fn = Arc::new(move |name: &str| map.get(name).cloned());
        let env = EnvProvider::with_prefix("MY_APP_").with_resolver(resolver_fn);

        let mut resolver = SecretResolver::new(ResolverConfig {
            env_prefix: "MY_APP_".into(),
            ..Default::default()
        });
        resolver.add_provider(Box::new(env));
        let val = resolver.resolve("rpc", "infura").unwrap();
        assert_eq!(val.value(), "infura-key");
    }

    #[test]
    fn multiple_namespaces_independent() {
        let file = FileProvider::new();
        file.insert("llm", "anthropic", "ant-key".into());
        file.insert("rpc", "alchemy", "alch-key".into());
        let mut resolver = SecretResolver::new(ResolverConfig::default());
        resolver.add_provider(Box::new(file));

        assert_eq!(
            resolver.resolve("llm", "anthropic").unwrap().value(),
            "ant-key"
        );
        assert_eq!(
            resolver.resolve("rpc", "alchemy").unwrap().value(),
            "alch-key"
        );
        assert!(resolver.resolve("llm", "openai").is_none());
    }
}
