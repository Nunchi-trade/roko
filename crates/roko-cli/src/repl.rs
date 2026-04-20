//! REPL mode — interactive read-eval-print loop.
//!
//! Activated when stdin is a TTY and no positional prompt argument is given.
//! Reads lines from stdin, processes built-in commands (`:quit`, `:help`,
//! `:status`), slash commands (`/plan`, `/explain`, `/replay`, etc.), and
//! dispatches everything else as a prompt through the universal loop.

use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

/// Built-in REPL commands (prefixed with `:`) and slash commands (prefixed with `/`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplCommand {
    /// Exit the REPL.
    Quit,
    /// Print available commands.
    Help,
    /// Show session status.
    Status,
    /// Send a prompt to the agent.
    Prompt(String),
    /// `/plan [args...]` — plan management.
    SlashPlan(Vec<String>),
    /// `/explain <topic>` — progressive topic help.
    SlashExplain(String),
    /// `/replay <hash>` — walk signal DAG.
    SlashReplay(String),
    /// `/status` — show workspace status (same as `:status` but also shows workspace info).
    SlashStatus,
    /// `/run <prompt>` — run a one-shot prompt.
    SlashRun(String),
    /// `/research <topic>` — research a topic.
    SlashResearch(String),
    /// `/learn [what]` — show learning subsystem state.
    SlashLearn(String),
    /// `/tune [subsystem]` — show adaptive thresholds.
    SlashTune(String),
}

/// Workspace context discovered on REPL startup.
#[derive(Debug, Clone, Default)]
pub struct WorkspaceContext {
    /// Path to the discovered `.roko/` directory, if any.
    pub roko_dir: Option<PathBuf>,
    /// Recent PRD slugs found in `.roko/prd/`.
    pub recent_prds: Vec<String>,
    /// Recent plan directories found in `plans/` or `.roko/plans/`.
    pub recent_plans: Vec<String>,
    /// Path to an interrupted executor snapshot, if any.
    pub interrupted_snapshot: Option<PathBuf>,
}

impl WorkspaceContext {
    /// Discover workspace context by searching upward from `start` for `.roko/`.
    pub fn discover(start: &Path) -> Self {
        let mut ctx = Self::default();

        // Walk upward to find .roko/
        let mut dir = start.to_path_buf();
        loop {
            let candidate = dir.join(".roko");
            if candidate.is_dir() {
                ctx.roko_dir = Some(candidate.clone());

                // Enumerate recent PRDs (up to 3)
                let prd_dir = candidate.join("prd");
                if prd_dir.is_dir() {
                    if let Ok(entries) = std::fs::read_dir(&prd_dir) {
                        let mut prds: Vec<_> = entries
                            .filter_map(|e| e.ok())
                            .filter(|e| e.path().is_dir())
                            .filter_map(|e| {
                                let name = e.file_name().to_string_lossy().to_string();
                                let mtime = e.metadata().ok()?.modified().ok()?;
                                Some((name, mtime))
                            })
                            .collect();
                        prds.sort_by(|a, b| b.1.cmp(&a.1));
                        ctx.recent_prds = prds.into_iter().take(3).map(|(n, _)| n).collect();
                    }
                }

                // Check for interrupted executor snapshot
                let snapshot = candidate.join("state").join("executor.json");
                if snapshot.is_file() {
                    ctx.interrupted_snapshot = Some(snapshot);
                }

                // Enumerate recent plans (up to 3)
                let plans_dir = dir.join("plans");
                if plans_dir.is_dir() {
                    if let Ok(entries) = std::fs::read_dir(&plans_dir) {
                        let mut plans: Vec<_> = entries
                            .filter_map(|e| e.ok())
                            .filter(|e| e.path().is_dir())
                            .filter_map(|e| {
                                let name = e.file_name().to_string_lossy().to_string();
                                let mtime = e.metadata().ok()?.modified().ok()?;
                                Some((name, mtime))
                            })
                            .collect();
                        plans.sort_by(|a, b| b.1.cmp(&a.1));
                        ctx.recent_plans = plans.into_iter().take(3).map(|(n, _)| n).collect();
                    }
                }

                break;
            }
            if !dir.pop() {
                break;
            }
        }

        ctx
    }

