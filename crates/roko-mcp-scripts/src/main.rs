//! MCP server for executing scripts from a configured directory.
//!
//! Exposes a single `run_script` tool that resolves a script beneath the
//! configured root directory, executes it, and returns captured stdout/stderr
//! in the MCP tool result payload.

use roko_mcp_stdio::{JsonRpcError, JsonRpcRequest, serve_stdio};
use serde::Deserialize;
use serde_json::{Value, json};
use std::env;
use std::ffi::OsStr;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone)]
struct AppConfig {
    working_dir: PathBuf,
}

#[derive(Debug, Deserialize)]
struct ToolsCallParams {
    name: String,
    #[serde(default = "empty_json_object")]
    arguments: Value,
}

#[derive(Debug, Deserialize)]
struct RunScriptArguments {
    name: String,
    #[serde(default)]
    args: Vec<String>,
}

#[derive(Debug)]
struct ScriptExecution {
    command: String,
    script_path: PathBuf,
    args: Vec<String>,
    stdout: String,
    stderr: String,
    exit_code: i32,
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("roko_mcp_scripts=info")
        .with_writer(io::stderr)
        .init();

    let config = AppConfig::from_env()?;
    serve_stdio(io::stdin().lock(), io::stdout().lock(), |request| {
        handle_request(request, &config)
    })?;
    Ok(())
}

fn handle_request(request: JsonRpcRequest, config: &AppConfig) -> Result<Value, JsonRpcError> {
    match request.method.as_str() {
        "initialize" => Ok(handle_initialize()),
        "tools/list" => Ok(handle_tools_list()),
        "tools/call" => handle_tools_call(request.params, config),
        _ => Err(JsonRpcError::method_not_found(&request.method)),
    }
}

fn handle_initialize() -> Value {
    json!({
        "protocolVersion": "2024-11-05",
        "capabilities": {
            "tools": {}
        },
        "serverInfo": {
            "name": "roko-mcp-scripts",
            "version": env!("CARGO_PKG_VERSION")
        }
    })
}

fn handle_tools_list() -> Value {
    json!({
        "tools": [{
            "name": "run_script",
            "description": "Execute a script from the configured directory and return stdout/stderr.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Script name or relative path beneath the configured directory."
                    },
                    "args": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Optional command-line arguments passed to the script."
                    }
                },
                "required": ["name"],
                "additionalProperties": false
            }
        }]
    })
}

fn handle_tools_call(params: Value, config: &AppConfig) -> Result<Value, JsonRpcError> {
    let params: ToolsCallParams = serde_json::from_value(params)
        .map_err(|err| JsonRpcError::invalid_params(format!("invalid tools/call params: {err}")))?;
    match params.name.as_str() {
        "run_script" => handle_run_script(params.arguments, config),
        _ => Err(JsonRpcError::invalid_params(format!(
            "unknown tool: {}",
            params.name
        ))),
    }
}

fn handle_run_script(arguments: Value, config: &AppConfig) -> Result<Value, JsonRpcError> {
    let args: RunScriptArguments = serde_json::from_value(arguments)
        .map_err(|err| JsonRpcError::invalid_params(format!("invalid run_script args: {err}")))?;

    let execution = execute_script(config, &args.name, &args.args);
    Ok(make_tool_result(execution))
}

