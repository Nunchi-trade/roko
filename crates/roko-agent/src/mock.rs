//! `MockAgent` — deterministic agent for tests.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::agent::{Agent, AgentResult};
use crate::usage::Usage;
use async_trait::async_trait;
use roko_core::{Body, Context, Kind, Provenance, Signal};
use serde::Deserialize;
use thiserror::Error;

/// An agent that returns canned responses.
///
/// Use [`MockAgent::reply`] or [`MockAgent::fail_with`] for a single fixed
/// response, or [`MockAgent::scripted_from_fixture`] to replay a directory of
/// `turn-*.json` files in order.
pub struct MockAgent {
    default_name: String,
    default_usage: Usage,
    working_dir: Option<PathBuf>,
    mode: MockMode,
}

enum MockMode {
    Fixed {
        reply: String,
        fail: bool,
    },
    Scripted {
        turns: Vec<MockFixtureTurn>,
        cursor: ScriptCursor,
    },
}

enum ScriptCursor {
    InMemory(AtomicUsize),
    Persistent(PathBuf),
}

#[derive(Debug, Clone, Deserialize)]
struct MockFixtureTurn {
    reply: String,
    #[serde(default)]
    fail: bool,
    #[serde(default)]
    usage: Option<Usage>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    writes: Vec<MockFixtureWrite>,
}

#[derive(Debug, Clone, Deserialize)]
struct MockFixtureWrite {
    path: String,
    content: String,
}

/// Errors produced while loading a scripted mock fixture.
#[derive(Debug, Error)]
pub enum MockFixtureError {
    /// The fixture directory existed but no `turn-*.json` files were found.
    #[error("mock fixture directory {path} was empty")]
    Empty { path: PathBuf },
    /// The fixture directory could not be read.
    #[error("failed to read mock fixture directory {path}: {source}")]
    ReadDir {
        /// The directory that failed to load.
        path: PathBuf,
        /// The underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// A fixture file could not be read.
    #[error("failed to read mock fixture file {path}: {source}")]
    ReadFile {
        /// The file that failed to load.
        path: PathBuf,
        /// The underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// A fixture file could not be parsed as JSON.
    #[error("failed to parse mock fixture file {path}: {source}")]
    ParseFile {
        /// The file that failed to parse.
        path: PathBuf,
        /// The underlying JSON parse error.
        #[source]
        source: serde_json::Error,
    },
}

impl MockAgent {
    /// A mock that always returns `reply`.
    #[must_use]
    pub fn reply(reply: impl Into<String>) -> Self {
        Self {
            default_name: "mock".into(),
            default_usage: Usage::zero(),
            working_dir: None,
            mode: MockMode::Fixed {
                reply: reply.into(),
                fail: false,
            },
        }
    }

    /// A mock that always fails with `reason`.
    #[must_use]
    pub fn fail_with(reason: impl Into<String>) -> Self {
        Self {
            default_name: "mock_fail".into(),
            default_usage: Usage::zero(),
            working_dir: None,
            mode: MockMode::Fixed {
                reply: reason.into(),
                fail: true,
            },
        }
    }

    /// A mock that replays a fixed sequence of reply texts.
    ///
    /// Once the scripted turns are exhausted, the last turn is repeated.
    #[must_use]
    pub fn scripted<T, I>(replies: I) -> Self
    where
        T: Into<String>,
        I: IntoIterator<Item = T>,
    {
        let turns = replies
            .into_iter()
            .map(|reply| MockFixtureTurn {
                reply: reply.into(),
                fail: false,
                usage: None,
                name: None,
                writes: Vec::new(),
            })
            .collect::<Vec<_>>();
        Self {
            default_name: "mock_scripted".into(),
            default_usage: Usage::zero(),
            working_dir: None,
            mode: MockMode::Scripted {
                turns: ensure_non_empty(turns),
                cursor: ScriptCursor::InMemory(AtomicUsize::new(0)),
            },
        }
    }

    /// Load a scripted mock from `crates/roko-agent/testdata/mock-dispatcher/<fixture>`.
    ///
    /// The fixture directory must contain one or more `turn-*.json` files.
    /// Each file is a JSON object with at least a `reply` field.
    #[must_use]
    pub fn scripted_from_fixture(fixture: impl AsRef<Path>) -> Result<Self, MockFixtureError> {
        let path = fixture_root().join(fixture.as_ref());
        Self::scripted_from_dir(path)
    }

    /// Load a scripted mock from the given directory.
    #[must_use]
    pub fn scripted_from_dir(path: impl AsRef<Path>) -> Result<Self, MockFixtureError> {
        let path = path.as_ref();
        let turns = load_scripted_turns(path)?;
        Ok(Self {
            default_name: path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("mock_scripted")
                .to_string(),
            default_usage: Usage::zero(),
            working_dir: None,
            mode: MockMode::Scripted {
                turns,
                cursor: ScriptCursor::InMemory(AtomicUsize::new(0)),
            },
        })
    }

    /// Pre-set usage metrics that the mock will report.
    #[must_use]
    pub const fn with_usage(mut self, usage: Usage) -> Self {
        self.default_usage = usage;
        self
    }

    /// Override the mock's display name.
    #[must_use]
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.default_name = name.into();
        self
    }

    /// Root scripted file writes in the given working directory.
    #[must_use]
    pub fn with_working_dir(mut self, working_dir: impl Into<PathBuf>) -> Self {
        self.working_dir = Some(working_dir.into());
        self
    }

    /// Persist scripted-turn progress to a state file so the fixture can span
    /// multiple CLI processes in an end-to-end test.
    #[must_use]
    pub fn with_state_path(mut self, state_path: impl Into<PathBuf>) -> Self {
        if let MockMode::Scripted { cursor, .. } = &mut self.mode {
            *cursor = ScriptCursor::Persistent(state_path.into());
        }
        self
    }
}

#[async_trait]
impl Agent for MockAgent {
    async fn run(&self, input: &Signal, _ctx: &Context) -> AgentResult {
        let (reply, fail, usage, name) = match &self.mode {
            MockMode::Fixed { reply, fail } => (
                reply.clone(),
                *fail,
                self.default_usage,
                self.default_name.clone(),
            ),
            MockMode::Scripted { turns, cursor } => {
                let idx = cursor.next_index();
                let repeated = idx >= turns.len();
                let turn = turns
                    .get(idx)
                    .or_else(|| turns.last())
                    .expect("scripted mock must have at least one turn");
                self.apply_writes(&turn.writes);
                let reply = if repeated {
                    format!("{} [mock-turn:{}]", turn.reply, idx + 1)
                } else {
                    turn.reply.clone()
                };
                let name = turn
                    .name
                    .clone()
                    .unwrap_or_else(|| self.default_name.clone());
                (
                    reply,
                    turn.fail,
                    turn.usage.unwrap_or(self.default_usage),
                    name,
                )
            }
        };

        let output = input
            .derive(Kind::AgentOutput, Body::text(&reply))
            .provenance(Provenance::agent(&name))
            .tag("agent", &name)
            .build();
        let r = AgentResult::ok(output).with_usage(usage);
        if fail {
            AgentResult {
                success: false,
                ..r
            }
        } else {
            r
        }
    }

