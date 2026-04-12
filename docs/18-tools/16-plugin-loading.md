# 16 — Plugin Loading Mechanisms

> Three mechanisms: Cargo workspace (compile-time), config-declared (runtime),
> MCP discovery (runtime). Lifecycle, validation, and hot-reload.

---

## Overview

Roko supports three mechanisms for loading plugins, each with different tradeoffs between
type safety, deployment flexibility, and isolation. These mechanisms are not mutually
exclusive — a production deployment typically uses all three simultaneously.

---

## Mechanism 1: Cargo Workspace Members (Compile-Time)

Domain plugins implemented as workspace crates are compiled directly into the binary.

### How It Works

```toml
# Cargo.toml (workspace root)
[workspace]
members = [
    # Core crates
    "crates/roko-core",
    "crates/roko-std",
    "crates/roko-agent",
    # Domain plugins (compile-time)
    "crates/roko-domain-chain",
    "crates/roko-domain-medical",
    "crates/roko-domain-legal",
]
```

At build time, Cargo compiles all workspace members. Domain plugins register their components
at application startup:

```rust
fn main() {
    let mut engine = Engine::new(config);

    // Register domain plugins at startup
    roko_domain_chain::register_domain(&mut engine);
    roko_domain_medical::register_domain(&mut engine);

    engine.run().await;
}
```

### Registration Pattern

Each domain plugin exports a `register_domain` function:

```rust
// crates/roko-domain-chain/src/lib.rs
pub fn register_domain(engine: &mut Engine) {
    // Register custom Engram kinds
    engine.register_kind(Kind::Custom("chain.transaction"));
    engine.register_kind(Kind::Custom("chain.block_event"));

    // Register gates
    engine.register_gate(Box::new(TxSimGate::new()));
    engine.register_gate(Box::new(WalletGate::new()));
    engine.register_gate(Box::new(VerifyChainGate::new()));

    // Register scorer
    engine.register_scorer(Box::new(ChainRelevanceScorer));

    // Register tools (423+ chain domain tools)
    for tool_def in chain_tools::ALL_TOOL_DEFS.iter() {
        engine.register_tool(tool_def);
    }

    // Register T0 probes
    for probe in chain_probes::probes() {
        engine.register_probe(probe);
    }

    // Register somatic dimensions
    engine.register_somatic_space(ChainSomaticSpace::dimensions());
}
```

### Properties

| Property | Value |
|---|---|
| **Type safety** | Full — compiler checks all trait implementations |
| **Performance** | Maximum — no IPC, no serialization overhead |
| **Hot-reload** | No — requires recompilation |
| **Isolation** | None — runs in-process |
| **Distribution** | Via Cargo crate (crates.io or git) |
| **Best for** | First-party domain plugins, performance-critical tools |

---

## Mechanism 2: Config-Declared (Runtime)

Plugins declared in `roko.toml` are loaded at runtime via dynamic linking (shared libraries).

### How It Works

```toml
# roko.toml
[[plugins]]
name = "medical"
path = "./plugins/libroko_domain_medical.so"  # .dylib on macOS
config = { reference_db = "/data/medical-refs.db" }

[[plugins]]
name = "custom-scorer"
path = "./plugins/libcustom_scorer.so"
config = { threshold = 0.85 }
```

At startup, the runtime loads each declared plugin:

```rust
pub async fn load_plugins(config: &AppConfig, engine: &mut Engine) -> Result<()> {
    for plugin_config in &config.plugins {
        // Load the shared library
        let lib = unsafe { libloading::Library::new(&plugin_config.path)? };

        // Get the registration function
        let register: libloading::Symbol<fn(&mut Engine, &serde_json::Value)> =
            unsafe { lib.get(b"register_plugin")? };

        // Call registration with plugin-specific config
        register(engine, &plugin_config.config);

        info!(name = %plugin_config.name, path = %plugin_config.path.display(), "loaded plugin");
    }

    Ok(())
}
```

### Plugin ABI Contract

Dynamic plugins must export a C-compatible registration function:

```rust
// In the plugin crate
#[no_mangle]
pub extern "C" fn register_plugin(engine: &mut Engine, config: &serde_json::Value) {
    let reference_db = config["reference_db"].as_str().unwrap();
    let gate = MedicalAccuracyGate::new(reference_db);
    engine.register_gate(Box::new(gate));
    // ... register other components
}
```

### Properties

| Property | Value |
|---|---|
| **Type safety** | Partial — ABI contract checked at load time |
| **Performance** | Near-native — function pointer calls |
| **Hot-reload** | Possible — unload and reload the shared library |
| **Isolation** | Minimal — shares process memory |
| **Distribution** | Via shared library file |
| **Best for** | Third-party plugins, hot-swappable components |

### Platform Considerations

| Platform | Library Extension | Notes |
|---|---|---|
| Linux | `.so` | Standard ELF shared object |
| macOS | `.dylib` | Mach-O dynamic library |
| Windows | `.dll` | PE dynamic link library |

---

## Mechanism 3: MCP Tool Discovery (Runtime)

MCP servers are discovered and loaded at runtime via the Model Context Protocol.

### How It Works

