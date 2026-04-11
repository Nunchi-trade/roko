use roko_agent::translate::openai::parse_glm_metadata;

#[test]
fn glm_web_search_response() {
    let raw = serde_json::json!({
        "choices": [{
            "message": {
                "role": "assistant",
                "content": "Here are the results."
            }
        }],
        "web_search": [
            {
                "title": "Result 1",
                "link": "https://example.com/result-1",
                "content": "First result snippet"
            }
        ]
    });

    let metadata = parse_glm_metadata(&raw);

    assert_eq!(
        metadata.web_search,
        Some(serde_json::json!([
            {
                "title": "Result 1",
                "link": "https://example.com/result-1",
                "content": "First result snippet"
            }
        ]))
    );
}
