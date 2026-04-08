//! CLI subcommands for secret management (items 43.17).
//!
//! Provides `roko secrets {set, get, list, rotate}` subcommands that wire
//! into the `roko-core` secret store backends.

use anyhow::{Context as _, Result};
use clap::Subcommand;
use roko_core::secrets::namespace::Namespace;
use roko_core::secrets::{FileStore, SecretStore};
use std::io::Read;
use std::path::Path;

/// `roko secrets` subcommands.
#[derive(Debug, Subcommand)]
pub enum SecretsCmd {
    /// Store a secret (reads value from stdin).
    Set {
        /// Secret namespace (e.g. `llm`).
        namespace: String,
        /// Secret key within namespace (e.g. `anthropic`).
        key: String,
    },
    /// Retrieve a secret value.
    Get {
        /// Secret namespace (e.g. `llm`).
        namespace: String,
        /// Secret key within namespace (e.g. `anthropic`).
        key: String,
    },
    /// List secret keys, optionally filtered by namespace.
    List {
        /// Optional namespace filter (e.g. `llm`). Omit to list all.
        namespace: Option<String>,
    },
    /// Rotate a secret (reads new value from stdin).
    Rotate {
        /// Secret namespace (e.g. `llm`).
        namespace: String,
        /// Secret key within namespace (e.g. `anthropic`).
        key: String,
    },
}

/// Execute a secrets subcommand against the store at `workdir/.roko/secrets.toml`.
pub fn dispatch_secrets(cmd: &SecretsCmd, workdir: &Path) -> Result<()> {
    let secrets_path = workdir.join(".roko").join("secrets.toml");
    let store = FileStore::open(&secrets_path)
        .with_context(|| format!("open secrets store at {}", secrets_path.display()))?;
    match cmd {
        SecretsCmd::Set { namespace, key } => cmd_set(&store, namespace, key),
        SecretsCmd::Get { namespace, key } => cmd_get(&store, namespace, key),
        SecretsCmd::List { namespace } => cmd_list(&store, namespace.as_deref(), &secrets_path),
        SecretsCmd::Rotate { namespace, key } => cmd_rotate(&store, namespace, key),
    }
}

fn cmd_set(store: &FileStore, namespace: &str, key: &str) -> Result<()> {
    let value = read_value_from_stdin()?;
    let ns = Namespace::new(namespace, key);
    store.set(&ns, value).map_err(|e| anyhow::anyhow!("{e}"))?;
    println!("secret {namespace}.{key} stored");
    Ok(())
}

fn cmd_get(store: &FileStore, namespace: &str, key: &str) -> Result<()> {
    let ns = Namespace::new(namespace, key);
    match store.get(&ns).map_err(|e| anyhow::anyhow!("{e}"))? {
        Some(value) => {
            println!("{value}");
        }
        None => {
            println!("(not set)");
        }
    }
    Ok(())
}

fn cmd_list(store: &FileStore, namespace: Option<&str>, secrets_path: &Path) -> Result<()> {
    // Read the TOML file directly to enumerate keys.
    let text = std::fs::read_to_string(secrets_path).unwrap_or_default();
    if text.trim().is_empty() {
        println!("(no secrets stored)");
        return Ok(());
    }
    let value: toml::Value = text.parse().with_context(|| "parse secrets.toml")?;
    let Some(root) = value.as_table() else {
        println!("(no secrets stored)");
        return Ok(());
    };

    let mut printed = false;
    for (cat, cat_val) in root {
        if let Some(filter) = namespace {
            if cat != filter {
                continue;
            }
        }
        if let Some(providers) = cat_val.as_table() {
            for provider_key in providers.keys() {
                let ns = Namespace::new(cat, provider_key);
                // Verify we can actually read it.
                let exists = store
                    .get(&ns)
                    .map_err(|e| anyhow::anyhow!("{e}"))?
                    .is_some();
                if exists {
                    println!("{cat}.{provider_key}");
                    printed = true;
                }
            }
        }
    }
    if !printed {
        if let Some(ns) = namespace {
            println!("(no secrets in namespace {ns})");
        } else {
            println!("(no secrets stored)");
        }
    }
    Ok(())
}