```toml
# roko.toml
[[agent.mcp_servers]]
name = "medical-tools"
command = "roko-mcp-medical"
args = ["--db", "/data/medical.db"]
env = { MEDICAL_API_KEY = "${MEDICAL_API_KEY}" }
```

At agent startup:
1. The MCP client spawns the server process
2. Sends `tools/list` to discover available tools
3. Converts MCP tool schemas to Roko ToolDef format
4. Merges discovered tools into the agent's tool registry
5. Tool calls are dispatched via `tools/call` over stdio

```rust
pub async fn load_mcp_tools(
    servers: &[McpServerConfig],
    registry: &mut MergedToolRegistry,
) -> Result<Vec<McpServerHandle>> {
    let mut handles = Vec::new();

    for server_config in servers {
        // Spawn the MCP server process
        let handle = McpServerHandle::spawn(server_config).await?;

        // Discover tools
        let tools = handle.client.discover_tools().await?;

        // Convert and register
        for mcp_tool in tools {
            let tool_def = convert_mcp_tool(&mcp_tool, &server_config.name);
            registry.add_mcp_tool(tool_def);
        }

        handles.push(handle);
    }

    Ok(handles)
}
```

### Properties

| Property | Value |
|---|---|
| **Type safety** | None — JSON Schema validation only |
| **Performance** | IPC overhead (~1-5ms per call via stdio) |
| **Hot-reload** | Yes — restart the server process |
| **Isolation** | Full — separate process, separate permissions |
| **Distribution** | Via binary or container image |
| **Best for** | Untrusted plugins, cross-language tools, security-sensitive operations |

---

## Mechanism Comparison

| Aspect | Workspace | Config-Declared | MCP Discovery |
|---|---|---|---|
| Load time | Compile time | Application startup | Agent session startup |
| Language | Rust only | Rust only (ABI) | Any language |
| Type checking | Full (compiler) | Partial (ABI contract) | None (JSON Schema) |
| IPC cost | Zero | Negligible | ~1-5ms per call |
| Process isolation | No | No | Yes |
| Hot-reload | No | Possible | Yes |
| Security | High trust | Medium trust | Low trust (sandboxable) |
| Memory sharing | Full | Full | None |
| Debugging | Standard Rust tools | dlopen debugging | Process debugging |

---

## Plugin Lifecycle

All three mechanisms follow the same lifecycle:

```
Discovery → Validation → Initialization → Running → Shutdown
```

### Discovery

| Mechanism | Discovery Method |
|---|---|
| Workspace | Cargo build graph + explicit `register_domain()` calls |
| Config-declared | `[[plugins]]` entries in `roko.toml` |
| MCP | `[[agent.mcp_servers]]` entries + `tools/list` protocol call |

### Validation

On load, plugins are validated regardless of mechanism:

```rust
pub fn validate_plugin(plugin: &dyn Plugin) -> Result<Vec<Warning>> {
    let mut warnings = Vec::new();

    // Check version compatibility
    if plugin.api_version() != ROKO_PLUGIN_API_VERSION {
        return Err(PluginError::IncompatibleVersion {
            expected: ROKO_PLUGIN_API_VERSION,
            actual: plugin.api_version(),
        });
    }

    // Check for tool name conflicts
    for tool in plugin.tools() {
        if engine.has_tool(tool.name) {
            warnings.push(Warning::ToolNameConflict {
                name: tool.name.to_string(),
                existing_source: engine.tool_source(tool.name),
                new_source: plugin.name().to_string(),
            });
        }
    }

    // Check required capabilities
    for cap in plugin.required_capabilities() {
        if !engine.has_capability(cap) {
            warnings.push(Warning::MissingCapability {
                capability: cap.to_string(),
                plugin: plugin.name().to_string(),
            });
        }
    }

    Ok(warnings)
}
```

### Initialization

Each plugin receives its configuration and initializes:

```rust
#[async_trait]
pub trait Plugin: Send + Sync {
    fn name(&self) -> &str;
    fn api_version(&self) -> u32;
    fn tools(&self) -> &[ToolDef];
    fn required_capabilities(&self) -> &[&str];

    async fn init(&self, config: &serde_json::Value) -> Result<()>;
    async fn shutdown(&self) -> Result<()>;
}
```

### Health Monitoring

The runtime monitors plugin health:

| Check | Frequency | Action on Failure |
|---|---|---|
| Heartbeat (MCP) | Every 30s | Restart server process |
| Error rate | Continuous | Circuit breaker (5 errors → pause 60s) |
| Memory usage | Every 60s | Warning at 80% of limit |
| Response time | Per call | Warning at >5s, timeout at configurable limit |

---

## Recommended Loading Strategy

| Plugin Type | Recommended Mechanism | Rationale |
|---|---|---|
| Core domain (chain, coding) | **Workspace** | Performance-critical, heavily tested |
| Custom domain (medical, legal) | **Workspace** or **Config** | Depends on deployment model |
| Operations tools (GitHub, Slack) | **MCP** | Process isolation, language flexibility |
| Community plugins | **MCP** | Untrusted code, easy distribution |
| Experimental tools | **MCP** | Easy to swap, no recompilation |
| Performance-sensitive tools | **Workspace** | Zero IPC overhead |

The typical production deployment uses workspace for core domains, MCP for operations
integrations, and potentially config-declared for hot-swappable custom components.
