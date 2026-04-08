//! [`StaticToolRegistry`] — compile-time registry of the 16 built-ins (§36.9).
//!
//! This unit struct implements [`roko_core::tool::ToolRegistry`] over
//! the shared [`super::builtin::ROKO_BUILTIN_TOOLS`] slice. Every Roko
//! deployment uses this registry as its base tool set; config-driven
//! extensions (MCP tools, role overrides) layer on top through a
//! separate compound registry (shipped in a later phase).

use roko_core::error::{Result, RokoError};
use roko_core::tool::{ToolDef, ToolRegistry};

use super::builtin::ROKO_BUILTIN_TOOLS;

/// Registry of the 16 built-in Roko tools (§36.b).
///
/// Zero-sized: the definitions live in the
/// [`ROKO_BUILTIN_TOOLS`] static. Lookups are a linear scan —
/// 16 entries is small enough that a hashmap would be slower after
/// allocation overhead.
#[derive(Debug, Clone, Copy, Default)]
pub struct StaticToolRegistry;

impl StaticToolRegistry {
    /// Construct the registry. Cheap (zero-sized).
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Number of built-in tools — always [`super::builtin::TOOL_COUNT`].
    #[must_use]
    pub fn len(&self) -> usize {
        ROKO_BUILTIN_TOOLS.len()
    }

    /// Is the registry empty? Always `false` for the static registry.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        ROKO_BUILTIN_TOOLS.is_empty()
    }
}

impl ToolRegistry for StaticToolRegistry {
    fn get(&self, name: &str) -> Option<&ToolDef> {
        ROKO_BUILTIN_TOOLS.iter().find(|t| t.name == name)
    }

    fn all(&self) -> &[ToolDef] {
        ROKO_BUILTIN_TOOLS.as_slice()
    }

    fn validate_args(&self, name: &str, args: &serde_json::Value) -> Result<()> {
        let def = self
            .get(name)
            .ok_or_else(|| RokoError::invalid(format!("unknown tool: {name}")))?;
        validate_against_schema(&def.parameters, args)
    }
}

/// Validate `args` against a tool's JSON schema.
///
/// Day-one implementation: checks the top-level `type` constraint (the
/// only constraint the §36.9 stub schemas set). Real JSON-schema
/// validation — required-property enforcement, nested types, enums —
/// ships with the concrete handler schemas in §36.b via the
/// `jsonschema` crate.
fn validate_against_schema(
    schema: &roko_core::tool::ToolSchema,
    args: &serde_json::Value,
) -> Result<()> {
    let schema_value = schema.as_value();
    let expected_type = schema_value.get("type").and_then(serde_json::Value::as_str);
    match expected_type {
        Some("object") => {
            if args.is_object() {
                Ok(())
            } else {
                Err(RokoError::invalid(format!(
                    "schema validation failed: expected object, got {}",
                    json_kind(args)
                )))
            }
        }
        Some("array") => {
            if args.is_array() {
                Ok(())
            } else {
                Err(RokoError::invalid(format!(
                    "schema validation failed: expected array, got {}",
                    json_kind(args)
                )))
            }
        }
        // Unknown or missing "type" → permissive until §36.b ships real schemas.
        _ => Ok(()),
    }
}