fn cmd_rotate(store: &FileStore, namespace: &str, key: &str) -> Result<()> {
    let new_value = read_value_from_stdin()?;
    let ns = Namespace::new(namespace, key);
    store
        .rotate(&ns, new_value)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    println!("secret {namespace}.{key} rotated");
    Ok(())
}

/// Read a secret value from stdin, trimming trailing whitespace.
fn read_value_from_stdin() -> Result<String> {
    let mut buf = String::new();
    std::io::stdin()
        .read_to_string(&mut buf)
        .context("read secret value from stdin")?;
    Ok(buf.trim_end().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_cmd_parse_set() {
        // Verify clap can parse the Set variant.
        use clap::Parser;
        #[derive(Parser)]
        struct Wrapper {
            #[command(subcommand)]
            cmd: SecretsCmd,
        }
        let w = Wrapper::parse_from(["test", "set", "llm", "anthropic"]);
        match w.cmd {
            SecretsCmd::Set { namespace, key } => {
                assert_eq!(namespace, "llm");
                assert_eq!(key, "anthropic");
            }
            _ => panic!("expected Set"),
        }
    }

    #[test]
    fn secrets_cmd_parse_get() {
        use clap::Parser;
        #[derive(Parser)]
        struct Wrapper {
            #[command(subcommand)]
            cmd: SecretsCmd,
        }
        let w = Wrapper::parse_from(["test", "get", "rpc", "alchemy"]);
        match w.cmd {
            SecretsCmd::Get { namespace, key } => {
                assert_eq!(namespace, "rpc");
                assert_eq!(key, "alchemy");
            }
            _ => panic!("expected Get"),
        }
    }

    #[test]
    fn secrets_cmd_parse_list_all() {
        use clap::Parser;
        #[derive(Parser)]
        struct Wrapper {
            #[command(subcommand)]
            cmd: SecretsCmd,
        }
        let w = Wrapper::parse_from(["test", "list"]);
        match w.cmd {
            SecretsCmd::List { namespace } => assert!(namespace.is_none()),
            _ => panic!("expected List"),
        }
    }

    #[test]
    fn secrets_cmd_parse_list_filtered() {
        use clap::Parser;
        #[derive(Parser)]
        struct Wrapper {
            #[command(subcommand)]
            cmd: SecretsCmd,
        }
        let w = Wrapper::parse_from(["test", "list", "llm"]);
        match w.cmd {
            SecretsCmd::List { namespace } => assert_eq!(namespace.as_deref(), Some("llm")),
            _ => panic!("expected List"),
        }
    }

    #[test]
    fn secrets_cmd_parse_rotate() {
        use clap::Parser;
        #[derive(Parser)]
        struct Wrapper {
            #[command(subcommand)]
            cmd: SecretsCmd,
        }
        let w = Wrapper::parse_from(["test", "rotate", "llm", "anthropic"]);
        match w.cmd {
            SecretsCmd::Rotate { namespace, key } => {
                assert_eq!(namespace, "llm");
                assert_eq!(key, "anthropic");
            }
            _ => panic!("expected Rotate"),
        }
    }

    #[test]
    fn dispatch_get_on_empty_store() {
        let dir = tempfile::tempdir().unwrap();
        let workdir = dir.path();
        std::fs::create_dir_all(workdir.join(".roko")).unwrap();
        let cmd = SecretsCmd::Get {
            namespace: "llm".into(),
            key: "anthropic".into(),
        };
        // Should succeed (prints "(not set)"), not panic.
        let result = dispatch_secrets(&cmd, workdir);
        assert!(result.is_ok());
    }

    #[test]
    fn dispatch_list_empty_store() {
        let dir = tempfile::tempdir().unwrap();
        let workdir = dir.path();
        std::fs::create_dir_all(workdir.join(".roko")).unwrap();
        let cmd = SecretsCmd::List { namespace: None };
        let result = dispatch_secrets(&cmd, workdir);
        assert!(result.is_ok());
    }
}
