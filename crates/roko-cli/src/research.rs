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
use roko_agent::gemini::GroundingMetadata;
use roko_agent::perplexity::embed::PerplexityEmbedAgent;
use roko_agent::perplexity::types::{PerplexityMetadata, SearchOptions};
use roko_core::config::schema::{GeminiConfig, PerplexityConfig};

fn research_dir(workdir: &Path) -> PathBuf {
    workdir.join(".roko").join("research")
}

/// Ensure the research directory exists.
pub fn ensure_dirs(workdir: &Path) -> Result<()> {
    std::fs::create_dir_all(research_dir(workdir))?;
    Ok(())
}

/// Convert a Perplexity response + metadata into rich markdown and save it.
///
/// Writes a `# Research: <topic>` document with:
/// - The model-generated `content`
/// - A numbered `## Sources` section linking each citation to its title
/// - A `## Search Context` section with full snippets for agent consumption
pub fn save_research_with_citations(
    workdir: &Path,
    topic: &str,
    content: &str,
    metadata: &PerplexityMetadata,
) -> Result<PathBuf> {
    let mut doc = String::new();
    writeln!(doc, "# Research: {topic}\n")?;
    writeln!(
        doc,
        "> Generated via Perplexity Sonar — {}\n",
        chrono::Local::now().format("%Y-%m-%d")
    )?;
    writeln!(doc, "{content}\n")?;

    if !metadata.citations.is_empty() {
        writeln!(doc, "\n## Sources\n")?;
        for (i, url) in metadata.citations.iter().enumerate() {
            let title = metadata
                .search_results
                .get(i)
                .map(|r| r.title.as_str())
                .unwrap_or("Source");
            writeln!(doc, "{i}. [{title}]({url})")?;
        }
    }

    if !metadata.search_results.is_empty() {
        writeln!(doc, "\n## Search Context\n")?;
        for result in &metadata.search_results {
            writeln!(doc, "### [{}]({})", result.title, result.url)?;
            if let Some(date) = &result.date {
                writeln!(doc, "> Published: {date}")?;
            }
            writeln!(doc, "\n{}\n", result.content)?;
        }
    }

    let path = research_dir(workdir).join(format!("{}.md", slug(topic)));
    std::fs::write(&path, doc)?;
    Ok(path)
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
        let _ = writeln!(
            prompt,
            "## What already exists (do NOT duplicate)\n{master_index}\n---\n"
        );
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

/// Build a research prompt with Perplexity-aware instructions and populate
/// [`SearchOptions`] from the provider config.
///
/// Differences from [`build_research_prompt`]:
/// - Replaces `[AUTHOR-YEAR]` citation format with `[N]` bracket notation
///   (Perplexity returns numbered citations automatically).
/// - Removes the "verify papers exist" instruction — Perplexity grounds
///   responses against live search so hallucinated citations are not an issue.
pub fn build_research_prompt_perplexity(
    workdir: &Path,
    topic: &str,
    context: &str,
    mode: ResearchMode,
    pplx_config: &PerplexityConfig,
) -> (String, SearchOptions) {
    let prompt = build_research_prompt(workdir, topic, context, mode);

    // Replace [AUTHOR-YEAR] citation format everywhere with [N] bracket
    // notation matching Perplexity's auto-numbered citations.
    let prompt = prompt.replace("[AUTHOR-YEAR]", "[N]");
    // Drop the "verify papers exist" instruction — Perplexity grounds
    // responses against live search so hallucinated citations are not an issue.
    let prompt = prompt.replace(
        " Verify papers exist — do not hallucinate citations.",
        " Perplexity citations are auto-verified from live search.",
    );

    let search_opts = SearchOptions {
        search_mode: if pplx_config.academic_mode {
            Some("academic".to_string())
        } else {
            None
        },
        search_recency_filter: Some(pplx_config.search_recency_filter.clone()),
        search_domain_filter: if pplx_config.search_domain_filter.is_empty() {
            None
        } else {
            Some(pplx_config.search_domain_filter.clone())
        },
        return_related_questions: Some(pplx_config.return_related_questions),
        return_images: Some(pplx_config.return_images),
        ..Default::default()
    };

    (prompt, search_opts)
}

/// Build a research prompt for Gemini grounding-backed research.
///
/// Returns the prompt and whether Google Search grounding should be enabled.
#[must_use]
pub fn build_research_prompt_gemini(
    workdir: &Path,
    topic: &str,
    mode: ResearchMode,
    gemini_config: &GeminiConfig,
) -> (String, bool) {
    let prompt = build_research_prompt(workdir, topic, "", mode);
    let enable_grounding = gemini_config.grounding_model.is_some();
    (prompt, enable_grounding)
}

/// Extract `(title, url)` citation pairs from Gemini grounding metadata.
#[must_use]
pub fn grounding_to_citations(meta: &GroundingMetadata) -> Vec<(String, String)> {
    let mut citations = Vec::new();

    if let Some(chunks) = &meta.grounding_chunks {
        for chunk in chunks {
            if let Some(web) = &chunk.web {
                let citation = (web.title.clone(), web.uri.clone());
                if !citations.contains(&citation) {
                    citations.push(citation);
                }
            }
        }
    }

    citations
}

/// Convert Gemini grounding metadata into the same markdown research shape
/// used by search-grounded Perplexity research.
pub fn save_research_with_grounding(
    workdir: &Path,
    topic: &str,
    content: &str,
    metadata: &GroundingMetadata,
) -> Result<PathBuf> {
    let mut doc = String::new();
    writeln!(doc, "# Research: {topic}\n")?;
    writeln!(
        doc,
        "> Generated via Gemini Google Search grounding — {}\n",
        chrono::Local::now().format("%Y-%m-%d")
    )?;
    writeln!(doc, "{content}\n")?;

    let citations = grounding_to_citations(metadata);
    if !citations.is_empty() {
        writeln!(doc, "\n## Sources\n")?;
        for (i, (title, url)) in citations.iter().enumerate() {
            writeln!(doc, "{}. [{title}]({url})", i + 1)?;
        }
    }

    let has_queries = metadata
        .web_search_queries
        .as_ref()
        .is_some_and(|queries| !queries.is_empty());
    let has_supports = metadata
        .grounding_supports
        .as_ref()
        .is_some_and(|supports| !supports.is_empty());

    if has_queries || has_supports {
        writeln!(doc, "\n## Search Context\n")?;

        if let Some(queries) = &metadata.web_search_queries
            && !queries.is_empty()
        {
            writeln!(doc, "### Queries\n")?;
            for query in queries {
                writeln!(doc, "- {query}")?;
            }
            writeln!(doc)?;
        }

        if let Some(supports) = &metadata.grounding_supports
            && !supports.is_empty()
        {
            writeln!(doc, "### Supports\n")?;
            for support in supports {
                let refs = support
                    .grounding_chunk_indices
                    .iter()
                    .map(|index| (index + 1).to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                let segment = support.segment.text.trim();

                if refs.is_empty() {
                    writeln!(doc, "- {segment}")?;
                } else {
                    writeln!(doc, "- {segment} [{refs}]")?;
                }
            }
        }
    }

    let path = research_dir(workdir).join(format!("{}.md", slug(topic)));
    std::fs::write(&path, doc)?;
    Ok(path)
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

// ── Embedding index ───────────────────────────────────────────────────────────

/// Split markdown `content` into chunks of at most `max_tokens` (estimated at
/// 4 chars/token), respecting paragraph boundaries where possible.
pub fn chunk_markdown(content: &str, max_tokens: usize) -> Vec<String> {
    if content.trim().is_empty() {
        return Vec::new();
    }
    let max_chars = max_tokens * 4;
    let mut chunks = Vec::new();
    let mut current = String::new();

    for para in content.split("\n\n") {
        if current.len() + para.len() + 2 > max_chars && !current.is_empty() {
            chunks.push(current.trim().to_string());
            current = para.to_string();
        } else {
            if !current.is_empty() {
                current.push_str("\n\n");
            }
            current.push_str(para);
        }
    }
    if !current.trim().is_empty() {
        chunks.push(current.trim().to_string());
    }
    chunks
}

/// A single entry in the research index.
struct IndexEntry {
    file: PathBuf,
    chunk: String,
    embedding: Vec<f32>,
}

/// In-memory semantic index over `.roko/research/` chunks.
pub struct ResearchIndex {
    entries: Vec<IndexEntry>,
}

/// A single search result.
pub struct ResearchHit {
    pub file: PathBuf,
    pub chunk: String,
    pub score: f32,
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    dot / (norm_a * norm_b)
}

impl ResearchIndex {
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Add a chunk with its pre-computed embedding.
    pub fn add(&mut self, file: PathBuf, chunk: String, embedding: Vec<f32>) {
        self.entries.push(IndexEntry {
            file,
            chunk,
            embedding,
        });
    }

    /// Return the top-k entries by cosine similarity to `query_embedding`.
    pub fn search(&self, query_embedding: &[f32], top_k: usize) -> Result<Vec<ResearchHit>> {
        let mut scored: Vec<(f32, usize)> = self
            .entries
            .iter()
            .enumerate()
            .map(|(i, e)| (cosine_similarity(query_embedding, &e.embedding), i))
            .collect();
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        let hits = scored
            .into_iter()
            .take(top_k)
            .map(|(score, i)| ResearchHit {
                file: self.entries[i].file.clone(),
                chunk: self.entries[i].chunk.clone(),
                score,
            })
            .collect();
        Ok(hits)
    }

    /// Number of indexed chunks.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl Default for ResearchIndex {
    fn default() -> Self {
        Self::new()
    }
}

/// Build a [`ResearchIndex`] from all `.roko/research/*.md` files in `workdir`.
///
/// Each file is split into ~512-token chunks and embedded via `embed_agent`.
pub async fn build_research_index(
    workdir: &Path,
    embed_agent: &PerplexityEmbedAgent,
) -> Result<ResearchIndex> {
    let files = list_research(workdir)?;
    let mut index = ResearchIndex::new();

    for file in files {
        let content = std::fs::read_to_string(&file)?;
        let chunks = chunk_markdown(&content, 512);
        let texts: Vec<&str> = chunks.iter().map(String::as_str).collect();
        if texts.is_empty() {
            continue;
        }
        let embeddings = embed_agent
            .embed(&texts)
            .await
            .map_err(|e| anyhow::anyhow!("embed failed: {e}"))?;

        for (chunk, embedding) in chunks.into_iter().zip(embeddings) {
            index.add(file.clone(), chunk, embedding);
        }
    }

    Ok(index)
}

/// Semantically search the index for chunks relevant to `query`.
pub async fn search_research(
    index: &ResearchIndex,
    embed_agent: &PerplexityEmbedAgent,
    query: &str,
    top_k: usize,
) -> Result<Vec<ResearchHit>> {
    let query_embedding = embed_agent
        .embed(&[query])
        .await
        .map_err(|e| anyhow::anyhow!("embed failed: {e}"))?;
    index.search(&query_embedding[0], top_k)
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
    fn research_prompt_perplexity() {
        let cfg = PerplexityConfig {
            academic_mode: true,
            search_recency_filter: "month".to_string(),
            search_domain_filter: vec!["arxiv.org".to_string()],
            return_images: false,
            return_related_questions: true,
            ..Default::default()
        };

        let (prompt, opts) = build_research_prompt_perplexity(
            Path::new("/test"),
            "transformer architectures",
            "",
            ResearchMode::Topic,
            &cfg,
        );

        // Perplexity-aware citation format is present.
        assert!(prompt.contains("[N]"), "expected [N] bracket notation");
        // "verify papers exist" instruction is removed.
        assert!(
            !prompt.to_lowercase().contains("verify papers exist"),
            "should not contain verify papers exist"
        );
        // [AUTHOR-YEAR] format is removed.
        assert!(
            !prompt.contains("[AUTHOR-YEAR]"),
            "should not contain [AUTHOR-YEAR]"
        );

        // SearchOptions are populated from config.
        assert_eq!(opts.search_mode.as_deref(), Some("academic"));
        assert_eq!(opts.search_recency_filter.as_deref(), Some("month"));
        assert_eq!(
            opts.search_domain_filter.as_deref(),
            Some(["arxiv.org".to_string()].as_slice())
        );
        assert_eq!(opts.return_related_questions, Some(true));
        assert_eq!(opts.return_images, Some(false));
    }

    #[test]
    fn research_gemini_grounding_prompt_enabled_when_model_configured() {
        let cfg = GeminiConfig {
            grounding_model: Some("gemini-3-flash-preview".to_string()),
            ..Default::default()
        };

        let (prompt, enable_grounding) = build_research_prompt_gemini(
            Path::new("/test"),
            "rust async runtimes",
            ResearchMode::Topic,
            &cfg,
        );

        assert!(enable_grounding);
        assert!(prompt.contains("rust async runtimes"));
        assert!(prompt.contains("[AUTHOR-YEAR]"));
    }

    #[test]
    fn research_gemini_grounding_saves_sources_and_supports() {
        use roko_agent::gemini::{
            GroundingChunk, GroundingMetadata, GroundingSupport, TextSegment, WebChunk,
        };

        let tmp = tempfile::tempdir().unwrap();
        ensure_dirs(tmp.path()).unwrap();

        let metadata = GroundingMetadata {
            web_search_queries: Some(vec!["Rust async runtimes benchmark".to_string()]),
            grounding_chunks: Some(vec![
                GroundingChunk {
                    web: Some(WebChunk {
                        uri: "https://tokio.rs".to_string(),
                        title: "Tokio".to_string(),
                    }),
                },
                GroundingChunk {
                    web: Some(WebChunk {
                        uri: "https://docs.rs/async-std".to_string(),
                        title: "async-std".to_string(),
                    }),
                },
            ]),
            grounding_supports: Some(vec![GroundingSupport {
                segment: TextSegment {
                    start_index: 0,
                    end_index: 32,
                    text: "Tokio remains the dominant runtime.".to_string(),
                },
                grounding_chunk_indices: vec![0],
                confidence_scores: Some(vec![0.93]),
            }]),
            search_entry_point: None,
        };

        let path = save_research_with_grounding(
            tmp.path(),
            "rust async runtimes",
            "Tokio remains the dominant runtime.",
            &metadata,
        )
        .unwrap();

        let doc = std::fs::read_to_string(&path).unwrap();
        assert!(doc.contains("# Research: rust async runtimes"));
        assert!(doc.contains("Gemini Google Search grounding"));
        assert!(doc.contains("## Sources"));
        assert!(doc.contains("[Tokio](https://tokio.rs)"));
        assert!(doc.contains("[async-std](https://docs.rs/async-std)"));
        assert!(doc.contains("## Search Context"));
        assert!(doc.contains("Rust async runtimes benchmark"));
        assert!(doc.contains("Tokio remains the dominant runtime. [1]"));
    }

    #[test]
    fn save_research_citations() {
        use roko_agent::perplexity::types::{Annotation, SearchResult};

        let tmp = tempfile::tempdir().unwrap();
        ensure_dirs(tmp.path()).unwrap();

        let metadata = PerplexityMetadata {
            citations: vec![
                "https://example.com/paper1".to_string(),
                "https://example.com/paper2".to_string(),
            ],
            search_results: vec![
                SearchResult {
                    url: "https://example.com/paper1".to_string(),
                    title: "Attention Is All You Need".to_string(),
                    content: "The dominant sequence transduction models...".to_string(),
                    date: Some("2017-06-12".to_string()),
                    last_updated: None,
                },
                SearchResult {
                    url: "https://example.com/paper2".to_string(),
                    title: "BERT".to_string(),
                    content: "We introduce a new language model...".to_string(),
                    date: None,
                    last_updated: None,
                },
            ],
            annotations: vec![Annotation {
                start_index: 0,
                end_index: 10,
                title: "Attention Is All You Need".to_string(),
                url: "https://example.com/paper1".to_string(),
            }],
            related_questions: vec![],
        };

        let path = save_research_with_citations(
            tmp.path(),
            "transformer architectures",
            "Transformers are the dominant architecture.",
            &metadata,
        )
        .unwrap();

        let doc = std::fs::read_to_string(&path).unwrap();
        assert!(doc.contains("# Research: transformer architectures"));
        assert!(doc.contains("Perplexity Sonar"));
        assert!(doc.contains("## Sources"));
        assert!(doc.contains("[Attention Is All You Need](https://example.com/paper1)"));
        assert!(doc.contains("[BERT](https://example.com/paper2)"));
        assert!(doc.contains("## Search Context"));
        assert!(doc.contains("> Published: 2017-06-12"));
        assert!(doc.contains("The dominant sequence transduction models..."));
        assert!(path.ends_with("transformer-architectures.md"));
    }

    #[test]
    fn ensure_dirs_creates() {
        let tmp = tempfile::tempdir().unwrap();
        ensure_dirs(tmp.path()).unwrap();
        assert!(tmp.path().join(".roko/research").is_dir());
    }

    // ── research_index ────────────────────────────────────────────────────────

    #[test]
    fn research_index_add_and_search() {
        let mut index = ResearchIndex::new();
        let file = PathBuf::from("/test/research.md");

        // Three entries with 2D mock embeddings.
        index.add(
            file.clone(),
            "chunk about agents".to_string(),
            vec![1.0, 0.0],
        );
        index.add(
            file.clone(),
            "chunk about models".to_string(),
            vec![0.0, 1.0],
        );
        index.add(
            file.clone(),
            "chunk about orchestration".to_string(),
            vec![0.9, 0.1_f32],
        );

        // Query pointing along [1.0, 0.0] — should rank "agents" first,
        // "orchestration" second.
        let results = index.search(&[1.0, 0.0], 2).unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].chunk, "chunk about agents");
        assert_eq!(results[1].chunk, "chunk about orchestration");
        assert!(results[0].score > results[1].score);
    }

    #[test]
    fn research_index_top_k_capped_at_index_size() {
        let mut index = ResearchIndex::new();
        let file = PathBuf::from("/test/a.md");
        index.add(file.clone(), "x".to_string(), vec![1.0]);
        index.add(file.clone(), "y".to_string(), vec![0.5]);

        let results = index.search(&[1.0], 10).unwrap();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn research_index_empty_search_returns_empty() {
        let index = ResearchIndex::new();
        let results = index.search(&[1.0, 0.0], 5).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn chunk_markdown_splits_on_paragraphs() {
        // With max_tokens=3 (~12 chars), each paragraph forces a new chunk.
        let content = "First paragraph.\n\nSecond paragraph.\n\nThird paragraph.";
        let chunks = chunk_markdown(content, 3);
        assert!(
            chunks.len() >= 2,
            "expected multiple chunks, got {chunks:?}"
        );
        let all = chunks.join(" ");
        assert!(all.contains("First paragraph"));
        assert!(all.contains("Second paragraph"));
        assert!(all.contains("Third paragraph"));
    }

    #[test]
    fn chunk_markdown_empty_returns_empty() {
        assert!(chunk_markdown("", 512).is_empty());
        assert!(chunk_markdown("   \n\n  ", 512).is_empty());
    }

    #[test]
    fn chunk_markdown_single_chunk_when_small() {
        let content = "Short content.";
        let chunks = chunk_markdown(content, 512);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], "Short content.");
    }
}
