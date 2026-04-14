//! Perplexity-specific response and request types.
//!
//! These types cover the extensions that Perplexity adds on top of the
//! OpenAI-compatible chat completions surface: citations, search results,
//! character-level annotations, and the Agent/Responses API.

use serde::{Deserialize, Serialize};

/// Citation data from Perplexity search-grounded responses.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerplexityMetadata {
    /// Source URLs cited in the response.
    pub citations: Vec<String>,
    /// Full search result entries with content snippets.
    pub search_results: Vec<SearchResult>,
    /// Character-level annotation spans linking text to sources.
    pub annotations: Vec<Annotation>,
    /// Related questions suggested by the model.
    pub related_questions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub url: String,
    pub title: String,
    pub content: String,
    pub date: Option<String>,
    pub last_updated: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Annotation {
    pub start_index: usize,
    pub end_index: usize,
    pub title: String,
    pub url: String,
}

/// Search filter options injected into Perplexity requests.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SearchOptions {
    pub search_domain_filter: Option<Vec<String>>,
    pub search_recency_filter: Option<String>,
    pub search_after_date_filter: Option<String>,
    pub search_before_date_filter: Option<String>,
    pub last_updated_after_filter: Option<String>,
    pub last_updated_before_filter: Option<String>,
    pub search_context_size: Option<String>,
    /// `"academic"` or `"web"`.
    pub search_mode: Option<String>,
    pub return_images: Option<bool>,
    pub return_related_questions: Option<bool>,
    pub user_location: Option<UserLocation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserLocation {
    pub country: Option<String>,
    pub city: Option<String>,
    pub region: Option<String>,
    pub timezone: Option<String>,
}

/// Request for the Perplexity Agent/Responses API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRequest {
    pub model: String,
    /// `"fast-search"`, `"pro-search"`, `"deep-research"`, or
    /// `"advanced-deep-research"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preset: Option<String>,
    /// Text string or message array.
    pub input: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
    #[serde(default)]
    pub stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<AgentTool>>,
    #[serde(flatten)]
    pub search_options: SearchOptions,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentTool {
    /// `"web_search"`, `"fetch_url"`, or `"function"`.
    #[serde(rename = "type")]
    pub tool_type: String,
    #[serde(flatten)]
    pub config: serde_json::Value,
}