    /// Render the workspace banner section.
    pub fn render_banner(&self, writer: &mut impl Write) -> io::Result<()> {
        if let Some(roko_dir) = &self.roko_dir {
            let workspace = roko_dir
                .parent()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| ".".to_string());
            writeln!(writer, "workspace: {workspace}")?;
        } else {
            writeln!(
                writer,
                "no .roko/ workspace found (run `roko init` to create one)"
            )?;
        }

        if !self.recent_prds.is_empty() {
            writeln!(writer, "recent PRDs: {}", self.recent_prds.join(", "))?;
        }

        if !self.recent_plans.is_empty() {
            writeln!(writer, "recent plans: {}", self.recent_plans.join(", "))?;
        }

        if let Some(snapshot) = &self.interrupted_snapshot {
            writeln!(writer, "interrupted session found: {}", snapshot.display())?;
            writeln!(
                writer,
                "  resume with: roko plan run plans/ --resume {}",
                snapshot.display()
            )?;
        }

        Ok(())
    }
}

/// Interactive REPL session state.
#[derive(Debug)]
pub struct ReplMode {
    /// Session identifier for this REPL session.
    pub session_id: String,
    /// Number of prompts processed so far.
    pub prompt_count: usize,
    /// Whether the REPL is still running.
    running: bool,
    /// Discovered workspace context.
    pub workspace: WorkspaceContext,
}

impl ReplMode {
    /// Create a new REPL session with the given session ID.
    #[must_use]
    pub fn new(session_id: String) -> Self {
        let workspace = WorkspaceContext::discover(
            &std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        );
        Self {
            session_id,
            prompt_count: 0,
            running: false,
            workspace,
        }
    }

    /// Create a new REPL session with explicit workspace context.
    #[must_use]
    pub fn new_with_workspace(session_id: String, workspace: WorkspaceContext) -> Self {
        Self {
            session_id,
            prompt_count: 0,
            running: false,
            workspace,
        }
    }

    /// Parse a line of user input into a REPL command.
    #[must_use]
    pub fn parse_input(line: &str) -> ReplCommand {
        let trimmed = line.trim();

        // Colon-prefixed built-ins
        match trimmed {
            ":quit" | ":q" | ":exit" => return ReplCommand::Quit,
            ":help" | ":h" | ":?" => return ReplCommand::Help,
            ":status" | ":s" => return ReplCommand::Status,
            _ => {}
        }

        // Slash commands
        if trimmed.starts_with('/') {
            let rest = &trimmed[1..];
            let mut parts = rest.splitn(2, char::is_whitespace);
            let verb = parts.next().unwrap_or("");
            let args_str = parts.next().unwrap_or("").trim();

            return match verb {
                "plan" | "p" => {
                    let args: Vec<String> = if args_str.is_empty() {
                        vec![]
                    } else {
                        args_str.split_whitespace().map(String::from).collect()
                    };
                    ReplCommand::SlashPlan(args)
                }
                "explain" | "e" => ReplCommand::SlashExplain(args_str.to_string()),
                "replay" | "inspect" => ReplCommand::SlashReplay(args_str.to_string()),
                "status" => ReplCommand::SlashStatus,
                "run" | "do" => ReplCommand::SlashRun(args_str.to_string()),
                "research" => ReplCommand::SlashResearch(args_str.to_string()),
                "learn" | "ask" => {
                    ReplCommand::SlashLearn(if args_str.is_empty() {
                        "all".to_string()
                    } else {
                        args_str.to_string()
                    })
                }
                "tune" => ReplCommand::SlashTune(if args_str.is_empty() {
                    "gates".to_string()
                } else {
                    args_str.to_string()
                }),
                "help" | "h" | "?" => ReplCommand::Help,
                "quit" | "q" | "exit" => ReplCommand::Quit,
                _ => {
                    // Unknown slash command — treat as prompt
                    ReplCommand::Prompt(trimmed.to_string())
                }
            };
        }

        ReplCommand::Prompt(trimmed.to_string())
    }

