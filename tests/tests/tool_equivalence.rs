//! Cross-backend equivalence test for the tool registry.
//!
//! Verifies that tool definitions are consistent across the static
//! registry, that all 16 tools are present, that there are no duplicate
//! names, and that every schema is valid JSON.

use std::collections::HashSet;

use roko_core::tool::ToolRegistry;
use roko_std::{ROKO_BUILTIN_TOOLS, StaticToolRegistry, TOOL_COUNT};

#[test]
fn all_16_tools_present_in_static_registry() {
    let reg = StaticToolRegistry::new();
    assert_eq!(reg.all().len(), 16);
    assert_eq!(reg.len(), TOOL_COUNT);
}

#[test]
fn no_duplicate_names_in_registry() {
    let reg = StaticToolRegistry::new();
    let mut seen = HashSet::new();
    for def in reg.all() {
        assert!(
            seen.insert(def.name.clone()),
            "duplicate tool name: {}",
            def.name
        );
    }
    assert_eq!(seen.len(), TOOL_COUNT);
}

#[test]
fn all_schemas_are_valid_json_objects() {
    let reg = StaticToolRegistry::new();
    for def in reg.all() {
        let schema = def.parameters.as_value();
        assert!(
            schema.is_object(),
            "tool `{}` schema is not a JSON object: {:?}",
            def.name,
            schema
        );
        // Verify it round-trips through serde.
        let serialized =
            serde_json::to_string(schema).unwrap_or_else(|e| panic!("{}: {e}", def.name));
        let parsed: serde_json::Value = serde_json::from_str(&serialized)
            .unwrap_or_else(|e| panic!("{} re-parse: {e}", def.name));
        assert_eq!(
            schema, &parsed,
            "schema round-trip mismatch for `{}`",
            def.name
        );
    }
}

#[test]
fn registry_get_matches_all_iteration() {
    let reg = StaticToolRegistry::new();
    for def in reg.all() {
        let looked_up = reg.get(&def.name);
        assert!(
            looked_up.is_some(),
            "get({}) returned None but tool is in all()",
            def.name
        );
        let looked_up = looked_up.expect("just checked");
        assert_eq!(looked_up.name, def.name);
        assert_eq!(looked_up.description, def.description);
    }
}

#[test]
fn builtin_tools_and_registry_agree() {
    let reg = StaticToolRegistry::new();
    let reg_names: HashSet<&str> = reg.all().iter().map(|d| d.name.as_str()).collect();
    let builtin_names: HashSet<&str> = ROKO_BUILTIN_TOOLS.iter().map(|d| d.name.as_str()).collect();
    assert_eq!(
        reg_names, builtin_names,
        "registry and ROKO_BUILTIN_TOOLS disagree on tool names"
    );
}

#[test]
fn every_tool_has_type_object_schema() {
    let reg = StaticToolRegistry::new();
    for def in reg.all() {
        let ty = def
            .parameters
            .as_value()
            .get("type")
            .and_then(serde_json::Value::as_str);
        assert_eq!(
            ty,
            Some("object"),
            "tool `{}` schema missing `type: object`",
            def.name
        );
    }
}

#[test]
fn every_tool_has_nonempty_description() {
    let reg = StaticToolRegistry::new();
    for def in reg.all() {
        assert!(
            !def.description.is_empty(),
            "tool `{}` has empty description",
            def.name
        );
    }
}

/// Validate args: object → ok, non-object → err for every tool.
#[test]
fn validate_args_accepts_objects_rejects_non_objects() {
    let reg = StaticToolRegistry::new();
    for def in reg.all() {
        assert!(
            reg.validate_args(&def.name, &serde_json::json!({})).is_ok(),
            "validate_args({}, {{}}) should succeed",
            def.name
        );
        assert!(
            reg.validate_args(&def.name, &serde_json::json!("string"))
                .is_err(),
            "validate_args({}, \"string\") should fail",
            def.name
        );
    }
}
