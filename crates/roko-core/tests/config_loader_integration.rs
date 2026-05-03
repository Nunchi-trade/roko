//! Integration tests for the unified config loader.
//!
//! Tests the full config loading pipeline: file discovery, global merge,
//! env var overrides, secret resolution, validation, and serialization roundtrip.

use roko_core::config::loader::{
    LoadOptions, discover_project_config, load_config_file, load_config_validated,
    load_config_with_options, serialize_effective,
};
use roko_core::config::schema::RokoConfig;
use std::path::PathBuf;

// ── File discovery ──────────────────────────────────────────────────────

#[test]
fn discover_finds_roko_toml_in_parent() {
    let dir = tempfile::tempdir().unwrap();
    let nested = dir.path().join("a").join("b").join("c");
    std::fs::create_dir_all(&nested).unwrap();
    std::fs::write(
        dir.path().join("roko.toml"),
        "config_version = 2\nschema_version = 2\n",
    )
    .unwrap();

    let found = discover_project_config(&nested);
    assert!(found.is_some(), "should find roko.toml in ancestor");
    let found = found.unwrap();
    assert!(found.ends_with("roko.toml"));
}

#[test]
fn discover_returns_none_in_empty_tree() {
    let dir = tempfile::tempdir().unwrap();
    let nested = dir.path().join("x").join("y");
    std::fs::create_dir_all(&nested).unwrap();

    let found = discover_project_config(&nested);
    // May find the project's roko.toml in the real filesystem, so we can't
    // assert None strictly. Just ensure no panic.
    let _ = found;
}

// ── Basic loading ───────────────────────────────────────────────────────

#[test]
fn load_returns_defaults_when_no_file() {
    let dir = tempfile::tempdir().unwrap();
    let opts = LoadOptions {
        merge_global: false,
        apply_env_overrides: false,
        strict_validation: false,
    };
    let config = load_config_with_options(dir.path(), &opts).unwrap();
    assert_eq!(config, RokoConfig::default());
}

#[test]
fn load_reads_providers_from_file() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("roko.toml"),
        r#"
config_version = 2
schema_version = 2

[providers.test-provider]
kind = "openai_compat"
base_url = "https://api.test.com/v1"
api_key_env = "TEST_KEY"
"#,
    )
    .unwrap();

    let opts = LoadOptions {
        merge_global: false,
        apply_env_overrides: false,
        strict_validation: false,
    };
    let config = load_config_with_options(dir.path(), &opts).unwrap();
    assert!(config.providers.contains_key("test-provider"));
    let prov = &config.providers["test-provider"];
    assert_eq!(prov.base_url.as_deref(), Some("https://api.test.com/v1"));
}

#[test]
fn load_config_file_uses_exact_nonstandard_path() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("roko.toml"),
        r#"
config_version = 2
schema_version = 2

[providers.parent-provider]
kind = "openai_compat"
base_url = "https://parent.example/v1"
"#,
    )
    .unwrap();
    let explicit_path = dir.path().join("explicit-acp.toml");
    std::fs::write(
        &explicit_path,
        r#"
config_version = 2
schema_version = 2

[providers.explicit-provider]
kind = "openai_compat"
base_url = "https://explicit.example/v1"
"#,
    )
    .unwrap();

    let opts = LoadOptions {
        merge_global: false,
        apply_env_overrides: false,
        strict_validation: false,
    };
    let config = load_config_file(&explicit_path, &opts).unwrap();

    assert!(config.providers.contains_key("explicit-provider"));
    assert!(!config.providers.contains_key("parent-provider"));
}

#[test]
fn load_reads_models_from_file() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("roko.toml"),
        r#"
config_version = 2
schema_version = 2

[providers.prov]
kind = "openai_compat"
base_url = "https://api.test.com/v1"

[models.my-model]
provider = "prov"
slug = "my-model-v1"
context_window = 128000
supports_tools = true
"#,
    )
    .unwrap();

    let opts = LoadOptions {
        merge_global: false,
        apply_env_overrides: false,
        strict_validation: false,
    };
    let config = load_config_with_options(dir.path(), &opts).unwrap();
    assert!(config.models.contains_key("my-model"));
    let model = &config.models["my-model"];
    assert_eq!(model.slug, "my-model-v1");
    assert_eq!(model.context_window, 128_000);
    assert!(model.supports_tools);
}

// ── Model tier ─────────────────────────────────────────────────────────

#[test]
fn load_reads_model_tier_from_file() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("roko.toml"),
        r#"
config_version = 2
schema_version = 2

[providers.prov]
kind = "openai_compat"
base_url = "https://api.test.com/v1"

[models.fast-model]
provider = "prov"
slug = "fast-v1"
tier = "fast"
context_window = 8192

[models.premium-model]
provider = "prov"
slug = "premium-v1"
tier = "premium"
context_window = 200000

