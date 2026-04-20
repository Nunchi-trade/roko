//! `roko new` scaffolders — generate compilable boilerplate for Synapse traits.
//!
//! Nine scaffold types are supported:
//! - `gate` — Gate trait impl with one passing test
//! - `scorer` — Scorer trait impl
//! - `router` — Router trait impl
//! - `policy` — Policy trait impl
//! - `substrate` — Substrate trait impl
//! - `composer` — Composer trait impl
//! - `domain` — Full domain profile with config, gates, templates
//! - `template` — Prompt template module
//! - `event-source` — EventSource trait impl

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

/// Supported scaffold types.
pub const SCAFFOLD_TYPES: &[&str] = &[
    "gate",
    "scorer",
    "router",
    "policy",
    "substrate",
    "composer",
    "domain",
    "template",
    "event-source",
];

/// Generate a scaffold of the given type with the given name.
///
/// Writes files under `output_dir` (defaults to current directory).
/// Returns the list of files created.
pub fn scaffold(type_name: &str, name: &str, output_dir: &Path) -> Result<Vec<PathBuf>> {
    match type_name {
        "gate" => scaffold_gate(name, output_dir),
        "scorer" => scaffold_scorer(name, output_dir),
        "router" => scaffold_router(name, output_dir),
        "policy" => scaffold_policy(name, output_dir),
        "substrate" => scaffold_substrate(name, output_dir),
        "composer" => scaffold_composer(name, output_dir),
        "domain" => scaffold_domain(name, output_dir),
        "template" => scaffold_template(name, output_dir),
        "event-source" | "event_source" => scaffold_event_source(name, output_dir),
        _ => bail!(
            "unknown scaffold type: `{type_name}`\navailable types: {}",
            SCAFFOLD_TYPES.join(", ")
        ),
    }
}

/// Convert a kebab-case or snake_case name to PascalCase for struct names.
fn to_pascal_case(name: &str) -> String {
    name.split(|c: char| c == '-' || c == '_')
        .filter(|s| !s.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                None => String::new(),
                Some(c) => c.to_uppercase().collect::<String>() + &chars.as_str().to_lowercase(),
            }
        })
        .collect()
}

/// Convert a name to snake_case for module and file names.
fn to_snake_case(name: &str) -> String {
    name.replace('-', "_").to_lowercase()
}

fn write_scaffold_file(path: &Path, content: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating directory {}", parent.display()))?;
    }
    fs::write(path, content).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

fn scaffold_gate(name: &str, output_dir: &Path) -> Result<Vec<PathBuf>> {
    let struct_name = format!("{}Gate", to_pascal_case(name));
    let mod_name = to_snake_case(name);
    let file_path = output_dir.join(format!("{mod_name}_gate.rs"));

    let content = format!(
        r#"//! {struct_name} — custom gate implementation.

use async_trait::async_trait;
use roko_core::{{Body, Context, Engram, Kind, Result}};
use roko_core::traits::Gate;

/// {struct_name} validates engrams against custom criteria.
pub struct {struct_name} {{
    /// Minimum score threshold for passing the gate.
    pub threshold: f32,
}}

impl {struct_name} {{
    /// Create a new {struct_name} with the given threshold.
    pub fn new(threshold: f32) -> Self {{
        Self {{ threshold }}
    }}
}}

#[async_trait]
impl Gate for {struct_name} {{
    async fn check(&self, engram: &Engram, ctx: &Context) -> Result<bool> {{
        // TODO: implement your gate logic here.
        // Return true if the engram passes, false if it should be rejected.
        let _ = ctx;
        Ok(engram.score >= self.threshold)
    }}

    fn name(&self) -> &'static str {{
        "{mod_name}_gate"
    }}
}}

#[cfg(test)]
mod tests {{
    use super::*;
    use roko_core::{{Body, ContentHash, Engram, Kind, Provenance}};

    fn test_engram(score: f32) -> Engram {{
        Engram {{
            hash: ContentHash::from_bytes(b"test"),
            kind: Kind::Signal,
            body: Body::Text("test engram".into()),
            score,
            provenance: Provenance::default(),
            parents: vec![],
            created_ms: 0,
        }}
    }}