const fn json_kind(v: &serde_json::Value) -> &'static str {
    match v {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "bool",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

#[cfg(test)]
mod tests {
    use super::super::builtin::{BUILTIN_TOOL_NAMES, TOOL_COUNT};
    use super::*;
    use roko_core::AgentRole;
    use std::collections::HashSet;

    #[test]
    fn builtin_count_is_sixteen() {
        assert_eq!(TOOL_COUNT, 16);
        assert_eq!(ROKO_BUILTIN_TOOLS.len(), TOOL_COUNT);
        assert_eq!(BUILTIN_TOOL_NAMES.len(), TOOL_COUNT);
    }

    #[test]
    fn all_len_matches_tool_count() {
        let reg = StaticToolRegistry::new();
        assert_eq!(reg.all().len(), TOOL_COUNT);
        assert_eq!(reg.len(), TOOL_COUNT);
        assert!(!reg.is_empty());
    }

    #[test]
    fn no_duplicate_names() {
        let mut seen = HashSet::new();
        for t in ROKO_BUILTIN_TOOLS.iter() {
            assert!(
                seen.insert(t.name.clone()),
                "duplicate tool name: {}",
                t.name
            );
        }
        assert_eq!(seen.len(), TOOL_COUNT);
    }

    #[test]
    fn builtin_tool_names_match_definitions() {
        let reg = StaticToolRegistry::new();
        for (i, name) in BUILTIN_TOOL_NAMES.iter().enumerate() {
            let def = reg.get(name).expect("name must be present");
            assert_eq!(def.name, *name, "index {i}");
        }
    }

    #[test]
    fn get_missing_returns_none() {
        let reg = StaticToolRegistry::new();
        assert!(reg.get("definitely_not_a_real_tool").is_none());
    }

    #[test]
    fn get_existing_returns_some() {
        let reg = StaticToolRegistry::new();
        assert!(reg.get("read_file").is_some());
        assert!(reg.get("bash").is_some());
        assert!(reg.get("task").is_some());
    }

    #[test]
    fn validate_unknown_tool_is_err() {
        let reg = StaticToolRegistry::new();
        let result = reg.validate_args("zzz_no_such_tool", &serde_json::json!({}));
        assert!(result.is_err());
        let err = result.expect_err("validation should fail");
        let msg = format!("{err}");
        assert!(msg.contains("unknown tool"));
    }

    #[test]
    fn validate_known_tool_with_object_args_is_ok() {
        let reg = StaticToolRegistry::new();
        assert!(
            reg.validate_args("read_file", &serde_json::json!({}))
                .is_ok()
        );
        assert!(
            reg.validate_args("bash", &serde_json::json!({"command": "ls"}))
                .is_ok()
        );
    }

    #[test]
    fn validate_known_tool_with_non_object_args_is_err() {
        let reg = StaticToolRegistry::new();
        // Schemas declare `type: "object"` via `ToolSchema::any_object`,
        // so non-object args must fail.
        let result = reg.validate_args("read_file", &serde_json::json!("not an object"));
        assert!(result.is_err());
        let err = result.expect_err("validation should fail");
        let msg = format!("{err}");
        assert!(msg.contains("schema validation failed"));
    }

    #[test]
    fn for_role_implementer_is_nonempty() {
        let reg = StaticToolRegistry::new();
        let tools = reg.for_role(AgentRole::Implementer);
        assert!(
            !tools.is_empty(),
            "Implementer should see at least one tool"
        );
        // Implementer has read + write + exec → should see write_file and bash.
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"read_file"));
        assert!(names.contains(&"write_file"));
        assert!(names.contains(&"bash"));
    }

    #[test]
    fn for_role_auditor_is_read_only_subset() {
        let reg = StaticToolRegistry::new();
        let tools = reg.for_role(AgentRole::Auditor);
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        // Auditor is read-only → read_file/grep/glob/ls visible; write_file/bash not.
        assert!(names.contains(&"read_file"));
        assert!(names.contains(&"grep"));
        assert!(names.contains(&"glob"));
        assert!(names.contains(&"ls"));
        assert!(!names.contains(&"write_file"));
        assert!(!names.contains(&"bash"));
        assert!(!names.contains(&"web_fetch"));
    }

    #[test]
    fn for_role_preserves_allowlist_invariants() {
        let reg = StaticToolRegistry::new();
        // Every role enumerates without panic and returns a subset of all().
        let all_roles: Vec<AgentRole> = std::iter::once(AgentRole::Conductor)
            .chain(AgentRole::ALL_AGENTS.iter().copied())
            .collect();
        for role in all_roles {
            let tools = reg.for_role(role);
            assert!(tools.len() <= TOOL_COUNT);
        }
    }
}