    fn name(&self) -> &str {
        &self.default_name
    }
}

impl ScriptCursor {
    fn next_index(&self) -> usize {
        match self {
            Self::InMemory(cursor) => cursor.fetch_add(1, Ordering::SeqCst),
            Self::Persistent(path) => next_persistent_index(path),
        }
    }
}

impl MockAgent {
    fn apply_writes(&self, writes: &[MockFixtureWrite]) {
        let Some(root) = self.working_dir.as_ref() else {
            return;
        };

        for file_write in writes {
            let path = if Path::new(&file_write.path).is_absolute() {
                PathBuf::from(&file_write.path)
            } else {
                root.join(&file_write.path)
            };
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).unwrap_or_else(|err| {
                    panic!("create mock fixture parent {}: {err}", parent.display())
                });
            }
            fs::write(&path, &file_write.content)
                .unwrap_or_else(|err| panic!("write mock fixture file {}: {err}", path.display()));
        }
    }
}

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("testdata")
        .join("mock-dispatcher")
}

fn next_persistent_index(path: &Path) -> usize {
    let current = fs::read_to_string(path)
        .ok()
        .and_then(|text| text.trim().parse::<usize>().ok())
        .unwrap_or(0);

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap_or_else(|err| {
            panic!("create mock fixture state dir {}: {err}", parent.display())
        });
    }
    fs::write(path, current.saturating_add(1).to_string())
        .unwrap_or_else(|err| panic!("write mock fixture state {}: {err}", path.display()));
    current
}

fn load_scripted_turns(path: &Path) -> Result<Vec<MockFixtureTurn>, MockFixtureError> {
    let mut entries = fs::read_dir(path)
        .map_err(|source| MockFixtureError::ReadDir {
            path: path.to_path_buf(),
            source,
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| MockFixtureError::ReadDir {
            path: path.to_path_buf(),
            source,
        })?;

    entries.sort_by(|a, b| a.file_name().cmp(&b.file_name()));

    let mut turns = Vec::new();
    for entry in entries {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }

        let raw = fs::read_to_string(&path).map_err(|source| MockFixtureError::ReadFile {
            path: path.clone(),
            source,
        })?;
        let turn = serde_json::from_str::<MockFixtureTurn>(&raw).map_err(|source| {
            MockFixtureError::ParseFile {
                path: path.clone(),
                source,
            }
        })?;
        turns.push(turn);
    }

    if turns.is_empty() {
        return Err(MockFixtureError::Empty {
            path: path.to_path_buf(),
        });
    }

    Ok(turns)
}