    fn test_ctx() -> Context {{
        Context::default()
    }}

    #[tokio::test]
    async fn passes_above_threshold() {{
        let gate = {struct_name}::new(0.5);
        let engram = test_engram(0.8);
        assert!(gate.check(&engram, &test_ctx()).await.unwrap());
    }}

    #[tokio::test]
    async fn rejects_below_threshold() {{
        let gate = {struct_name}::new(0.5);
        let engram = test_engram(0.2);
        assert!(!gate.check(&engram, &test_ctx()).await.unwrap());
    }}
}}
"#
    );

    write_scaffold_file(&file_path, &content)?;
    Ok(vec![file_path])
}

fn scaffold_scorer(name: &str, output_dir: &Path) -> Result<Vec<PathBuf>> {
    let struct_name = format!("{}Scorer", to_pascal_case(name));
    let mod_name = to_snake_case(name);
    let file_path = output_dir.join(format!("{mod_name}_scorer.rs"));

    let content = format!(
        r#"//! {struct_name} — custom scorer implementation.

use async_trait::async_trait;
use roko_core::{{Context, Engram, Result}};
use roko_core::traits::Scorer;

/// {struct_name} assigns relevance scores to engrams.
pub struct {struct_name};

#[async_trait]
impl Scorer for {struct_name} {{
    async fn score(&self, engram: &Engram, ctx: &Context) -> Result<f32> {{
        // TODO: implement your scoring logic here.
        // Return a value in [0.0, 1.0] indicating relevance.
        let _ = ctx;
        Ok(engram.score)
    }}

    fn name(&self) -> &'static str {{
        "{mod_name}_scorer"
    }}
}}

#[cfg(test)]
mod tests {{
    use super::*;
    use roko_core::{{Body, ContentHash, Engram, Kind, Provenance}};

    fn test_engram(score: f32) -> Engram {{
        Engram {{
            hash: ContentHash::from_bytes(b"test"),
            kind: Kind::Signal,
            body: Body::Text("test engram".into()),
            score,
            provenance: Provenance::default(),
            parents: vec![],
            created_ms: 0,
        }}
    }}

    #[tokio::test]
    async fn scores_engram() {{
        let scorer = {struct_name};
        let engram = test_engram(0.75);
        let score = scorer.score(&engram, &Context::default()).await.unwrap();
        assert!((score - 0.75).abs() < f32::EPSILON);
    }}
}}
"#
    );

    write_scaffold_file(&file_path, &content)?;
    Ok(vec![file_path])
}

fn scaffold_router(name: &str, output_dir: &Path) -> Result<Vec<PathBuf>> {
    let struct_name = format!("{}Router", to_pascal_case(name));
    let mod_name = to_snake_case(name);
    let file_path = output_dir.join(format!("{mod_name}_router.rs"));

    let content = format!(
        r#"//! {struct_name} — custom router implementation.

use async_trait::async_trait;
use roko_core::{{Context, Engram, Result}};
use roko_core::traits::Router;

/// {struct_name} routes engrams to appropriate handlers.
pub struct {struct_name} {{
    /// Default route label when no specific route matches.
    pub default_route: String,
}}

impl {struct_name} {{
    /// Create a new {struct_name} with the given default route.
    pub fn new(default_route: impl Into<String>) -> Self {{
        Self {{ default_route: default_route.into() }}
    }}
}}

#[async_trait]
impl Router for {struct_name} {{
    async fn route(&self, engram: &Engram, ctx: &Context) -> Result<String> {{
        // TODO: implement your routing logic here.
        // Return a route label string identifying the target handler.
        let _ = (engram, ctx);
        Ok(self.default_route.clone())
    }}

    fn name(&self) -> &'static str {{
        "{mod_name}_router"
    }}
}}

#[cfg(test)]
mod tests {{
    use super::*;
    use roko_core::{{Body, ContentHash, Engram, Kind, Provenance}};

