//! STATUS: NOT WIRED -- built but no non-test runtime caller.
//!
//! Cheap enrichment pass for noisy gate failures before retry.
//!
//! The caller is expected to provide a low-cost agent (for example a
//! mechanical/haiku-tier model). This helper constrains the prompt size,
//! requests a short diagnosis, and falls back to a deterministic summary when
//! the enrichment agent fails.

use roko_agent::Agent;
use roko_core::{Body, Context, Signal, Kind};

const RAW_ERROR_LIMIT: usize = 4_000;
const TASK_CONTEXT_LIMIT: usize = 1_000;
const DIAGNOSIS_SENTENCE_LIMIT: usize = 2;
const DIAGNOSIS_CHAR_LIMIT: usize = 320;

/// Summarize raw compiler or test output into a short retry-ready diagnosis.
///
/// The supplied `agent` should already be configured for the cheapest
/// mechanical-tier summarization pass. If the call fails or returns unusable
/// output, this falls back to a deterministic two-sentence digest extracted
/// from the raw error text.
pub async fn enrich_error_digest(raw_error: &str, agent: &dyn Agent, task_context: &str) -> String {
    let prompt = format!(
        "Diagnose this compilation/test error in 2 sentences. \
         Be specific about which file, line, and type mismatch.\n\
         Error output:\n{}\n\
         Task context:\n{}\n\
         Diagnosis:",
        truncate_chars(raw_error, RAW_ERROR_LIMIT),
        truncate_chars(task_context, TASK_CONTEXT_LIMIT),
    );

    let input = Signal::builder(Kind::Prompt)
        .body(Body::text(prompt))
        .build();
    let result = agent.run(&input, &Context::now()).await;

    if result.success
        && let Ok(text) = result.output.body.as_text()
    {
        let normalized = normalize_diagnosis(text);
        if !normalized.is_empty() {
            return normalized;
        }
    }

    fallback_diagnosis(raw_error, task_context)
}

fn normalize_diagnosis(text: &str) -> String {
    let compact = collapse_whitespace(text);
    if compact.is_empty() {
        return String::new();
    }

    let sentences = first_sentences(&compact, DIAGNOSIS_SENTENCE_LIMIT);
    if !sentences.is_empty() {
        return truncate_chars(&sentences, DIAGNOSIS_CHAR_LIMIT);
    }

    truncate_chars(&compact, DIAGNOSIS_CHAR_LIMIT)
}

fn fallback_diagnosis(raw_error: &str, task_context: &str) -> String {
    let file_line = extract_file_line(raw_error);
    let headline = extract_headline(raw_error);
    let mismatch = extract_mismatch(raw_error);

    let first = match (file_line.as_deref(), headline.as_deref()) {
        (Some(location), Some(detail)) if detail.contains(location) => {
            format!("The primary error is {detail}")
        }
        (Some(location), Some(detail)) => {
            format!("The failure points to {location}, where {detail}")
        }
        (Some(location), None) => format!("The failure points to {location}"),
        (None, Some(detail)) => format!("The primary error is {detail}"),
        (None, None) => {
            "The gate failed with compiler or test output that needs a targeted fix".to_string()
        }
    };

    let second = if let Some(detail) = mismatch {
        format!("The retry should fix {detail}")
    } else if !task_context.trim().is_empty() {
        format!(
            "The retry should focus on the failing code path for {}",
            truncate_chars(&collapse_whitespace(task_context), 140)
        )
    } else {
        "The retry should focus on the cited file and align the implementation with the failing check"
            .to_string()
    };

    format!("{} {}", ensure_sentence(&first), ensure_sentence(&second))
}

fn extract_file_line(raw_error: &str) -> Option<String> {
    raw_error.lines().find_map(|line| {
        let trimmed = line.trim();
        let candidate = trimmed.strip_prefix("-->").unwrap_or(trimmed).trim();
        candidate
            .split_whitespace()
            .find(|token| is_file_line_token(token))
            .map(|token| token.trim_end_matches([':', ',']).to_string())
    })
}

fn is_file_line_token(token: &str) -> bool {
    let Some((_, tail)) = token.rsplit_once(".rs:") else {
        return false;
    };

    let mut parts = tail.split(':');
    parts
        .next()
        .is_some_and(|part| !part.is_empty() && part.chars().all(|ch| ch.is_ascii_digit()))
}

fn extract_headline(raw_error: &str) -> Option<String> {
    raw_error
        .lines()
        .map(str::trim)
        .find(|line| {
            !line.is_empty()
                && (line.starts_with("error[")
                    || line.starts_with("error:")
                    || line.contains("panicked at")
                    || line.contains("FAIL")
                    || line.contains("failed"))
        })
        .map(clean_fragment)
}

