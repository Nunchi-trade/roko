//! think command handler.
#![allow(unused_imports)]

use crate::*;
use serde::Serialize;

#[derive(Debug, Serialize)]
struct ResearchArtifactHit {
    path: PathBuf,
    score: usize,
    preview: String,
}

/// `roko think <question>` - read-only research over local explain,
/// knowledge, research artifacts, and repository context.
pub(crate) async fn cmd_think(
    cli: &Cli,
    question: Vec<String>,
    workdir: Option<PathBuf>,
) -> Result<i32> {
    let workdir = workdir.unwrap_or_else(|| resolve_workdir(cli));
    let question = question.join(" ");
    let question = question.trim();
    if question.is_empty() {
        bail!("provide a question to think about");
    }

    let keywords = extract_keywords(question);
    let keyword_refs = keywords.iter().map(String::as_str).collect::<Vec<_>>();
    let repo_context = if keyword_refs.is_empty() {
        None
    } else {
        Some(roko_cli::repo_context::build_repo_context(&workdir, &keyword_refs).await?)
    };
    let explain_topic = matching_explain_topic(question);
    let knowledge_store = roko_neuro::KnowledgeStore::for_workdir(&workdir);
    let knowledge_entries = knowledge_store.query(question, 5).with_context(|| {
        format!(
            "query knowledge store at {}",
            knowledge_store.path().display()
        )
    })?;
    let research_hits = research_artifact_hits(&workdir, &keywords)?;

    if cli.json {
        let payload = serde_json::json!({
            "workdir": workdir,
            "question": question,
            "keywords": keywords,
            "explain_topic": explain_topic,
            "repo_context": repo_context,
            "knowledge": {
                "path": knowledge_store.path(),
                "entries": knowledge_entries,
            },
            "research_artifacts": research_hits,
            "mutated_files": [],
        });
        println!("{}", serde_json::to_string_pretty(&payload)?);
        return Ok(EXIT_SUCCESS);
    }

    println!("Analysis: {question}");
    println!("Workspace: {}", workdir.display());
    println!("Mode: read-only");
    println!();

    if let Some(topic) = explain_topic.as_deref() {
        if let Some(entry) = roko_cli::explain::find_topic(topic) {
            println!("Concept help: {topic}");
            print!("{}", roko_cli::explain::render_topic(entry, 1));
            println!();
        }
    }

    if let Some(context) = repo_context.as_ref() {
        print_repo_context(context);
    }

    print_knowledge_matches(knowledge_store.path(), &knowledge_entries);
    print_research_hits(&research_hits);
    println!("No source files were modified.");

    Ok(EXIT_SUCCESS)
}

fn extract_keywords(question: &str) -> Vec<String> {
    const STOPWORDS: &[&str] = &[
        "a", "about", "an", "and", "are", "as", "at", "be", "codebase", "does", "for", "how", "in",
        "is", "it", "of", "on", "or", "our", "the", "this", "to", "what", "where", "with", "work",
        "works",
    ];

    let mut keywords = Vec::new();
    for word in question
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '-')
        .map(str::trim)
        .filter(|word| !word.is_empty())
    {
        let lower = word.to_ascii_lowercase();
        if lower.len() < 3 || STOPWORDS.contains(&lower.as_str()) {
            continue;
        }
        if !keywords.contains(&lower) {
            keywords.push(lower);
        }
        if keywords.len() >= 8 {
            break;
        }
    }
    keywords
}

fn matching_explain_topic(question: &str) -> Option<String> {
    let lower = question.to_ascii_lowercase();
    roko_cli::explain::topic_names()
        .into_iter()
        .find(|topic| {
            lower
                .split(|c: char| !c.is_ascii_alphanumeric())
                .any(|word| word == *topic)
        })
        .map(str::to_string)
}

fn print_repo_context(context: &roko_cli::repo_context::RepoContextPack) {
    println!("Repository context:");
    println!("  project: {}", context.project_kind);
    if context.workspace_members.is_empty() {
        println!("  workspace members: none found");
    } else {
        println!(
            "  workspace members: {}",
            context
                .workspace_members
                .iter()
                .take(8)
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        );
    }

    if !context.key_files.is_empty() {
        println!("  key files:");
        for path in context.key_files.iter().take(8) {
            println!("    {}", path.display());
        }
    }

    if !context.matching_symbols.is_empty() {
        println!("  symbol/text matches:");
        for hit in context.matching_symbols.iter().take(10) {
            println!("    {}:{} {}", hit.file.display(), hit.line, hit.text);
        }
    }

    if !context.related_prds.is_empty() {
        println!("  related PRDs:");
        for path in context.related_prds.iter().take(5) {
            println!("    {}", path.display());
        }
    }

    if !context.related_plans.is_empty() {
        println!("  related plans:");
        for path in context.related_plans.iter().take(5) {
            println!("    {}", path.display());
        }
    }

    if context.key_files.is_empty()
        && context.matching_symbols.is_empty()
        && context.related_prds.is_empty()
        && context.related_plans.is_empty()
    {
        println!("  no direct file, symbol, PRD, or plan matches");
    }
    println!();
}

fn print_knowledge_matches(path: &Path, entries: &[roko_neuro::KnowledgeEntry]) {
    println!("Durable knowledge: {}", path.display());
    if entries.is_empty() {
        println!("  no matches");
        println!();
        return;
    }

    for (idx, entry) in entries.iter().enumerate() {
        println!(
            "  {}. [{:?}] confidence {:.2} {}",
            idx + 1,
            entry.kind,
            entry.confidence.clamp(0.0, 1.0),
            entry.content.trim()
        );
    }
    println!();
}

fn research_artifact_hits(workdir: &Path, keywords: &[String]) -> Result<Vec<ResearchArtifactHit>> {
    if keywords.is_empty() {
        return Ok(Vec::new());
    }

    let dir = workdir.join(".roko").join("research");
    if !dir.is_dir() {
        return Ok(Vec::new());
    }

    let mut hits = Vec::new();
    for entry in std::fs::read_dir(&dir).with_context(|| format!("read {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        if !path.extension().is_some_and(|ext| ext == "md") {
            continue;
        }

        let content =
            std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        let haystack = format!(
            "{}\n{}",
            path.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default(),
            content
        )
        .to_ascii_lowercase();
        let score = keywords
            .iter()
            .map(|keyword| haystack.matches(keyword).count())
            .sum::<usize>();
        if score == 0 {
            continue;
        }

        hits.push(ResearchArtifactHit {
            preview: preview_line(&content, keywords),
            path,
            score,
        });
    }

    hits.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.path.cmp(&b.path)));
    hits.truncate(5);
    Ok(hits)
}

fn preview_line(content: &str, keywords: &[String]) -> String {
    let matching_line = content.lines().find(|line| {
        let lower = line.to_ascii_lowercase();
        keywords.iter().any(|keyword| lower.contains(keyword))
    });
    let line = matching_line
        .or_else(|| content.lines().find(|line| !line.trim().is_empty()))
        .unwrap_or_default()
        .trim();
    truncate_chars(line, 180)
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    let mut out = value.chars().take(max_chars).collect::<String>();
    if value.chars().count() > max_chars {
        out.push_str("...");
    }
    out
}

fn print_research_hits(hits: &[ResearchArtifactHit]) {
    println!("Research artifacts:");
    if hits.is_empty() {
        println!("  no matches");
        println!();
        return;
    }

    for hit in hits {
        println!("  {} (score {})", hit.path.display(), hit.score);
        if !hit.preview.is_empty() {
            println!("    {}", hit.preview);
        }
    }
    println!();
}