    fn test_engram() -> Engram {{
        Engram {{
            hash: ContentHash::from_bytes(b"test"),
            kind: Kind::Signal,
            body: Body::Text("test engram".into()),
            score: 1.0,
            provenance: Provenance::default(),
            parents: vec![],
            created_ms: 0,
        }}
    }}

    #[tokio::test]
    async fn routes_to_default() {{
        let router = {struct_name}::new("default");
        let route = router.route(&test_engram(), &Context::default()).await.unwrap();
        assert_eq!(route, "default");
    }}
}}
"#
    );

    write_scaffold_file(&file_path, &content)?;
    Ok(vec![file_path])
}

fn scaffold_policy(name: &str, output_dir: &Path) -> Result<Vec<PathBuf>> {
    let struct_name = format!("{}Policy", to_pascal_case(name));
    let mod_name = to_snake_case(name);
    let file_path = output_dir.join(format!("{mod_name}_policy.rs"));

    let content = format!(
        r#"//! {struct_name} — custom policy implementation.

use async_trait::async_trait;
use roko_core::{{Context, Engram, Result}};
use roko_core::traits::Policy;

/// {struct_name} decides whether an action should proceed.
pub struct {struct_name};

#[async_trait]
impl Policy for {struct_name} {{
    async fn evaluate(&self, engram: &Engram, ctx: &Context) -> Result<bool> {{
        // TODO: implement your policy logic here.
        // Return true to permit the action, false to deny.
        let _ = (engram, ctx);
        Ok(true)
    }}

    fn name(&self) -> &'static str {{
        "{mod_name}_policy"
    }}
}}

#[cfg(test)]
mod tests {{
    use super::*;
    use roko_core::{{Body, ContentHash, Engram, Kind, Provenance}};

    fn test_engram() -> Engram {{
        Engram {{
            hash: ContentHash::from_bytes(b"test"),
            kind: Kind::Signal,
            body: Body::Text("test engram".into()),
            score: 1.0,
            provenance: Provenance::default(),
            parents: vec![],
            created_ms: 0,
        }}
    }}

    #[tokio::test]
    async fn permits_by_default() {{
        let policy = {struct_name};
        assert!(policy.evaluate(&test_engram(), &Context::default()).await.unwrap());
    }}
}}
"#
    );

    write_scaffold_file(&file_path, &content)?;
    Ok(vec![file_path])
}

fn scaffold_substrate(name: &str, output_dir: &Path) -> Result<Vec<PathBuf>> {
    let struct_name = format!("{}Substrate", to_pascal_case(name));
    let mod_name = to_snake_case(name);
    let file_path = output_dir.join(format!("{mod_name}_substrate.rs"));

    let content = format!(
        r#"//! {struct_name} — custom substrate implementation.

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;
use roko_core::{{Body, ContentHash, Context, Engram, Kind, Query, Result}};
use roko_core::traits::Substrate;

/// {struct_name} stores engrams in memory.
pub struct {struct_name} {{
    store: Mutex<HashMap<ContentHash, Engram>>,
}}

impl {struct_name} {{
    /// Create a new empty {struct_name}.
    pub fn new() -> Self {{
        Self {{
            store: Mutex::new(HashMap::new()),
        }}
    }}
}}

impl Default for {struct_name} {{
    fn default() -> Self {{
        Self::new()
    }}
}}

#[async_trait]
impl Substrate for {struct_name} {{
    async fn put(&self, engram: Engram) -> Result<ContentHash> {{
        let hash = engram.hash.clone();
        self.store.lock().unwrap().insert(hash.clone(), engram);
        Ok(hash)
    }}

    async fn get(&self, id: &ContentHash) -> Result<Option<Engram>> {{
        Ok(self.store.lock().unwrap().get(id).cloned())
    }}

    async fn query(&self, _q: &Query, _ctx: &Context) -> Result<Vec<Engram>> {{
        // TODO: implement query filtering logic.
        Ok(self.store.lock().unwrap().values().cloned().collect())
    }}

    async fn prune(&self, threshold: f32, _ctx: &Context) -> Result<usize> {{
        let mut store = self.store.lock().unwrap();
        let before = store.len();
        store.retain(|_, e| e.score >= threshold);
        Ok(before - store.len())
    }}

    async fn len(&self) -> Result<usize> {{
        Ok(self.store.lock().unwrap().len())
    }}

    fn name(&self) -> &'static str {{
        "{mod_name}_substrate"
    }}
}}

