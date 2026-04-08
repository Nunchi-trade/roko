//! `roko research` — auto-research to enhance PRDs, plans, and tasks.
//!
//! Provides agent-driven research capabilities:
//! - Deep-dive into a topic with academic citations
//! - Enhance existing PRDs with research findings
//! - Optimize task decomposition based on latest techniques
//! - Analyze execution data for self-learning insights
//!
//! Research artifacts live in `.roko/research/` as markdown files.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::Result;

fn research_dir(workdir: &Path) -> PathBuf {
    workdir.join(".roko").join("research")
}

/// Ensure the research directory exists.
pub fn ensure_dirs(workdir: &Path) -> Result<()> {
    std::fs::create_dir_all(research_dir(workdir))?;
    Ok(())
}

/// The system prompt for research agents.
pub const RESEARCH_SYSTEM_PROMPT: &str = r#"You are a technical research analyst. Your job is to find, synthesize, and apply academic research and industry best practices to improve software engineering artifacts.

## Research standards

1. **Academic rigor**: Cite real papers with full author, title, venue, year. Use [AUTHOR-YEAR] format. Verify papers exist — do not hallucinate citations.
2. **Practical relevance**: Every finding must connect to a concrete implementation recommendation. "Interesting but not actionable" is not useful.
3. **Recency bias**: Prefer 2023-2026 papers. Older foundational work (pre-2020) only when it's the canonical reference.
4. **Breadth**: Search across: agent scaffolding, code generation, task decomposition, context engineering, multi-agent systems, software testing, formal methods, and the specific domain of the topic.
5. **Contrarian findings**: Actively seek papers that challenge the current approach. "X doesn't work as well as claimed" is more valuable than "X confirms what we already believe."

## Research sources to check

- arXiv cs.SE, cs.AI, cs.CL, cs.MA (multi-agent systems)
- ACL, EMNLP, NeurIPS, ICML, ICLR, ISSTA, ICSE, FSE
- Anthropic research blog, OpenAI research, Google DeepMind
- SWE-bench leaderboard and papers
- HumanEval, MBPP, and code generation benchmarks
- Recent agent framework papers (LangChain, CrewAI, AutoGen, Magentic-One)

## Output format

Structure every research output as:

### Finding: [one-line summary]
**Source**: [AUTHOR-YEAR] full citation
**Relevance**: How this applies to the current artifact
**Recommendation**: Concrete change to make
**Confidence**: High / Medium / Low (based on evidence strength)

## When enhancing existing documents

1. Read the document fully first
2. Identify claims without citations — add them
3. Identify design decisions without justification — find supporting research
4. Identify potential improvements the author missed — propose them with citations
5. Check for contradictions with recent research — flag them
6. Add mermaid diagrams where they'd clarify architecture
"#;