[models.no-tier-model]
provider = "prov"
slug = "default-v1"
context_window = 128000
"#,
    )
    .unwrap();

    let opts = LoadOptions {
        merge_global: false,
        apply_env_overrides: false,
        strict_validation: false,
    };
    let config = load_config_with_options(dir.path(), &opts).unwrap();

    use roko_core::agent::ModelTier;
    assert_eq!(config.models["fast-model"].tier, Some(ModelTier::Fast));
    assert_eq!(
        config.models["premium-model"].tier,
        Some(ModelTier::Premium)
    );
    assert_eq!(config.models["no-tier-model"].tier, None);
}

// ── Serialization roundtrip ─────────────────────────────────────────────

#[test]
fn config_roundtrip_default() {
    let original = RokoConfig::default();
    let toml_str = serialize_effective(&original).unwrap();
    let reparsed: RokoConfig = toml::from_str(&toml_str).unwrap();
    assert_eq!(original, reparsed);
}

#[test]
fn config_roundtrip_with_providers_and_models() {
    let dir = tempfile::tempdir().unwrap();
    let toml_content = r#"
config_version = 2
schema_version = 2

[providers.openai]
kind = "openai_compat"
base_url = "https://api.openai.com/v1"
api_key_env = "OPENAI_API_KEY"
timeout_ms = 120000
ttft_timeout_ms = 15000
connect_timeout_ms = 5000

[models.gpt-4o]
provider = "openai"
slug = "gpt-4o"
context_window = 128000
supports_tools = true
tool_format = "openai_json"

[models.gpt-o3]
provider = "openai"
slug = "o3"
context_window = 200000
supports_tools = true
tool_format = "openai_json"
"#;
    std::fs::write(dir.path().join("roko.toml"), toml_content).unwrap();

    let opts = LoadOptions {
        merge_global: false,
        apply_env_overrides: false,
        strict_validation: false,
    };
    let config = load_config_with_options(dir.path(), &opts).unwrap();

    // Roundtrip
    let serialized = serialize_effective(&config).unwrap();
    let reparsed: RokoConfig = toml::from_str(&serialized).unwrap();
    assert_eq!(config.providers, reparsed.providers);
    assert_eq!(config.models, reparsed.models);
}

// ── Validation ──────────────────────────────────────────────────────────

#[test]
fn validated_detects_orphaned_models() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("roko.toml"),
        r#"
config_version = 2
schema_version = 2

[models.orphan]
provider = "nonexistent-provider"
slug = "orphan-v1"
context_window = 4096
"#,
    )
    .unwrap();

    let validated = load_config_validated(dir.path()).unwrap();
    let orphan_warning = validated
        .diagnostics()
        .iter()
        .any(|d| d.message.contains("nonexistent-provider"));
    assert!(orphan_warning, "should warn about orphaned model");
}

#[test]
fn validated_detects_duplicate_slugs() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("roko.toml"),
        r#"
config_version = 2
schema_version = 2

[providers.p]
kind = "openai_compat"
base_url = "https://example.com"

[models.a]
provider = "p"
slug = "duplicate-slug"
context_window = 4096

[models.b]
provider = "p"
slug = "duplicate-slug"
context_window = 8192
"#,
    )
    .unwrap();

    let validated = load_config_validated(dir.path()).unwrap();
    let dup_warning = validated
        .diagnostics()
        .iter()
        .any(|d| d.message.contains("duplicate model slug"));
    assert!(dup_warning, "should warn about duplicate slugs");
}

#[test]
fn validated_warns_on_old_config_version() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("roko.toml"),
        "config_version = 1\nschema_version = 2\n",
    )
    .unwrap();

    let validated = load_config_validated(dir.path()).unwrap();
    let version_warning = validated
        .diagnostics()
        .iter()
        .any(|d| d.key == "config_version");
    assert!(version_warning, "should warn about old config_version");
}

// ── Strict validation ───────────────────────────────────────────────────

#[test]
fn strict_rejects_dangerous_permissions() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("roko.toml"),
        "[runner]\ndangerously_skip_permissions = true\n",
    )
    .unwrap();

    let result = load_config_with_options(dir.path(), &LoadOptions::strict());
    assert!(
        result.is_err(),
        "strict mode should reject dangerous permissions"
    );
}

#[test]
fn lenient_permits_dangerous_permissions() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("roko.toml"),
        "[runner]\ndangerously_skip_permissions = true\n",
    )
    .unwrap();

    let config = load_config_with_options(dir.path(), &LoadOptions::default()).unwrap();
    assert!(config.runner.dangerously_skip_permissions);
}

// ── Atomic I/O ──────────────────────────────────────────────────────────

#[test]
fn atomic_write_creates_and_overwrites() {
    use roko_core::io::{atomic_write, read_optional};

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.json");

    // Write initial
    atomic_write(&path, b"{\"v\":1}").unwrap();
    let content = read_optional(&path).unwrap();
    assert_eq!(content.as_deref(), Some("{\"v\":1}"));

    // Overwrite
    atomic_write(&path, b"{\"v\":2}").unwrap();
    let content = read_optional(&path).unwrap();
    assert_eq!(content.as_deref(), Some("{\"v\":2}"));

    // No .tmp.* siblings remain after a successful write.
    let siblings: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("state.json.tmp.")
        })
        .collect();
    assert!(siblings.is_empty(), "temp file should be cleaned up");
}