fn extract_mismatch(raw_error: &str) -> Option<String> {
    if let Some(line) = raw_error
        .lines()
        .map(str::trim)
        .find(|line| line.contains("expected ") && line.contains("found "))
    {
        return Some(truncate_chars(&focus_fragment(line), 180));
    }

    let detail = raw_error
        .lines()
        .map(str::trim)
        .filter(|line| {
            !line.is_empty()
                && (line.contains("mismatched types")
                    || line.contains("expected ")
                    || line.contains("found ")
                    || line.contains("no method named")
                    || line.contains("cannot find")
                    || line.contains("trait bound")
                    || line.contains("borrow of moved value"))
        })
        .take(2)
        .map(focus_fragment)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ");

    if detail.is_empty() {
        None
    } else {
        Some(truncate_chars(&detail, 180))
    }
}

fn clean_fragment(text: &str) -> String {
    collapse_whitespace(text)
        .trim_end_matches(['.', '!', '?'])
        .to_string()
}

fn focus_fragment(text: &str) -> String {
    let cleaned = clean_fragment(text);
    const KEYWORDS: &[&str] = &[
        "expected ",
        "found ",
        "mismatched types",
        "no method named",
        "cannot find",
        "trait bound",
        "borrow of moved value",
    ];

    let start = KEYWORDS
        .iter()
        .filter_map(|keyword| cleaned.find(keyword))
        .min()
        .unwrap_or(0);

    cleaned[start..]
        .trim_start_matches(['|', '-', '^', ':', ' '])
        .to_string()
}

fn first_sentences(text: &str, limit: usize) -> String {
    let mut sentences = Vec::new();
    let mut start = 0;

    for (idx, ch) in text.char_indices() {
        if matches!(ch, '.' | '!' | '?') && is_sentence_boundary(text, idx, ch) {
            let end = idx + ch.len_utf8();
            let sentence = text[start..end].trim();
            if !sentence.is_empty() {
                sentences.push(sentence.to_string());
                if sentences.len() == limit {
                    break;
                }
            }
            start = end;
        }
    }

    if sentences.len() < limit {
        let tail = text[start..].trim();
        if !tail.is_empty() {
            sentences.push(ensure_sentence(tail));
        }
    }

    if sentences.is_empty() {
        String::new()
    } else {
        sentences
            .into_iter()
            .take(limit)
            .collect::<Vec<_>>()
            .join(" ")
    }
}

fn is_sentence_boundary(text: &str, idx: usize, ch: char) -> bool {
    let next_idx = idx + ch.len_utf8();
    let mut rest = text[next_idx..].chars().skip_while(|c| c.is_whitespace());
    match rest.next() {
        None => true,
        Some(next) => next.is_ascii_uppercase() || matches!(next, '"' | '\'' | '('),
    }
}

fn ensure_sentence(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if trimmed.ends_with(['.', '!', '?']) {
        trimmed.to_string()
    } else {
        format!("{trimmed}.")
    }
}

fn collapse_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    text.chars().take(max_chars).collect()
}

#[cfg(test)]
mod tests {
    use super::enrich_error_digest;
    use roko_agent::MockAgent;

    #[tokio::test]
    async fn error_enrichment_returns_two_sentence_mock_diagnosis() {
        let agent = MockAgent::reply(
            "The compile error is in crates/roko-learn/src/model_router.rs:144:17 where \
             Rust reports a mismatched types error. The function expects a String but the \
             code passes an Option<String>. Extra commentary that should be dropped.",
        );

        let diagnosis = enrich_error_digest(
            "error[E0308]: mismatched types",
            &agent,
            "Task 2J.09 implements error enrichment.",
        )
        .await;

        assert_eq!(
            diagnosis,
            "The compile error is in crates/roko-learn/src/model_router.rs:144:17 where Rust reports a mismatched types error. The function expects a String but the code passes an Option<String>."
        );
    }

    #[tokio::test]
    async fn error_enrichment_falls_back_when_agent_fails() {
        let agent = MockAgent::fail_with("upstream timeout");
        let raw_error = "\
error[E0308]: mismatched types
  --> crates/roko-learn/src/model_router.rs:144:17
   |
144 |     route(model)
   |     ----- ^^^^^ expected `String`, found `Option<String>`
   |
";

        let diagnosis = enrich_error_digest(
            raw_error,
            &agent,
            "Update the router selection path for retry handling.",
        )
        .await;

        assert_eq!(
            diagnosis,
            "The failure points to crates/roko-learn/src/model_router.rs:144:17, where error[E0308]: mismatched types. The retry should fix expected `String`, found `Option<String>`."
        );
    }

    #[tokio::test]
    async fn error_enrichment_falls_back_when_agent_output_is_empty() {
        let agent = MockAgent::reply("   ");

        let diagnosis = enrich_error_digest(
            "thread 'router' panicked at src/lib.rs:91:5: assertion failed: left == right",
            &agent,
            "",
        )
        .await;

        assert_eq!(
            diagnosis,
            "The primary error is thread 'router' panicked at src/lib.rs:91:5: assertion failed: left == right. The retry should focus on the cited file and align the implementation with the failing check."
        );
    }
}
