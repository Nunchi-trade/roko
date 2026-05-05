# Plan: Dry-Run Flag for Workflow Execution Preview

## Summary

Implements `--dry-run` flag for `roko run` that resolves all configuration,
model selection, prompt assembly, and gate pipeline construction without
dispatching to any LLM or executing any gates. Produces a structured
`DryRunPreview` output (JSON or human-readable) and exits with code 0 on
success or 2 on config error.

## Task Dependency Graph