#[cfg(test)]
mod tests {{
    use super::*;
    use roko_core::Provenance;

    fn test_engram() -> Engram {{
        Engram {{
            hash: ContentHash::from_bytes(b"test"),
            kind: Kind::Signal,
            body: Body::Text("test engram".into()),
            score: 1.0,
            provenance: Provenance::default(),
            parents: vec![],
            created_ms: 0,
        }}
    }}

    #[tokio::test]
    async fn put_and_get() {{
        let substrate = {struct_name}::new();
        let engram = test_engram();
        let hash = substrate.put(engram.clone()).await.unwrap();
        let retrieved = substrate.get(&hash).await.unwrap();
        assert!(retrieved.is_some());
    }}

    #[tokio::test]
    async fn starts_empty() {{
        let substrate = {struct_name}::new();
        assert_eq!(substrate.len().await.unwrap(), 0);
    }}
}}
"#
    );

    write_scaffold_file(&file_path, &content)?;
    Ok(vec![file_path])
}

fn scaffold_composer(name: &str, output_dir: &Path) -> Result<Vec<PathBuf>> {
    let struct_name = format!("{}Composer", to_pascal_case(name));
    let mod_name = to_snake_case(name);
    let file_path = output_dir.join(format!("{mod_name}_composer.rs"));

    let content = format!(
        r#"//! {struct_name} — custom composer implementation.

use async_trait::async_trait;
use roko_core::{{Context, Engram, Result}};
use roko_core::traits::Composer;

/// {struct_name} assembles context from engram history.
pub struct {struct_name};

#[async_trait]
impl Composer for {struct_name} {{
    async fn compose(&self, engrams: &[Engram], ctx: &Context) -> Result<String> {{
        // TODO: implement your composition logic here.
        // Combine the engrams into a prompt string for the model.
        let _ = ctx;
        let parts: Vec<&str> = engrams
            .iter()
            .filter_map(|e| e.body.as_text())
            .collect();
        Ok(parts.join("\n\n"))
    }}

    fn name(&self) -> &'static str {{
        "{mod_name}_composer"
    }}
}}

#[cfg(test)]
mod tests {{
    use super::*;
    use roko_core::{{Body, ContentHash, Engram, Kind, Provenance}};

    fn test_engrams() -> Vec<Engram> {{
        vec![
            Engram {{
                hash: ContentHash::from_bytes(b"a"),
                kind: Kind::Signal,
                body: Body::Text("first".into()),
                score: 1.0,
                provenance: Provenance::default(),
                parents: vec![],
                created_ms: 0,
            }},
            Engram {{
                hash: ContentHash::from_bytes(b"b"),
                kind: Kind::Signal,
                body: Body::Text("second".into()),
                score: 1.0,
                provenance: Provenance::default(),
                parents: vec![],
                created_ms: 0,
            }},
        ]
    }}

    #[tokio::test]
    async fn composes_engrams() {{
        let composer = {struct_name};
        let result = composer.compose(&test_engrams(), &Context::default()).await.unwrap();
        assert!(result.contains("first"));
        assert!(result.contains("second"));
    }}
}}
"#
    );

    write_scaffold_file(&file_path, &content)?;
    Ok(vec![file_path])
}

