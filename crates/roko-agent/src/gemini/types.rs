//! Gemini-native request and response types.
#![allow(missing_docs)]

use serde::{Deserialize, Serialize};

/// Reuse the canonical Gemini safety-setting shape from config.
pub use roko_core::config::schema::SafetySetting as SafetySettingRequest;

// ── Request types ─────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerateContentRequest {
    pub contents: Vec<Content>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_instruction: Option<Content>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<GeminiTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_config: Option<ToolConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generation_config: Option<GenerationConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub safety_settings: Option<Vec<SafetySettingRequest>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cached_content: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Content {
    pub role: String,
    pub parts: Vec<Part>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Part {
    Text {
        text: String,
    },
    FunctionCall {
        #[serde(rename = "functionCall")]
        function_call: FunctionCallPart,
    },
    FunctionResponse {
        #[serde(rename = "functionResponse")]
        function_response: FunctionResponsePart,
    },
    ExecutableCode {
        #[serde(rename = "executableCode")]
        executable_code: ExecutableCodePart,
    },
    CodeExecutionResult {
        #[serde(rename = "codeExecutionResult")]
        code_execution_result: CodeExecutionResultPart,
    },
    InlineData {
        #[serde(rename = "inlineData")]
        inline_data: InlineDataPart,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionCallPart {
    pub name: String,
    pub args: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionResponsePart {
    pub name: String,
    pub response: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutableCodePart {
    pub language: String,
    pub code: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeExecutionResultPart {
    pub outcome: String,
    pub output: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InlineDataPart {
    pub mime_type: String,
    pub data: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum GeminiTool {
    FunctionDeclarations {
        #[serde(rename = "functionDeclarations")]
        function_declarations: Vec<FunctionDeclaration>,
    },
    GoogleSearch {
        google_search: serde_json::Value,
    },
    CodeExecution {
        code_execution: serde_json::Value,
    },
    UrlContext {
        url_context: serde_json::Value,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionDeclaration {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolConfig {
    pub function_calling_config: FunctionCallingConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionCallingConfig {
    pub mode: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_function_names: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerationConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_sequences: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_mime_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_schema: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking_config: Option<ThinkingConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThinkingConfig {
    pub thinking_level: String,
}

// ── Response types ────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerateContentResponse {
    pub candidates: Vec<Candidate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage_metadata: Option<UsageMetadata>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    pub content: Content,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
    #[serde(default)]
    pub safety_ratings: Vec<SafetyRating>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grounding_metadata: Option<GroundingMetadata>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroundingMetadata {
    /// Search queries the model executed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub web_search_queries: Option<Vec<String>>,
    /// Source chunks from search results.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grounding_chunks: Option<Vec<GroundingChunk>>,
    /// Maps response text spans to source chunks (inline citations).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grounding_supports: Option<Vec<GroundingSupport>>,
    /// HTML/CSS for rendering search suggestions (ToS requirement).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search_entry_point: Option<SearchEntryPoint>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroundingChunk {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub web: Option<WebChunk>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebChunk {
    pub uri: String,
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroundingSupport {
    pub segment: TextSegment,
    pub grounding_chunk_indices: Vec<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence_scores: Option<Vec<f64>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextSegment {
    pub start_index: usize,
    pub end_index: usize,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchEntryPoint {
    pub rendered_content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageMetadata {
    pub prompt_token_count: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub candidates_token_count: Option<u64>,
    pub total_token_count: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cached_content_token_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking_token_count: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SafetyRating {
    pub category: String,
    pub probability: String,
}

// ── Adapter metadata ──────────────────────────────────

/// Gemini-specific metadata preserved in ChatResponse.metadata.extra.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeminiMetadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grounding_metadata: Option<GroundingMetadata>,
    #[serde(default)]
    pub code_execution_results: Vec<CodeExecutionResultPart>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cached_tokens: Option<u64>,
    #[serde(default)]
    pub safety_ratings: Vec<SafetyRating>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn example_gemini_response_json() -> serde_json::Value {
        json!({
            "candidates": [
                {
                    "content": {
                        "role": "model",
                        "parts": [
                            {
                                "text": "I checked the latest Rust docs and validated the result."
                            },
                            {
                                "functionCall": {
                                    "name": "web_search",
                                    "args": {
                                        "query": "Rust edition 2024 let chains"
                                    },
                                    "id": "call-1"
                                }
                            },
                            {
                                "codeExecutionResult": {
                                    "outcome": "OUTCOME_OK",
                                    "output": "verification passed"
                                }
                            }
                        ]
                    },
                    "finishReason": "STOP",
                    "safetyRatings": [
                        {
                            "category": "HARM_CATEGORY_HARASSMENT",
                            "probability": "NEGLIGIBLE"
                        }
                    ],
                    "groundingMetadata": {
                        "webSearchQueries": [
                            "Rust edition 2024 let chains"
                        ],
                        "groundingChunks": [
                            {
                                "web": {
                                    "uri": "https://doc.rust-lang.org/edition-guide/rust-2024/",
                                    "title": "Rust Edition Guide"
                                }
                            }
                        ],
                        "groundingSupports": [
                            {
                                "segment": {
                                    "startIndex": 0,
                                    "endIndex": 43,
                                    "text": "I checked the latest Rust docs and validated"
                                },
                                "groundingChunkIndices": [0],
                                "confidenceScores": [0.98]
                            }
                        ],
                        "searchEntryPoint": {
                            "renderedContent": "<div>Search</div>"
                        }
                    }
                }
            ],
            "usageMetadata": {
                "promptTokenCount": 120,
                "candidatesTokenCount": 48,
                "totalTokenCount": 168,
                "cachedContentTokenCount": 12,
                "thinkingTokenCount": 9
            }
        })
    }

    #[test]
    fn gemini_types_response_round_trip() {
        let original = example_gemini_response_json();
        let parsed: GenerateContentResponse =
            serde_json::from_value(original.clone()).expect("deserialize GenerateContentResponse");

        assert_eq!(parsed.candidates.len(), 1);
        assert_eq!(parsed.candidates[0].content.role, "model");
        assert_eq!(parsed.candidates[0].content.parts.len(), 3);
        assert_eq!(parsed.candidates[0].finish_reason.as_deref(), Some("STOP"));
        assert_eq!(parsed.candidates[0].safety_ratings.len(), 1);
        assert_eq!(
            parsed.candidates[0]
                .grounding_metadata
                .as_ref()
                .and_then(|meta| meta.web_search_queries.as_ref())
                .and_then(|queries| queries.first())
                .map(String::as_str),
            Some("Rust edition 2024 let chains")
        );
        assert_eq!(
            parsed
                .usage_metadata
                .as_ref()
                .and_then(|usage| usage.thinking_token_count),
            Some(9)
        );

        let reserialized =
            serde_json::to_value(&parsed).expect("serialize GenerateContentResponse");
        assert_eq!(reserialized, original);
    }
}