/// Build the prompt for a research task.
#[allow(clippy::too_many_lines)]
pub fn build_research_prompt(
    workdir: &Path,
    topic: &str,
    context: &str,
    mode: ResearchMode,
) -> String {
    let mut prompt = String::new();
    let _ = writeln!(prompt, "{RESEARCH_SYSTEM_PROMPT}");
    let _ = writeln!(prompt, "\n---\n");
    let _ = writeln!(prompt, "## Workspace: {}\n", workdir.display());

    // Include master index so the agent knows what exists
    let master_index = std::fs::read_to_string(workdir.join(".roko/INDEX.md")).unwrap_or_default();
    if !master_index.is_empty() {
        let _ = writeln!(prompt, "## What already exists (do NOT duplicate)\n{master_index}\n---\n");
    }

    match mode {
        ResearchMode::Topic => {
            let _ = writeln!(prompt, "## Research task\n");
            let _ = writeln!(prompt, "Deep-dive research on: **{topic}**\n");
            let _ = writeln!(
                prompt,
                "Produce a research document with 10-20 findings, each with citation, relevance, and recommendation."
            );
            let _ = writeln!(
                prompt,
                "Save the output to .roko/research/{}.md",
                slug(topic)
            );
        }
        ResearchMode::EnhancePrd => {
            let _ = writeln!(prompt, "## Enhancement task\n");
            let _ = writeln!(prompt, "Read this PRD and enhance it with research:\n");
            let _ = writeln!(prompt, "{context}\n");
            let _ = writeln!(prompt, "For each section:");
            let _ = writeln!(
                prompt,
                "1. Add missing citations (find real papers that support design decisions)"
            );
            let _ = writeln!(
                prompt,
                "2. Add mermaid diagrams where architecture would be clearer"
            );
            let _ = writeln!(prompt, "3. Identify improvements from recent research");
            let _ = writeln!(prompt, "4. Flag any claims that contradict recent findings");
            let _ = writeln!(
                prompt,
                "\nUpdate the PRD file in place. Also save a research summary to .roko/research/{}.md",
                slug(topic)
            );
        }
        ResearchMode::EnhancePlan => {
            let _ = writeln!(prompt, "## Plan enhancement task\n");
            let _ = writeln!(prompt, "Read this implementation plan and optimize it:\n");
            let _ = writeln!(prompt, "{context}\n");
            let _ = writeln!(prompt, "Research and apply:");
            let _ = writeln!(
                prompt,
                "1. Better task decomposition strategies (cite SWE-bench, Agentless, etc.)"
            );
            let _ = writeln!(prompt, "2. More precise context injection techniques");
            let _ = writeln!(prompt, "3. Stronger verification approaches");
            let _ = writeln!(
                prompt,
                "4. Cost optimization (cheaper models for simple tasks)"
            );
            let _ = writeln!(prompt, "\nUpdate the plan files in place.");
        }
        ResearchMode::EnhanceTasks => {
            let _ = writeln!(prompt, "## Task optimization task\n");
            let _ = writeln!(
                prompt,
                "Read these tasks and optimize for maximum efficiency:\n"
            );
            let _ = writeln!(prompt, "{context}\n");
            let _ = writeln!(prompt, "For each task:");
            let _ = writeln!(
                prompt,
                "1. Can it be split into smaller Tier 0 (Haiku-capable) subtasks?"
            );
            let _ = writeln!(
                prompt,
                "2. Is the context surgical enough? Reduce to exact line ranges."
            );
            let _ = writeln!(
                prompt,
                "3. Are acceptance criteria truly machine-verifiable?"
            );
            let _ = writeln!(
                prompt,
                "4. Can parallelism be increased by removing unnecessary dependencies?"
            );
            let _ = writeln!(
                prompt,
                "5. What anti-patterns should be added from research on common agent failures?"
            );
            let _ = writeln!(prompt, "\nUpdate tasks.toml in place.");
        }
        ResearchMode::AnalyzeExecution => {
            let _ = writeln!(prompt, "## Execution analysis task\n");
            let _ = writeln!(
                prompt,
                "Analyze the execution data and identify optimization opportunities:\n"
            );
            let _ = writeln!(prompt, "{context}\n");
            let _ = writeln!(prompt, "Compute and report:");
            let _ = writeln!(
                prompt,
                "1. First-attempt pass rate (FAPR) by task tier and model"
            );
            let _ = writeln!(
                prompt,
                "2. Cost per task by tier — are we using expensive models for easy tasks?"
            );
            let _ = writeln!(prompt, "3. Retry patterns — what kinds of tasks fail most?");
            let _ = writeln!(prompt, "4. Context size vs success rate correlation");
            let _ = writeln!(
                prompt,
                "5. Recommendations: which bandit weights to adjust, which task types need better context"
            );
            let _ = writeln!(
                prompt,
                "\nSave analysis to .roko/research/execution-analysis-{}.md",
                chrono::Local::now().format("%Y%m%d")
            );
        }
    }

    if !context.is_empty()
        && !matches!(
            mode,
            ResearchMode::EnhancePrd
                | ResearchMode::EnhancePlan
                | ResearchMode::EnhanceTasks
                | ResearchMode::AnalyzeExecution
        )
    {
        let _ = writeln!(prompt, "\n## Additional context\n{context}");
    }

    prompt
}

/// Research mode determines what kind of output to produce.
#[derive(Debug, Clone, Copy)]
pub enum ResearchMode {
    /// Pure research on a topic → .roko/research/<slug>.md
    Topic,
    /// Enhance a PRD with research findings and citations
    EnhancePrd,
    /// Optimize an implementation plan with research-backed techniques
    EnhancePlan,
    /// Optimize tasks for efficiency, parallelism, and model selection
    EnhanceTasks,
    /// Analyze execution episodes for self-learning insights
    AnalyzeExecution,
}

fn slug(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

/// List research artifacts.
pub fn list_research(workdir: &Path) -> Result<Vec<PathBuf>> {
    ensure_dirs(workdir)?;
    let dir = research_dir(workdir);
    let mut files = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "md") {
                files.push(path);
            }
        }
    }
    files.sort();
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_basic() {
        assert_eq!(
            slug("Git Worktree Best Practices"),
            "git-worktree-best-practices"
        );
    }

    #[test]
    fn build_topic_prompt() {
        let prompt = build_research_prompt(
            Path::new("/test"),
            "context engineering for coding agents",
            "",
            ResearchMode::Topic,
        );
        assert!(prompt.contains("context engineering"));
        assert!(prompt.contains("arXiv"));
        assert!(prompt.contains("[AUTHOR-YEAR]"));
    }

    #[test]
    fn ensure_dirs_creates() {
        let tmp = tempfile::tempdir().unwrap();
        ensure_dirs(tmp.path()).unwrap();
        assert!(tmp.path().join(".roko/research").is_dir());
    }
}
