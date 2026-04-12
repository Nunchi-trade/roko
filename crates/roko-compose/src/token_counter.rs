//! Model-aware token counting for prompt budget management.

use std::{
    env, fs,
    path::{Path, PathBuf},
};

/// Client-side token counting strategy selected per model family.
pub enum TokenCounter {
    /// OpenAI-style models counted with tiktoken.
    Tiktoken(tiktoken_rs::CoreBPE),
    /// Models with an available HuggingFace `tokenizer.json`.
    HuggingFace(tokenizers::Tokenizer),
    /// Conservative fallback when no exact tokenizer is available.
    Heuristic {
        /// Average characters-per-token used for conservative estimation.
        chars_per_token: f64,
    },
}

impl TokenCounter {
    /// Count tokens for `text`.
    #[must_use]
    pub fn count(&self, text: &str) -> usize {
        match self {
            Self::Tiktoken(bpe) => bpe.encode_with_special_tokens(text).len(),
            Self::HuggingFace(tokenizer) => tokenizer
                .encode(text, false)
                .map(|encoding| encoding.get_ids().len())
                .unwrap_or(0),
            Self::Heuristic { chars_per_token } => heuristic_count(text, *chars_per_token),
        }
    }

    /// Select the best available tokenizer for `slug`.
    #[must_use]
    pub fn for_model(slug: &str) -> Self {
        if slug.starts_with("claude-") || slug.starts_with("gpt-") || slug.starts_with("o1") {
            return tiktoken_rs::o200k_base()
                .map(Self::Tiktoken)
                .unwrap_or(Self::Heuristic {
                    chars_per_token: 4.0,
                });
        }

        if slug.starts_with("glm-") {
            return Self::try_hf("zai-org/GLM-4.7").unwrap_or(Self::Heuristic {
                chars_per_token: 3.8,
            });
        }

        if slug.starts_with("kimi-") {
            return Self::try_hf("moonshotai/Kimi-K2-Instruct").unwrap_or(Self::Heuristic {
                chars_per_token: 3.5,
            });
        }

        Self::Heuristic {
            chars_per_token: 4.0,
        }
    }

    fn try_hf(repo_id: &str) -> Option<Self> {
        find_hf_tokenizer_json(repo_id)
            .and_then(|path| tokenizers::Tokenizer::from_file(path).ok())
            .map(Self::HuggingFace)
    }
}

fn heuristic_count(text: &str, chars_per_token: f64) -> usize {
    if text.is_empty() {
        return 0;
    }

    ((text.len() as f64) / chars_per_token).ceil() as usize
}

fn find_hf_tokenizer_json(repo_id: &str) -> Option<PathBuf> {
    let repo_key = format!("models--{}", repo_id.replace('/', "--"));
    let repo_dir_candidates = hf_cache_roots()
        .into_iter()
        .map(|root| root.join(&repo_key))
        .collect::<Vec<_>>();

    for repo_dir in repo_dir_candidates {
        let direct = repo_dir.join("tokenizer.json");
        if direct.is_file() {
            return Some(direct);
        }

        let refs_main = repo_dir.join("refs").join("main");
        if let Ok(snapshot) = fs::read_to_string(&refs_main) {
            let resolved = repo_dir
                .join("snapshots")
                .join(snapshot.trim())
                .join("tokenizer.json");
            if resolved.is_file() {
                return Some(resolved);
            }
        }

        let snapshots_dir = repo_dir.join("snapshots");
        if let Some(found) = first_tokenizer_json_in_dir(&snapshots_dir) {
            return Some(found);
        }
    }

    None
}

fn hf_cache_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();

    if let Some(path) = env_path("HF_HUB_CACHE") {
        roots.push(path);
    }
    if let Some(path) = env_path("HUGGINGFACE_HUB_CACHE") {
        roots.push(path);
    }
    if let Some(path) = env_path("HF_HOME") {
        roots.push(path.join("hub"));
    }
    if let Some(home) = env_path("HOME") {
        roots.push(home.join(".cache").join("huggingface").join("hub"));
    }

    roots
}

fn env_path(key: &str) -> Option<PathBuf> {
    env::var_os(key).map(PathBuf::from)
}

fn first_tokenizer_json_in_dir(dir: &Path) -> Option<PathBuf> {
    let entries = fs::read_dir(dir).ok()?;

    for entry in entries.flatten() {
        let candidate = entry.path().join("tokenizer.json");
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::TokenCounter;

    #[test]
    fn token_counter_for_claude_uses_tiktoken() {
        assert!(matches!(
            TokenCounter::for_model("claude-opus-4-6"),
            TokenCounter::Tiktoken(_)
        ));
    }

    #[test]
    fn token_counter_for_glm_counts_reasonably() {
        let count = TokenCounter::for_model("glm-5.1").count("hello world");
        assert!((1..=6).contains(&count), "unexpected token count: {count}");
    }

    #[test]
    fn token_counter_heuristic_is_conservative_for_non_empty_text() {
        let counter = TokenCounter::Heuristic {
            chars_per_token: 4.0,
        };

        assert_eq!(counter.count(""), 0);
        assert_eq!(counter.count("a"), 1);
        assert_eq!(counter.count("hello world"), 3);
    }
}