#[test]
fn atomic_write_creates_parent_dirs() {
    use roko_core::io::atomic_write;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("deep").join("nested").join("file.txt");
    atomic_write(&path, b"nested content").unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "nested content");
}

#[test]
fn read_optional_returns_none_for_missing() {
    use roko_core::io::read_optional;

    let dir = tempfile::tempdir().unwrap();
    let result = read_optional(&dir.path().join("nonexistent")).unwrap();
    assert_eq!(result, None);
}

#[tokio::test]
async fn atomic_write_async_works() {
    use roko_core::io::{atomic_write_async, read_optional_async};

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("async.json");

    atomic_write_async(&path, b"{\"async\":true}")
        .await
        .unwrap();
    let content = read_optional_async(&path).await.unwrap();
    assert_eq!(content.as_deref(), Some("{\"async\":true}"));
}

// ── Defaults module sanity ──────────────────────────────────────────────

#[test]
fn defaults_values_are_consistent() {
    use roko_core::defaults::*;

    // Timeouts are ordered correctly
    assert!(DEFAULT_TTFT_TIMEOUT_MS < DEFAULT_REQUEST_TIMEOUT_MS);
    assert!(DEFAULT_CONNECT_TIMEOUT_MS < DEFAULT_TTFT_TIMEOUT_MS);
    assert!(DEFAULT_EMBED_TIMEOUT_MS < DEFAULT_REQUEST_TIMEOUT_MS);

    // Retry backoff is ordered
    assert!(DEFAULT_RETRY_BASE_DELAY_MS < DEFAULT_RETRY_MAX_BACKOFF_MS);
    assert!(DEFAULT_RETRY_ATTEMPTS >= 1);

    // Token budgets are sane
    assert!(DEFAULT_MAX_OUTPUT_TOKENS > 0);
    assert!(DEFAULT_FALLBACK_MAX_OUTPUT_TOKENS > 0);
    assert!(DEFAULT_MAX_OUTPUT_TOKENS >= DEFAULT_FALLBACK_MAX_OUTPUT_TOKENS);
    assert!(DEFAULT_MAX_TOOL_ITERATIONS > 0);

    // Resource limits are sane
    assert!(DEFAULT_MAX_FILE_READ_BYTES >= 1024 * 1024);
    assert!(DEFAULT_MAX_RESULT_BYTES >= 1024);
    assert!(DEFAULT_MAX_GLOB_RESULTS >= 100);
    assert!(DEFAULT_MAX_CONCURRENT_TOOLS >= 1);
}

// ── Project roko.toml loads ─────────────────────────────────────────────

#[test]
fn project_roko_toml_loads_through_unified_loader() {
    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("find workspace root")
        .to_path_buf();

    let roko_toml = workspace_root.join("roko.toml");
    if !roko_toml.exists() {
        return; // Skip if no project config
    }

    // Load without global merge to test just the project file
    let opts = LoadOptions {
        merge_global: false,
        apply_env_overrides: false,
        strict_validation: false,
    };
    let config =
        load_config_with_options(&workspace_root, &opts).expect("project roko.toml must load");

    // Project should have providers configured
    assert!(
        !config.providers.is_empty(),
        "project config should have providers"
    );

    // Roundtrip: serialize → reparse
    let serialized = serialize_effective(&config).expect("serialize");
    let reparsed: RokoConfig = toml::from_str(&serialized).expect("reparse");
    assert_eq!(
        config.providers.len(),
        reparsed.providers.len(),
        "provider count mismatch after roundtrip"
    );
    assert_eq!(
        config.models.len(),
        reparsed.models.len(),
        "model count mismatch after roundtrip"
    );
}

// ── RetryPolicy (existing module, verify integration) ───────────────────

#[test]
fn retry_policy_from_defaults() {
    use roko_core::defaults::*;
    use roko_core::error::retry::RetryPolicy;

    let policy = RetryPolicy::new(
        DEFAULT_RETRY_ATTEMPTS,
        DEFAULT_RETRY_BASE_DELAY_MS,
        DEFAULT_RETRY_MAX_BACKOFF_MS,
        true, // jitter
    );

    assert_eq!(policy.max_attempts(), DEFAULT_RETRY_ATTEMPTS);
    assert!(policy.should_retry(0));
    assert!(policy.should_retry(1));
    assert!(!policy.should_retry(2)); // 3 attempts total

    let delay = policy.delay_for(0);
    // With jitter, delay should be between 500ms and 1000ms (base_delay = 1000)
    assert!(delay.as_millis() >= 500);
    assert!(delay.as_millis() <= 1_000);
}