    /// Run the REPL loop, reading from the provided reader and writing to the
    /// provided writer. Returns the commands collected during the session.
    ///
    /// Slash commands are dispatched inline (e.g. `/explain` prints topic info).
    /// `Prompt` values are collected for the caller to wire into the universal loop.
    pub fn run<R: BufRead, W: Write>(
        &mut self,
        reader: &mut R,
        writer: &mut W,
    ) -> io::Result<Vec<ReplCommand>> {
        self.running = true;
        let mut commands = Vec::new();

        writeln!(writer, "roko repl (session: {})", self.session_id)?;
        self.workspace.render_banner(writer)?;
        writeln!(writer, "type :help for commands, :quit to exit")?;

        loop {
            if !self.running {
                break;
            }
            write!(writer, "roko> ")?;
            writer.flush()?;

            let mut line = String::new();
            let bytes = reader.read_line(&mut line)?;
            if bytes == 0 {
                // EOF
                break;
            }

            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            let cmd = Self::parse_input(trimmed);
            match &cmd {
                ReplCommand::Quit => {
                    writeln!(
                        writer,
                        "goodbye ({} prompt(s) processed)",
                        self.prompt_count
                    )?;
                    self.running = false;
                    commands.push(cmd);
                    break;
                }
                ReplCommand::Help => {
                    writeln!(writer, "built-in commands:")?;
                    writeln!(writer, "  :help, :h, :?       show this help")?;
                    writeln!(writer, "  :status, :s          show session status")?;
                    writeln!(writer, "  :quit, :q, :exit     exit the REPL")?;
                    writeln!(writer)?;
                    writeln!(writer, "slash commands:")?;
                    writeln!(writer, "  /plan [args]         plan management (list, show, run)")?;
                    writeln!(writer, "  /explain <topic>     progressive topic help")?;
                    writeln!(writer, "  /replay <hash>       walk signal DAG by hash")?;
                    writeln!(writer, "  /run <prompt>        run a one-shot prompt")?;
                    writeln!(writer, "  /research <topic>    research a topic")?;
                    writeln!(writer, "  /learn [what]        show learning state")?;
                    writeln!(writer, "  /tune [subsystem]    show adaptive thresholds")?;
                    writeln!(writer, "  /status              show workspace status")?;
                    writeln!(writer)?;
                    writeln!(writer, "  <text>               send prompt to agent")?;
                    commands.push(cmd);
                }
                ReplCommand::Status => {
                    writeln!(writer, "session : {}", self.session_id)?;
                    writeln!(writer, "prompts : {}", self.prompt_count)?;
                    commands.push(cmd);
                }
                ReplCommand::SlashExplain(topic) => {
                    if topic.is_empty() {
                        writeln!(writer, "usage: /explain <topic>")?;
                        writeln!(
                            writer,
                            "topics: {}",
                            crate::explain::topic_names().join(", ")
                        )?;
                    } else {
                        // Parse optional depth from args: "/explain gates 2"
                        let mut parts = topic.splitn(2, char::is_whitespace);
                        let topic_name = parts.next().unwrap_or("");
                        let depth: u8 = parts
                            .next()
                            .and_then(|s| s.trim().parse().ok())
                            .unwrap_or(1);
                        match crate::explain::find_topic(topic_name) {
                            Some(entry) => {
                                write!(
                                    writer,
                                    "{}",
                                    crate::explain::render_topic(entry, depth.clamp(1, 3))
                                )?;
                            }
                            None => {
                                writeln!(writer, "unknown topic: {topic_name}")?;
                                writeln!(
                                    writer,
                                    "available: {}",
                                    crate::explain::topic_names().join(", ")
                                )?;
                            }
                        }
                    }
                    commands.push(cmd);
                }
                ReplCommand::SlashStatus => {
                    writeln!(writer, "session : {}", self.session_id)?;
                    writeln!(writer, "prompts : {}", self.prompt_count)?;
                    self.workspace.render_banner(writer)?;
                    commands.push(cmd);
                }
                ReplCommand::SlashPlan(args) => {
                    if args.is_empty() {
                        writeln!(writer, "[/plan] hint: use `roko plan list` or `roko plan run <dir>`")?;
                        if !self.workspace.recent_plans.is_empty() {
                            writeln!(
                                writer,
                                "recent plans: {}",
                                self.workspace.recent_plans.join(", ")
                            )?;
                        }
                    } else {
                        writeln!(
                            writer,
                            "[/plan {}] dispatching to plan handler...",
                            args.join(" ")
                        )?;
                    }
                    commands.push(cmd);
                }
                ReplCommand::SlashReplay(hash) => {
                    if hash.is_empty() {
                        writeln!(writer, "usage: /replay <hash>")?;
                    } else {
                        writeln!(writer, "[/replay] walking DAG from {hash}...")?;
                    }
                    commands.push(cmd);
                }
                ReplCommand::SlashRun(prompt) => {
                    if prompt.is_empty() {
                        writeln!(writer, "usage: /run <prompt>")?;
                    } else {
                        self.prompt_count += 1;
                        writeln!(writer, "[/run] dispatching prompt: {prompt}")?;
                    }
                    commands.push(cmd);
                }
                ReplCommand::SlashResearch(topic) => {
                    if topic.is_empty() {
                        writeln!(writer, "usage: /research <topic>")?;
                    } else {
                        writeln!(writer, "[/research] researching: {topic}")?;
                    }
                    commands.push(cmd);
                }
                ReplCommand::SlashLearn(what) => {
                    writeln!(writer, "[/learn] showing learning state: {what}")?;
                    commands.push(cmd);
                }
                ReplCommand::SlashTune(subsystem) => {
                    writeln!(writer, "[/tune] showing thresholds for: {subsystem}")?;
                    commands.push(cmd);
                }
                ReplCommand::Prompt(text) => {
                    if text.is_empty() {
                        continue;
                    }
                    self.prompt_count += 1;
                    writeln!(writer, "[prompt {}] {}", self.prompt_count, text)?;
                    commands.push(cmd);
                }
            }
        }

        Ok(commands)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_quit_variants() {
        assert_eq!(ReplMode::parse_input(":quit"), ReplCommand::Quit);
        assert_eq!(ReplMode::parse_input(":q"), ReplCommand::Quit);
        assert_eq!(ReplMode::parse_input(":exit"), ReplCommand::Quit);
    }

    #[test]
    fn parse_help_variants() {
        assert_eq!(ReplMode::parse_input(":help"), ReplCommand::Help);
        assert_eq!(ReplMode::parse_input(":h"), ReplCommand::Help);
        assert_eq!(ReplMode::parse_input(":?"), ReplCommand::Help);
    }

    #[test]
    fn parse_status() {
        assert_eq!(ReplMode::parse_input(":status"), ReplCommand::Status);
        assert_eq!(ReplMode::parse_input(":s"), ReplCommand::Status);
    }

    #[test]
    fn parse_prompt() {
        assert_eq!(
            ReplMode::parse_input("hello world"),
            ReplCommand::Prompt("hello world".to_string())
        );
    }

    #[test]
    fn parse_trims_whitespace() {
        assert_eq!(ReplMode::parse_input("  :quit  "), ReplCommand::Quit);
        assert_eq!(
            ReplMode::parse_input("  some prompt  "),
            ReplCommand::Prompt("some prompt".to_string())
        );
    }

    #[test]
    fn parse_slash_explain() {
        assert_eq!(
            ReplMode::parse_input("/explain gates"),
            ReplCommand::SlashExplain("gates".to_string())
        );
    }

    #[test]
    fn parse_slash_explain_alias() {
        assert_eq!(
            ReplMode::parse_input("/e routing"),
            ReplCommand::SlashExplain("routing".to_string())
        );
    }

    #[test]
    fn parse_slash_plan_no_args() {
        assert_eq!(
            ReplMode::parse_input("/plan"),
            ReplCommand::SlashPlan(vec![])
        );
    }

    #[test]
    fn parse_slash_plan_with_args() {
        assert_eq!(
            ReplMode::parse_input("/plan list"),
            ReplCommand::SlashPlan(vec!["list".to_string()])
        );
        assert_eq!(
            ReplMode::parse_input("/plan run plans/"),
            ReplCommand::SlashPlan(vec!["run".to_string(), "plans/".to_string()])
        );
    }

    #[test]
    fn parse_slash_replay() {
        assert_eq!(
            ReplMode::parse_input("/replay abc123"),
            ReplCommand::SlashReplay("abc123".to_string())
        );
        assert_eq!(
            ReplMode::parse_input("/inspect abc123"),
            ReplCommand::SlashReplay("abc123".to_string())
        );
    }

    #[test]
    fn parse_slash_status() {
        assert_eq!(ReplMode::parse_input("/status"), ReplCommand::SlashStatus);
    }

    #[test]
    fn parse_slash_run() {
        assert_eq!(
            ReplMode::parse_input("/run fix the tests"),
            ReplCommand::SlashRun("fix the tests".to_string())
        );
        assert_eq!(
            ReplMode::parse_input("/do fix the tests"),
            ReplCommand::SlashRun("fix the tests".to_string())
        );
    }

    #[test]
    fn parse_slash_research() {
        assert_eq!(
            ReplMode::parse_input("/research rust async patterns"),
            ReplCommand::SlashResearch("rust async patterns".to_string())
        );
    }

    #[test]
    fn parse_slash_learn() {
        assert_eq!(
            ReplMode::parse_input("/learn"),
            ReplCommand::SlashLearn("all".to_string())
        );
        assert_eq!(
            ReplMode::parse_input("/learn router"),
            ReplCommand::SlashLearn("router".to_string())
        );
        assert_eq!(
            ReplMode::parse_input("/ask episodes"),
            ReplCommand::SlashLearn("episodes".to_string())
        );
    }

    #[test]
    fn parse_slash_tune() {
        assert_eq!(
            ReplMode::parse_input("/tune"),
            ReplCommand::SlashTune("gates".to_string())
        );
        assert_eq!(
            ReplMode::parse_input("/tune routing"),
            ReplCommand::SlashTune("routing".to_string())
        );
    }

    #[test]
    fn parse_slash_help() {
        assert_eq!(ReplMode::parse_input("/help"), ReplCommand::Help);
        assert_eq!(ReplMode::parse_input("/h"), ReplCommand::Help);
        assert_eq!(ReplMode::parse_input("/?"), ReplCommand::Help);
    }

    #[test]
    fn parse_slash_quit() {
        assert_eq!(ReplMode::parse_input("/quit"), ReplCommand::Quit);
        assert_eq!(ReplMode::parse_input("/q"), ReplCommand::Quit);
        assert_eq!(ReplMode::parse_input("/exit"), ReplCommand::Quit);
    }

    #[test]
    fn parse_unknown_slash_becomes_prompt() {
        assert_eq!(
            ReplMode::parse_input("/unknown something"),
            ReplCommand::Prompt("/unknown something".to_string())
        );
    }

    #[test]
    fn repl_processes_commands() {
        let input = ":help\n:status\nhello agent\n:quit\n";
        let mut reader = io::Cursor::new(input);
        let mut output = Vec::new();
        let mut repl =
            ReplMode::new_with_workspace("test-session".into(), WorkspaceContext::default());

        let commands = repl.run(&mut reader, &mut output).unwrap();

        assert_eq!(commands.len(), 4);
        assert_eq!(commands[0], ReplCommand::Help);
        assert_eq!(commands[1], ReplCommand::Status);
        assert_eq!(commands[2], ReplCommand::Prompt("hello agent".to_string()));
        assert_eq!(commands[3], ReplCommand::Quit);
        assert_eq!(repl.prompt_count, 1);
    }

    #[test]
    fn repl_handles_eof() {
        let input = "first prompt\nsecond prompt\n";
        let mut reader = io::Cursor::new(input);
        let mut output = Vec::new();
        let mut repl =
            ReplMode::new_with_workspace("eof-test".into(), WorkspaceContext::default());

        let commands = repl.run(&mut reader, &mut output).unwrap();

        assert_eq!(commands.len(), 2);
        assert_eq!(repl.prompt_count, 2);
    }

    #[test]
    fn repl_skips_blank_lines() {
        let input = "\n\n  \nhello\n:quit\n";
        let mut reader = io::Cursor::new(input);
        let mut output = Vec::new();
        let mut repl =
            ReplMode::new_with_workspace("blank-test".into(), WorkspaceContext::default());

        let commands = repl.run(&mut reader, &mut output).unwrap();

        assert_eq!(commands.len(), 2);
        assert_eq!(commands[0], ReplCommand::Prompt("hello".to_string()));
        assert_eq!(commands[1], ReplCommand::Quit);
    }

    #[test]
    fn repl_output_contains_banner() {
        let input = ":quit\n";
        let mut reader = io::Cursor::new(input);
        let mut output = Vec::new();
        let mut repl =
            ReplMode::new_with_workspace("banner-test".into(), WorkspaceContext::default());

        repl.run(&mut reader, &mut output).unwrap();

        let out_str = String::from_utf8(output).unwrap();
        assert!(out_str.contains("roko repl"));
        assert!(out_str.contains("banner-test"));
        assert!(out_str.contains("goodbye"));
    }

    #[test]
    fn repl_slash_explain_in_session() {
        let input = "/explain gates\n:quit\n";
        let mut reader = io::Cursor::new(input);
        let mut output = Vec::new();
        let mut repl =
            ReplMode::new_with_workspace("explain-test".into(), WorkspaceContext::default());

        let commands = repl.run(&mut reader, &mut output).unwrap();

        assert_eq!(commands.len(), 2);
        assert_eq!(
            commands[0],
            ReplCommand::SlashExplain("gates".to_string())
        );
        let out_str = String::from_utf8(output).unwrap();
        assert!(out_str.contains("Gate Pipeline"));
    }

    #[test]
    fn repl_slash_status_shows_workspace() {
        let input = "/status\n:quit\n";
        let mut reader = io::Cursor::new(input);
        let mut output = Vec::new();
        let ws = WorkspaceContext {
            roko_dir: Some(PathBuf::from("/tmp/test/.roko")),
            recent_prds: vec!["my-feature".to_string()],
            recent_plans: vec![],
            interrupted_snapshot: None,
        };
        let mut repl = ReplMode::new_with_workspace("ws-test".into(), ws);

        let commands = repl.run(&mut reader, &mut output).unwrap();

        assert_eq!(commands[0], ReplCommand::SlashStatus);
        let out_str = String::from_utf8(output).unwrap();
        assert!(out_str.contains("/tmp/test"));
        assert!(out_str.contains("my-feature"));
    }

    #[test]
    fn workspace_discover_returns_default_for_nonexistent() {
        let ctx = WorkspaceContext::discover(Path::new("/nonexistent/path"));
        assert!(ctx.roko_dir.is_none());
        assert!(ctx.recent_prds.is_empty());
        assert!(ctx.interrupted_snapshot.is_none());
    }

    #[test]
    fn workspace_banner_no_workspace() {
        let ctx = WorkspaceContext::default();
        let mut out = Vec::new();
        ctx.render_banner(&mut out).unwrap();
        let s = String::from_utf8(out).unwrap();
        assert!(s.contains("no .roko/ workspace found"));
    }

    #[test]
    fn workspace_banner_with_workspace() {
        let ctx = WorkspaceContext {
            roko_dir: Some(PathBuf::from("/home/user/project/.roko")),
            recent_prds: vec!["auth-system".to_string(), "logging".to_string()],
            recent_plans: vec!["sprint-1".to_string()],
            interrupted_snapshot: None,
        };
        let mut out = Vec::new();
        ctx.render_banner(&mut out).unwrap();
        let s = String::from_utf8(out).unwrap();
        assert!(s.contains("/home/user/project"));
        assert!(s.contains("auth-system, logging"));
        assert!(s.contains("sprint-1"));
    }

    #[test]
    fn workspace_banner_with_interrupted_snapshot() {
        let ctx = WorkspaceContext {
            roko_dir: Some(PathBuf::from("/tmp/.roko")),
            recent_prds: vec![],
            recent_plans: vec![],
            interrupted_snapshot: Some(PathBuf::from("/tmp/.roko/state/executor.json")),
        };
        let mut out = Vec::new();
        ctx.render_banner(&mut out).unwrap();
        let s = String::from_utf8(out).unwrap();
        assert!(s.contains("interrupted session found"));
        assert!(s.contains("--resume"));
    }
}
