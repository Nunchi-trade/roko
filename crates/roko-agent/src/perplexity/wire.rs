use crate::perplexity::types::{Annotation, PerplexityMetadata, SearchOptions, SearchResult};
use serde_json::{Map, Value, json};

pub(crate) fn chat_endpoint(base_url: &str) -> String {
    let trimmed = base_url.trim_end_matches('/');
    format!("{trimmed}/chat/completions")
}

pub(crate) fn headers(api_key: &str) -> Vec<(String, String)> {
    vec![
        ("Authorization".to_string(), format!("Bearer {api_key}")),
        ("Content-Type".to_string(), "application/json".to_string()),
    ]
}

pub(crate) fn base_chat_body(model_slug: &str, messages: Vec<Value>) -> Value {
    json!({
        "model": model_slug,
        "messages": messages,
    })
}

pub(crate) fn apply_search_options(body: &mut Map<String, Value>, opts: &SearchOptions) {
    if let Some(ref filter) = opts.search_domain_filter {
        body.insert("search_domain_filter".to_string(), json!(filter));
    }
    if let Some(ref recency) = opts.search_recency_filter {
        body.insert("search_recency_filter".to_string(), json!(recency));
    }
    if let Some(ref mode) = opts.search_mode {
        body.insert("search_mode".to_string(), json!(mode));
    }
    if let Some(images) = opts.return_images {
        body.insert("return_images".to_string(), json!(images));
    }
    if let Some(related) = opts.return_related_questions {
        body.insert("return_related_questions".to_string(), json!(related));
    }
    if let Some(ref size) = opts.search_context_size {
        body.insert(
            "web_search_options".to_string(),
            json!({ "search_context_size": size }),
        );
    }
    if let Some(ref after) = opts.search_after_date_filter {
        body.insert("search_after_date_filter".to_string(), json!(after));
    }
    if let Some(ref before) = opts.search_before_date_filter {
        body.insert("search_before_date_filter".to_string(), json!(before));
    }
    if let Some(ref after) = opts.last_updated_after_filter {
        body.insert("last_updated_after_filter".to_string(), json!(after));
    }
    if let Some(ref before) = opts.last_updated_before_filter {
        body.insert("last_updated_before_filter".to_string(), json!(before));
    }
    if let Some(ref location) = opts.user_location {
        body.insert("user_location".to_string(), json!(location));
    }
}

pub(crate) fn parse_pplx_meta(raw: &Value) -> PerplexityMetadata {
    let citations: Vec<String> = raw
        .get("citations")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    let search_results: Vec<SearchResult> = raw
        .get("search_results")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    let annotations: Vec<Annotation> = raw
        .pointer("/choices/0/message/annotations")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    let related_questions: Vec<String> = raw
        .get("related_questions")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    PerplexityMetadata {
        citations,
        search_results,
        annotations,
        related_questions,
    }
}

pub(crate) fn metadata_is_empty(meta: &PerplexityMetadata) -> bool {
    meta.citations.is_empty()
        && meta.search_results.is_empty()
        && meta.annotations.is_empty()
        && meta.related_questions.is_empty()
}