fn scaffold_domain(name: &str, output_dir: &Path) -> Result<Vec<PathBuf>> {
    let pascal = to_pascal_case(name);
    let snake = to_snake_case(name);
    let dir = output_dir.join(&snake);

    let mut files = Vec::new();

    // mod.rs
    let mod_path = dir.join("mod.rs");
    let mod_content = format!(
        r#"//! {pascal} domain profile.
//!
//! This module defines a complete domain profile including:
//! - Gate implementation
//! - Prompt template
//! - Domain configuration

pub mod gate;
pub mod template;

/// Domain identifier for this profile.
pub const DOMAIN_NAME: &str = "{snake}";
"#
    );
    write_scaffold_file(&mod_path, &mod_content)?;
    files.push(mod_path);

    // gate.rs
    let gate_path = dir.join("gate.rs");
    let gate_content = format!(
        r#"//! {pascal} domain gate.

use async_trait::async_trait;
use roko_core::{{Context, Engram, Result}};
use roko_core::traits::Gate;

/// Gate for the {pascal} domain.
pub struct {pascal}Gate;

#[async_trait]
impl Gate for {pascal}Gate {{
    async fn check(&self, engram: &Engram, _ctx: &Context) -> Result<bool> {{
        Ok(engram.score > 0.0)
    }}

    fn name(&self) -> &'static str {{
        "{snake}_domain_gate"
    }}
}}

#[cfg(test)]
mod tests {{
    use super::*;
    use roko_core::{{Body, ContentHash, Engram, Kind, Provenance}};

    #[tokio::test]
    async fn gate_passes_positive_score() {{
        let gate = {pascal}Gate;
        let engram = Engram {{
            hash: ContentHash::from_bytes(b"test"),
            kind: Kind::Signal,
            body: Body::Text("test".into()),
            score: 0.5,
            provenance: Provenance::default(),
            parents: vec![],
            created_ms: 0,
        }};
        assert!(gate.check(&engram, &Context::default()).await.unwrap());
    }}
}}
"#
    );
    write_scaffold_file(&gate_path, &gate_content)?;
    files.push(gate_path);

    // template.rs
    let tmpl_path = dir.join("template.rs");
    let tmpl_content = format!(
        r##"//! {pascal} domain prompt template.

/// System prompt template for the {pascal} domain.
pub const SYSTEM_PROMPT: &str = "You are an agent operating in the {snake} domain.\n\
\n\
Your responsibilities:\n\
- Follow domain-specific conventions\n\
- Validate outputs against domain requirements\n\
- Report progress through structured signals\n";

/// Render the domain prompt with optional context.
pub fn render_prompt(context: Option<&str>) -> String {{
    match context {{
        Some(ctx) => format!("{{}}\n\nContext:\n{{}}", SYSTEM_PROMPT, ctx),
        None => SYSTEM_PROMPT.to_string(),
    }}
}}

#[cfg(test)]
mod tests {{
    use super::*;

    #[test]
    fn renders_base_prompt() {{
        let prompt = render_prompt(None);
        assert!(prompt.contains("{snake}"));
    }}

    #[test]
    fn renders_with_context() {{
        let prompt = render_prompt(Some("extra info"));
        assert!(prompt.contains("Context:"));
        assert!(prompt.contains("extra info"));
    }}
}}
"##
    );
    write_scaffold_file(&tmpl_path, &tmpl_content)?;
    files.push(tmpl_path);

    Ok(files)
}

fn scaffold_template(name: &str, output_dir: &Path) -> Result<Vec<PathBuf>> {
    let snake = to_snake_case(name);
    let pascal = to_pascal_case(name);
    let file_path = output_dir.join(format!("{snake}_template.rs"));

    let content = format!(
        "//! {pascal} prompt template.\n\
         \n\
         /// System prompt for the {pascal} template.\n\
         pub const SYSTEM_PROMPT: &str = \"You are an agent using the {snake} template.\\n\\\n\
         \\n\\\n\
         Follow these guidelines:\\n\\\n\
         - Be concise and accurate\\n\\\n\
         - Validate your work before reporting completion\\n\\\n\
         - Use structured output when available\\n\";\n\
         \n\
         #[cfg(test)]\n\
         mod tests {{\n\
             use super::*;\n\
         \n\
             #[test]\n\
             fn system_prompt_is_not_empty() {{\n\
                 assert!(!SYSTEM_PROMPT.is_empty());\n\
             }}\n\
         \n\
             #[test]\n\
             fn contains_template_name() {{\n\
                 assert!(SYSTEM_PROMPT.contains(\"{snake}\"));\n\
             }}\n\
         }}\n"
    );

    write_scaffold_file(&file_path, &content)?;
    Ok(vec![file_path])
}