fn ensure_non_empty(turns: Vec<MockFixtureTurn>) -> Vec<MockFixtureTurn> {
    if turns.is_empty() {
        vec![MockFixtureTurn {
            reply: String::new(),
            fail: false,
            usage: None,
            name: None,
            writes: Vec::new(),
        }]
    } else {
        turns
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prompt(text: &str) -> Signal {
        Signal::builder(Kind::Prompt).body(Body::text(text)).build()
    }

    #[tokio::test]
    async fn reply_returns_canned_text() {
        let agent = MockAgent::reply("hello from mock");
        let result = agent.run(&prompt("hi"), &Context::at(0)).await;
        assert!(result.success);
        assert_eq!(result.output.body.as_text().unwrap(), "hello from mock");
        assert_eq!(result.output.kind, Kind::AgentOutput);
    }

    #[tokio::test]
    async fn output_tracks_input_as_lineage() {
        let agent = MockAgent::reply("ok");
        let input = prompt("do X");
        let input_id = input.id;
        let result = agent.run(&input, &Context::at(0)).await;
        assert_eq!(result.output.lineage, vec![input_id]);
    }

    #[tokio::test]
    async fn fail_with_sets_success_false() {
        let agent = MockAgent::fail_with("bang");
        let result = agent.run(&prompt("x"), &Context::at(0)).await;
        assert!(!result.success);
    }

    #[tokio::test]
    async fn usage_is_reported() {
        let agent = MockAgent::reply("x").with_usage(Usage {
            input_tokens: 42,
            output_tokens: 17,
            ..Default::default()
        });
        let r = agent.run(&prompt("x"), &Context::at(0)).await;
        assert_eq!(r.usage.input_tokens, 42);
        assert_eq!(r.usage.output_tokens, 17);
    }

    #[tokio::test]
    async fn output_is_tagged_with_agent_name() {
        let agent = MockAgent::reply("x").with_name("my_mock");
        let r = agent.run(&prompt("x"), &Context::at(0)).await;
        assert_eq!(r.output.tag("agent"), Some("my_mock"));
    }

    #[tokio::test]
    async fn scripted_fixture_replays_turns_in_order() {
        let agent = MockAgent::scripted_from_fixture("self-host-fixture").unwrap();
        let first = agent.run(&prompt("one"), &Context::at(0)).await;
        let second = agent.run(&prompt("two"), &Context::at(0)).await;
        let third = agent.run(&prompt("three"), &Context::at(0)).await;
        let fourth = agent.run(&prompt("four"), &Context::at(0)).await;
        let fifth = agent.run(&prompt("five"), &Context::at(0)).await;

        assert!(
            first
                .output
                .body
                .as_text()
                .unwrap()
                .contains("status: draft")
        );
        assert!(second.output.body.as_text().unwrap().contains("Research"));
        assert!(
            third
                .output
                .body
                .as_text()
                .unwrap()
                .contains("Plan artifacts")
        );
        assert_eq!(
            fourth.output.body.as_text().unwrap(),
            "{\"outcome\":\"passed\",\"task_id\":\"T1\",\"summary\":\"Implemented the single smoke-test task and left the sample Rust project ready for cargo check.\",\"evidence_refs\":[\"Cargo.toml\",\"src/main.rs\"]}"
        );
        assert!(
            fifth
                .output
                .body
                .as_text()
                .unwrap()
                .contains("\"status\":\"passed\"")
        );
        assert_eq!(agent.name(), "self-host-fixture");
    }

    #[tokio::test]
    async fn scripted_fixture_repeated_tail_turns_stay_unique() {
        let agent = MockAgent::scripted_from_fixture("self-host-fixture").unwrap();
        for prompt_text in [
            "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten",
        ] {
            let _ = agent.run(&prompt(prompt_text), &Context::at(0)).await;
        }

        let eleventh = agent.run(&prompt("eleven"), &Context::at(0)).await;
        let twelfth = agent.run(&prompt("twelve"), &Context::at(0)).await;
        let eleventh_body = eleventh.output.body.as_text().unwrap();
        let twelfth_body = twelfth.output.body.as_text().unwrap();

        assert!(eleventh_body.contains("Finalized the mock self-hosting task"));
        assert!(twelfth_body.contains("Finalized the mock self-hosting task"));
        assert_ne!(eleventh_body, twelfth_body);
    }

    #[tokio::test]
    async fn scripted_fixture_applies_turn_specific_metadata() {
        let agent = MockAgent::scripted_from_fixture("self-host-fixture").unwrap();
        let r = agent.run(&prompt("one"), &Context::at(0)).await;
        assert_eq!(r.output.tag("agent"), Some("self-host-fixture-turn-01"));
        assert_eq!(r.usage.input_tokens, 13);
        assert_eq!(r.usage.output_tokens, 24);
    }

    #[tokio::test]
    async fn scripted_fixture_persists_turn_progress_across_agents() {
        let dir = tempfile::tempdir().unwrap();
        let state_path = dir.path().join("mock-state.txt");

        let first = MockAgent::scripted_from_fixture("self-host-fixture")
            .unwrap()
            .with_state_path(&state_path);
        let second = MockAgent::scripted_from_fixture("self-host-fixture")
            .unwrap()
            .with_state_path(&state_path);

        let one = first.run(&prompt("one"), &Context::at(0)).await;
        let two = second.run(&prompt("two"), &Context::at(0)).await;

        assert!(one.output.body.as_text().unwrap().contains("status: draft"));
        assert!(two.output.body.as_text().unwrap().contains("Research"));
    }
}