fn execute_script(config: &AppConfig, name: &str, args: &[String]) -> ScriptExecution {
    let working_dir = config.working_dir.clone();
    let resolved = match resolve_script_path(&working_dir, name) {
        Ok(path) => path,
        Err(err) => {
            return ScriptExecution {
                command: "run_script".to_string(),
                script_path: working_dir.join(name),
                args: args.to_vec(),
                stdout: String::new(),
                stderr: err,
                exit_code: 127,
            };
        }
    };

    let (command, command_args) = command_for_script(&resolved, args);
    let output = match Command::new(&command)
        .args(&command_args)
        .current_dir(&working_dir)
        .output()
    {
        Ok(output) => output,
        Err(err) => {
            return ScriptExecution {
                command,
                script_path: resolved,
                args: args.to_vec(),
                stdout: String::new(),
                stderr: format!("failed to spawn script: {err}"),
                exit_code: 127,
            };
        }
    };

    ScriptExecution {
        command,
        script_path: resolved,
        args: args.to_vec(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        exit_code: output.status.code().unwrap_or(-1),
    }
}

fn make_tool_result(execution: ScriptExecution) -> Value {
    let is_error = execution.exit_code != 0;
    let payload = json!({
        "command": execution.command,
        "script": execution.script_path.to_string_lossy(),
        "args": execution.args,
        "exit_code": execution.exit_code,
        "stdout": execution.stdout,
        "stderr": execution.stderr,
    });

    json!({
        "content": [{
            "type": "text",
            "text": payload.to_string(),
        }],
        "isError": is_error
    })
}

fn resolve_script_path(working_dir: &Path, name: &str) -> Result<PathBuf, String> {
    let requested = Path::new(name);
    let mut candidates = Vec::new();

    if requested.components().count() > 1 || requested.extension().is_some() {
        candidates.push(working_dir.join(requested));
    } else {
        candidates.push(working_dir.join(requested));
        candidates.push(working_dir.join("scripts").join(requested));
        for extension in ["sh", "py", "js", "rb"] {
            candidates.push(working_dir.join(requested.with_extension(extension)));
            candidates.push(
                working_dir
                    .join("scripts")
                    .join(requested.with_extension(extension)),
            );
        }
    }

    let canonical_working_dir = std::fs::canonicalize(working_dir)
        .unwrap_or_else(|_| working_dir.to_path_buf());

    for candidate in candidates {
        if !candidate.is_file() {
            continue;
        }

        let canonical_candidate = std::fs::canonicalize(&candidate)
            .map_err(|err| format!("failed to resolve script '{}': {err}", candidate.display()))?;

        if !canonical_candidate.starts_with(&canonical_working_dir) {
            return Err(format!(
                "script '{}' resolves outside configured directory",
                name
            ));
        }

        return Ok(canonical_candidate);
    }

    Err(format!(
        "script '{}' not found under {}",
        name,
        working_dir.display()
    ))
}

fn command_for_script(script_path: &Path, args: &[String]) -> (String, Vec<String>) {
    let command = match script_path.extension().and_then(OsStr::to_str) {
        Some("py") => "python3",
        Some("sh") => "bash",
        Some("js") => "node",
        _ => {
            return (
                script_path.to_string_lossy().into_owned(),
                args.to_vec(),
            );
        }
    };

    let mut command_args = Vec::with_capacity(args.len() + 1);
    command_args.push(script_path.to_string_lossy().into_owned());
    command_args.extend_from_slice(args);
    (command.to_string(), command_args)
}

fn empty_json_object() -> Value {
    Value::Object(Default::default())
}

impl AppConfig {
    fn from_env() -> anyhow::Result<Self> {
        let mut working_dir = env::var("ROKO_MCP_SCRIPTS_DIR")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .map(PathBuf::from)
            .unwrap_or(env::current_dir()?);
        let mut args = env::args_os().skip(1);

        while let Some(arg) = args.next() {
            let arg = arg.to_string_lossy().into_owned();
            match arg.as_str() {
                "--working-dir" | "--scripts-dir" => {
                    let value = args.next().ok_or_else(|| {
                        anyhow::anyhow!("missing value for {arg}")
                    })?;
                    working_dir = PathBuf::from(value);
                }
                value if value.starts_with("--working-dir=") => {
                    working_dir = PathBuf::from(value.trim_start_matches("--working-dir="));
                }
                value if value.starts_with("--scripts-dir=") => {
                    working_dir = PathBuf::from(value.trim_start_matches("--scripts-dir="));
                }
                _ => {}
            }
        }

        Ok(Self { working_dir })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn resolves_script_in_scripts_subdirectory() {
        let dir = temp_dir();
        let scripts_dir = dir.join("scripts");
        fs::create_dir_all(&scripts_dir).expect("create scripts dir");
        let script = scripts_dir.join("hello.sh");
        fs::write(&script, "#!/bin/sh\necho hello\n").expect("write script");

        let resolved = resolve_script_path(&dir, "hello").expect("resolved");
        assert_eq!(resolved, fs::canonicalize(script).expect("canonical script"));
    }

    #[test]
    fn command_for_python_script_uses_python3() {
        let script = Path::new("/tmp/example.py");
        let (command, args) = command_for_script(script, &["one".into(), "two".into()]);
        assert_eq!(command, "python3");
        assert_eq!(args, vec!["/tmp/example.py", "one", "two"]);
    }

    fn temp_dir() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let dir = env::temp_dir().join(format!("roko-mcp-scripts-{nanos}"));
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }
}