/// Response from the Perplexity Agent/Responses API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentResponse {
    pub id: String,
    pub model: String,
    pub status: String,
    pub output: Vec<AgentOutputItem>,
    pub usage: AgentUsage,
    #[serde(default)]
    pub citations: Vec<String>,
    #[serde(default)]
    pub search_results: Vec<SearchResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentOutputItem {
    pub role: String,
    pub content: Vec<ContentBlock>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentBlock {
    #[serde(rename = "type")]
    pub content_type: String,
    pub text: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn example_perplexity_metadata_json() -> serde_json::Value {
        json!({
            "citations": [
                "https://example.com/paper1",
                "https://example.com/paper2"
            ],
            "search_results": [
                {
                    "url": "https://example.com/paper1",
                    "title": "Attention Is All You Need",
                    "content": "The dominant sequence transduction models...",
                    "date": "2017-06-12",
                    "last_updated": null
                },
                {
                    "url": "https://example.com/paper2",
                    "title": "BERT: Pre-training of Deep Bidirectional Transformers",
                    "content": "We introduce a new language representation model...",
                    "date": null,
                    "last_updated": "2019-05-24"
                }
            ],
            "annotations": [
                {
                    "start_index": 0,
                    "end_index": 42,
                    "title": "Attention Is All You Need",
                    "url": "https://example.com/paper1"
                }
            ],
            "related_questions": [
                "What is the transformer architecture?",
                "How does BERT differ from GPT?"
            ]
        })
    }

    fn example_agent_response_json() -> serde_json::Value {
        json!({
            "id": "resp-abc123",
            "model": "sonar-pro",
            "status": "completed",
            "output": [
                {
                    "role": "assistant",
                    "content": [
                        {
                            "type": "text",
                            "text": "Transformers use self-attention mechanisms..."
                        }
                    ]
                }
            ],
            "usage": {
                "input_tokens": 128,
                "output_tokens": 64,
                "total_tokens": 192
            },
            "citations": ["https://example.com/paper1"],
            "search_results": [
                {
                    "url": "https://example.com/paper1",
                    "title": "Attention Is All You Need",
                    "content": "The dominant sequence transduction models...",
                    "date": "2017-06-12",
                    "last_updated": null
                }
            ]
        })
    }

    #[test]
    fn perplexity_metadata_round_trip() {
        let original = example_perplexity_metadata_json();
        let parsed: PerplexityMetadata =
            serde_json::from_value(original.clone()).expect("deserialize PerplexityMetadata");

        assert_eq!(parsed.citations.len(), 2);
        assert_eq!(parsed.citations[0], "https://example.com/paper1");
        assert_eq!(parsed.search_results.len(), 2);
        assert_eq!(parsed.search_results[0].title, "Attention Is All You Need");
        assert_eq!(
            parsed.search_results[0].date,
            Some("2017-06-12".to_string())
        );
        assert!(parsed.search_results[0].last_updated.is_none());
        assert_eq!(parsed.annotations.len(), 1);
        assert_eq!(parsed.annotations[0].start_index, 0);
        assert_eq!(parsed.annotations[0].end_index, 42);
        assert_eq!(parsed.related_questions.len(), 2);

        let reserialised = serde_json::to_value(&parsed).expect("serialize PerplexityMetadata");
        assert_eq!(reserialised, original);
    }

    #[test]
    fn agent_response_round_trip() {
        let original = example_agent_response_json();
        let parsed: AgentResponse =
            serde_json::from_value(original.clone()).expect("deserialize AgentResponse");

        assert_eq!(parsed.id, "resp-abc123");
        assert_eq!(parsed.model, "sonar-pro");
        assert_eq!(parsed.status, "completed");
        assert_eq!(parsed.output.len(), 1);
        assert_eq!(parsed.output[0].role, "assistant");
        assert_eq!(parsed.output[0].content.len(), 1);
        assert_eq!(parsed.output[0].content[0].content_type, "text");
        assert_eq!(
            parsed.output[0].content[0].text.as_deref(),
            Some("Transformers use self-attention mechanisms...")
        );
        assert_eq!(parsed.usage.input_tokens, 128);
        assert_eq!(parsed.usage.output_tokens, 64);
        assert_eq!(parsed.usage.total_tokens, 192);
        assert_eq!(parsed.citations.len(), 1);
        assert_eq!(parsed.search_results.len(), 1);

        let reserialised = serde_json::to_value(&parsed).expect("serialize AgentResponse");
        assert_eq!(reserialised, original);
    }

    #[test]
    fn agent_response_defaults_empty_citations_and_results() {
        let json = json!({
            "id": "resp-xyz",
            "model": "sonar",
            "status": "completed",
            "output": [],
            "usage": { "input_tokens": 10, "output_tokens": 5, "total_tokens": 15 }
        });
        let parsed: AgentResponse =
            serde_json::from_value(json).expect("deserialize AgentResponse without optionals");
        assert!(parsed.citations.is_empty());
        assert!(parsed.search_results.is_empty());
    }

    #[test]
    fn search_options_defaults_all_none() {
        let opts = SearchOptions::default();
        let serialised = serde_json::to_value(&opts).expect("serialize SearchOptions");
        // All fields are Option<_>, so a default should round-trip cleanly.
        let parsed: SearchOptions =
            serde_json::from_value(serialised).expect("deserialize SearchOptions");
        assert!(parsed.search_domain_filter.is_none());
        assert!(parsed.search_mode.is_none());
        assert!(parsed.user_location.is_none());
    }

    #[test]
    fn agent_request_skip_none_fields_in_serialization() {
        let req = AgentRequest {
            model: "sonar".to_string(),
            preset: None,
            input: json!("What is quantum entanglement?"),
            instructions: None,
            max_output_tokens: None,
            stream: false,
            tools: None,
            search_options: SearchOptions::default(),
        };
        let val = serde_json::to_value(&req).expect("serialize AgentRequest");
        assert!(!val.as_object().unwrap().contains_key("preset"));
        assert!(!val.as_object().unwrap().contains_key("instructions"));
        assert!(!val.as_object().unwrap().contains_key("tools"));
        assert_eq!(val["model"], "sonar");
    }
}