fn scaffold_event_source(name: &str, output_dir: &Path) -> Result<Vec<PathBuf>> {
    let struct_name = format!("{}EventSource", to_pascal_case(name));
    let mod_name = to_snake_case(name);
    let file_path = output_dir.join(format!("{mod_name}_event_source.rs"));

    let content = format!(
        r#"//! {struct_name} — custom event source implementation.

use std::time::Duration;

/// An event produced by this source.
#[derive(Debug, Clone)]
pub struct Event {{
    /// Event payload.
    pub payload: String,
    /// Timestamp in milliseconds.
    pub timestamp_ms: u64,
}}

/// {struct_name} polls for events at a configurable interval.
pub struct {struct_name} {{
    /// Polling interval.
    pub interval: Duration,
    /// Source identifier.
    pub source_id: String,
}}

impl {struct_name} {{
    /// Create a new {struct_name} with the given interval.
    pub fn new(source_id: impl Into<String>, interval: Duration) -> Self {{
        Self {{
            interval,
            source_id: source_id.into(),
        }}
    }}

    /// Poll for new events. Returns events since last poll.
    pub async fn poll(&self) -> Vec<Event> {{
        // TODO: implement your event polling logic here.
        Vec::new()
    }}

    /// Human-readable name for this event source.
    pub fn name(&self) -> &str {{
        &self.source_id
    }}
}}

#[cfg(test)]
mod tests {{
    use super::*;

    #[tokio::test]
    async fn poll_returns_empty_by_default() {{
        let source = {struct_name}::new("{mod_name}", Duration::from_secs(60));
        let events = source.poll().await;
        assert!(events.is_empty());
    }}

    #[test]
    fn has_source_id() {{
        let source = {struct_name}::new("my-source", Duration::from_secs(30));
        assert_eq!(source.name(), "my-source");
    }}
}}
"#
    );

    write_scaffold_file(&file_path, &content)?;
    Ok(vec![file_path])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn to_pascal_case_works() {
        assert_eq!(to_pascal_case("my-custom"), "MyCustom");
        assert_eq!(to_pascal_case("hello_world"), "HelloWorld");
        assert_eq!(to_pascal_case("simple"), "Simple");
        assert_eq!(to_pascal_case("my-cool-thing"), "MyCoolThing");
    }

    #[test]
    fn to_snake_case_works() {
        assert_eq!(to_snake_case("my-custom"), "my_custom");
        assert_eq!(to_snake_case("hello_world"), "hello_world");
        assert_eq!(to_snake_case("Simple"), "simple");
    }

    #[test]
    fn scaffold_types_list_is_complete() {
        assert_eq!(SCAFFOLD_TYPES.len(), 9);
        assert!(SCAFFOLD_TYPES.contains(&"gate"));
        assert!(SCAFFOLD_TYPES.contains(&"scorer"));
        assert!(SCAFFOLD_TYPES.contains(&"router"));
        assert!(SCAFFOLD_TYPES.contains(&"policy"));
        assert!(SCAFFOLD_TYPES.contains(&"substrate"));
        assert!(SCAFFOLD_TYPES.contains(&"composer"));
        assert!(SCAFFOLD_TYPES.contains(&"domain"));
        assert!(SCAFFOLD_TYPES.contains(&"template"));
        assert!(SCAFFOLD_TYPES.contains(&"event-source"));
    }

    #[test]
    fn scaffold_gate_produces_output() {
        let dir = tempfile::tempdir().unwrap();
        let files = scaffold_gate("my-custom", dir.path()).unwrap();
        assert_eq!(files.len(), 1);
        let content = std::fs::read_to_string(&files[0]).unwrap();
        assert!(content.contains("MyCustomGate"));
        assert!(content.contains("impl Gate for"));
        assert!(content.contains("#[cfg(test)]"));
        assert!(content.contains("passes_above_threshold"));
    }

    #[test]
    fn scaffold_scorer_produces_output() {
        let dir = tempfile::tempdir().unwrap();
        let files = scaffold_scorer("relevance", dir.path()).unwrap();
        assert_eq!(files.len(), 1);
        let content = std::fs::read_to_string(&files[0]).unwrap();
        assert!(content.contains("RelevanceScorer"));
        assert!(content.contains("impl Scorer for"));
    }

    #[test]
    fn scaffold_router_produces_output() {
        let dir = tempfile::tempdir().unwrap();
        let files = scaffold_router("priority", dir.path()).unwrap();
        assert_eq!(files.len(), 1);
        let content = std::fs::read_to_string(&files[0]).unwrap();
        assert!(content.contains("PriorityRouter"));
        assert!(content.contains("impl Router for"));
    }

    #[test]
    fn scaffold_policy_produces_output() {
        let dir = tempfile::tempdir().unwrap();
        let files = scaffold_policy("budget", dir.path()).unwrap();
        assert_eq!(files.len(), 1);
        let content = std::fs::read_to_string(&files[0]).unwrap();
        assert!(content.contains("BudgetPolicy"));
        assert!(content.contains("impl Policy for"));
    }

    #[test]
    fn scaffold_substrate_produces_output() {
        let dir = tempfile::tempdir().unwrap();
        let files = scaffold_substrate("redis", dir.path()).unwrap();
        assert_eq!(files.len(), 1);
        let content = std::fs::read_to_string(&files[0]).unwrap();
        assert!(content.contains("RedisSubstrate"));
        assert!(content.contains("impl Substrate for"));
    }

    #[test]
    fn scaffold_composer_produces_output() {
        let dir = tempfile::tempdir().unwrap();
        let files = scaffold_composer("summary", dir.path()).unwrap();
        assert_eq!(files.len(), 1);
        let content = std::fs::read_to_string(&files[0]).unwrap();
        assert!(content.contains("SummaryComposer"));
        assert!(content.contains("impl Composer for"));
    }

    #[test]
    fn scaffold_domain_produces_directory() {
        let dir = tempfile::tempdir().unwrap();
        let files = scaffold_domain("defi", dir.path()).unwrap();
        assert_eq!(files.len(), 3);
        // Check mod.rs exists
        assert!(files.iter().any(|f| f.ends_with("mod.rs")));
        // Check gate.rs exists
        assert!(files.iter().any(|f| f.ends_with("gate.rs")));
        // Check template.rs exists
        assert!(files.iter().any(|f| f.ends_with("template.rs")));
    }

    #[test]
    fn scaffold_template_produces_output() {
        let dir = tempfile::tempdir().unwrap();
        let files = scaffold_template("code-review", dir.path()).unwrap();
        assert_eq!(files.len(), 1);
        let content = std::fs::read_to_string(&files[0]).unwrap();
        assert!(content.contains("SYSTEM_PROMPT"));
        assert!(content.contains("code_review"));
    }

    #[test]
    fn scaffold_event_source_produces_output() {
        let dir = tempfile::tempdir().unwrap();
        let files = scaffold_event_source("github-webhook", dir.path()).unwrap();
        assert_eq!(files.len(), 1);
        let content = std::fs::read_to_string(&files[0]).unwrap();
        assert!(content.contains("GithubWebhookEventSource"));
        assert!(content.contains("async fn poll"));
    }

    #[test]
    fn unknown_type_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let result = scaffold("unknown", "test", dir.path());
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("unknown scaffold type"));
    }

    #[test]
    fn scaffold_dispatch_works_for_all_types() {
        let dir = tempfile::tempdir().unwrap();
        let name = "test_thing";
        for ty in SCAFFOLD_TYPES {
            let result = scaffold(ty, name, dir.path());
            assert!(result.is_ok(), "scaffold failed for type: {}", ty);
            let files = result.unwrap();
            assert!(
                !files.is_empty(),
                "scaffold produced no files for type: {}",
                ty
            );
        }
    }
}
