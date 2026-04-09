//! MCP server for executing scripts from a configured directory.
//!
//! Exposes `run_script` and `list_scripts` tools for scripts beneath the
//! configured root directory. Scripts can be listed with descriptions from
//! a `# description:` comment near the top of the file, executed, and return
//! captured stdout/stderr in the MCP tool result payload.

use roko_mcp_stdio::{JsonRpcError, JsonRpcRequest, serve_stdio};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::env;
use std::ffi::OsStr;
use std::io;
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::process::Command;
use tokio::time::timeout;

#[derive(Debug, Clone)]
struct AppConfig {
    working_dir: PathBuf,
    timeout: Duration,
    env_allowlist: BTreeSet<String>,
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

#[derive(Debug, Serialize)]
struct ScriptEntry {
    name: String,
    description: String,
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
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    serve_stdio(io::stdin().lock(), io::stdout().lock(), |request| {
        handle_request(request, &config, &runtime)
    })?;
    Ok(())
}

fn handle_request(
    request: JsonRpcRequest,
    config: &AppConfig,
    runtime: &tokio::runtime::Runtime,
) -> Result<Value, JsonRpcError> {
    match request.method.as_str() {
        "initialize" => Ok(handle_initialize()),
        "tools/list" => Ok(handle_tools_list()),
        "tools/call" => handle_tools_call(request.params, config, runtime),
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
        }, {
            "name": "list_scripts",
            "description": "List scripts available in the configured directory with descriptions.",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }
        }]
    })
}

fn handle_tools_call(
    params: Value,
    config: &AppConfig,
    runtime: &tokio::runtime::Runtime,
) -> Result<Value, JsonRpcError> {
    let params: ToolsCallParams = serde_json::from_value(params)
        .map_err(|err| JsonRpcError::invalid_params(format!("invalid tools/call params: {err}")))?;
    match params.name.as_str() {
        "run_script" => handle_run_script(params.arguments, config, runtime),
        "list_scripts" => handle_list_scripts(config),
        _ => Err(JsonRpcError::invalid_params(format!(
            "unknown tool: {}",
            params.name
        ))),
    }
}

fn handle_run_script(
    arguments: Value,
    config: &AppConfig,
    runtime: &tokio::runtime::Runtime,
) -> Result<Value, JsonRpcError> {
    let args: RunScriptArguments = serde_json::from_value(arguments)
        .map_err(|err| JsonRpcError::invalid_params(format!("invalid run_script args: {err}")))?;

    let execution = runtime.block_on(execute_script(config, &args.name, &args.args));
    Ok(make_tool_result(execution))
}

fn handle_list_scripts(config: &AppConfig) -> Result<Value, JsonRpcError> {
    let scripts = list_scripts(&config.working_dir).map_err(|err| {
        JsonRpcError::internal_error(format!("failed to list scripts: {err}"))
    })?;

    Ok(json!({
        "content": [{
            "type": "text",
            "text": json!({ "scripts": scripts }).to_string(),
        }],
        "isError": false
    }))
}

async fn execute_script(config: &AppConfig, name: &str, args: &[String]) -> ScriptExecution {
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
    let mut cmd = Command::new(&command);
    cmd.args(&command_args)
        .current_dir(&working_dir)
        .kill_on_drop(true);
    apply_env_allowlist(&mut cmd, &config.env_allowlist);

    let output = match timeout(config.timeout, cmd.output()).await {
        Ok(Ok(output)) => output,
        Ok(Err(err)) => {
            return ScriptExecution {
                command,
                script_path: resolved,
                args: args.to_vec(),
                stdout: String::new(),
                stderr: format!("failed to run script: {err}"),
                exit_code: 127,
            };
        }
        Err(_) => {
            return ScriptExecution {
                command,
                script_path: resolved,
                args: args.to_vec(),
                stdout: String::new(),
                stderr: format!("script timed out after {}s", config.timeout.as_secs()),
                exit_code: 124,
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

fn apply_env_allowlist(cmd: &mut Command, env_allowlist: &BTreeSet<String>) {
    cmd.env_clear();
    for (key, value) in filtered_env_entries(env_allowlist, env::vars()) {
        cmd.env(key, value);
    }
}

fn filtered_env_entries(
    env_allowlist: &BTreeSet<String>,
    source: impl IntoIterator<Item = (String, String)>,
) -> Vec<(String, String)> {
    source
        .into_iter()
        .filter(|(key, _)| env_allowlist.contains(key) || key == "PATH")
        .collect()
}

fn default_env_allowlist() -> BTreeSet<String> {
    BTreeSet::from([String::from("PATH")])
}

fn parse_timeout_secs(value: impl AsRef<str>) -> anyhow::Result<u64> {
    let value = value.as_ref().trim();
    let secs = value
        .parse::<u64>()
        .map_err(|err| anyhow::anyhow!("invalid timeout secs '{value}': {err}"))?;
    Ok(secs)
}

fn parse_env_allowlist(value: impl AsRef<str>) -> BTreeSet<String> {
    let mut allowlist = BTreeSet::new();
    for key in value.as_ref().split(',') {
        let key = key.trim();
        if !key.is_empty() {
            allowlist.insert(key.to_string());
        }
    }
    allowlist.insert("PATH".to_string());
    allowlist
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

fn list_scripts(root: &Path) -> io::Result<Vec<ScriptEntry>> {
    let canonical_root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let mut scripts = Vec::new();
    collect_scripts(&canonical_root, &canonical_root, &mut scripts)?;
    scripts.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(scripts)
}

fn collect_scripts(
    root: &Path,
    dir: &Path,
    scripts: &mut Vec<ScriptEntry>,
) -> io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;

        if file_type.is_dir() {
            collect_scripts(root, &path, scripts)?;
            continue;
        }

        if !file_type.is_file() {
            continue;
        }

        if !is_supported_script(&path) {
            continue;
        }

        let canonical = match std::fs::canonicalize(&path) {
            Ok(path) => path,
            Err(_) => continue,
        };

        if !canonical.starts_with(root) {
            continue;
        }

        let rel = match canonical.strip_prefix(root) {
            Ok(rel) => rel,
            Err(_) => continue,
        };

        let name = rel.to_string_lossy().replace('\\', "/");
        let description = read_script_description(&canonical)?.unwrap_or_default();
        scripts.push(ScriptEntry { name, description });
    }

    Ok(())
}

fn is_supported_script(path: &Path) -> bool {
    matches!(
        path.extension().and_then(OsStr::to_str),
        Some("sh" | "py" | "js" | "rb")
    )
}

fn read_script_description(path: &Path) -> io::Result<Option<String>> {
    let file = std::fs::File::open(path)?;
    let mut lines = io::BufReader::new(file).lines();

    let first = lines.next().transpose()?;
    let second = lines.next().transpose()?;
    let candidates = [first, second];

    for line in candidates.into_iter().flatten() {
        let trimmed = line.trim();
        if let Some(description) = trimmed.strip_prefix("# description:") {
            let description = description.trim();
            if !description.is_empty() {
                return Ok(Some(description.to_string()));
            }
            return Ok(Some(String::new()));
        }
    }

    Ok(None)
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
        let mut working_dir = env::var("ROKO_SCRIPTS_DIR")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .map(PathBuf::from)
            .or_else(|| {
                env::var("ROKO_MCP_SCRIPTS_DIR")
                    .ok()
                    .filter(|value| !value.trim().is_empty())
                    .map(PathBuf::from)
            })
            .unwrap_or_else(|| {
                env::current_dir()
                    .map(|dir| dir.join(".roko/scripts"))
                    .unwrap_or_else(|_| PathBuf::from(".roko/scripts"))
            });
        let mut timeout = env::var("ROKO_MCP_SCRIPTS_TIMEOUT_SECS")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .map(|value| parse_timeout_secs(value))
            .transpose()?
            .map(Duration::from_secs)
            .unwrap_or_else(|| Duration::from_secs(60));
        let mut env_allowlist = env::var("ROKO_MCP_SCRIPTS_ENV_ALLOWLIST")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .map(|value| parse_env_allowlist(&value))
            .unwrap_or_else(default_env_allowlist);
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
                "--timeout-secs" => {
                    let value = args.next().ok_or_else(|| anyhow::anyhow!("missing value for {arg}"))?;
                    timeout = Duration::from_secs(parse_timeout_secs(value.to_string_lossy())?);
                }
                value if value.starts_with("--timeout-secs=") => {
                    timeout = Duration::from_secs(parse_timeout_secs(
                        value.trim_start_matches("--timeout-secs="),
                    )?);
                }
                "--env-allowlist" => {
                    let value = args.next().ok_or_else(|| anyhow::anyhow!("missing value for {arg}"))?;
                    env_allowlist = parse_env_allowlist(value.to_string_lossy());
                }
                value if value.starts_with("--env-allowlist=") => {
                    env_allowlist =
                        parse_env_allowlist(value.trim_start_matches("--env-allowlist="));
                }
                _ => {}
            }
        }

        Ok(Self {
            working_dir,
            timeout,
            env_allowlist,
        })
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
    fn list_scripts_discovers_scripts_with_descriptions() {
        let dir = temp_dir();
        let scripts_dir = dir.join("scripts");
        fs::create_dir_all(&scripts_dir).expect("create scripts dir");

        let script = scripts_dir.join("hello.sh");
        fs::write(
            &script,
            "#!/bin/bash\n# description: say hello\nprintf 'hello\\n'\n",
        )
        .expect("write script");

        let nested_dir = scripts_dir.join("nested");
        fs::create_dir_all(&nested_dir).expect("create nested dir");
        let nested_script = nested_dir.join("build.py");
        fs::write(
            &nested_script,
            "#!/usr/bin/env python3\n# description: build things\nprint('build')\n",
        )
        .expect("write nested script");

        let scripts = list_scripts(&scripts_dir).expect("list scripts");
        assert_eq!(scripts.len(), 2);
        assert_eq!(scripts[0].name, "hello.sh");
        assert_eq!(scripts[0].description, "say hello");
        assert_eq!(scripts[1].name, "nested/build.py");
        assert_eq!(scripts[1].description, "build things");
    }

    #[test]
    fn read_script_description_uses_first_non_shebang_line() {
        let dir = temp_dir();
        let script = dir.join("tool.sh");
        fs::write(
            &script,
            "#!/bin/bash\n# description: first line comment\n",
        )
        .expect("write script");

        assert_eq!(
            read_script_description(&script).expect("description"),
            Some("first line comment".into())
        );
    }

    #[test]
    fn command_for_python_script_uses_python3() {
        let script = Path::new("/tmp/example.py");
        let (command, args) = command_for_script(script, &["one".into(), "two".into()]);
        assert_eq!(command, "python3");
        assert_eq!(args, vec!["/tmp/example.py", "one", "two"]);
    }

    #[test]
    fn filtered_env_entries_keeps_only_allowlisted_keys_and_path() {
        let allowlist = BTreeSet::from([String::from("KEEP")]);
        let source = vec![
            (String::from("KEEP"), String::from("yes")),
            (String::from("DROP"), String::from("no")),
            (String::from("PATH"), String::from("/usr/bin")),
        ];

        let filtered = filtered_env_entries(&allowlist, source);
        assert!(filtered.contains(&(String::from("KEEP"), String::from("yes"))));
        assert!(filtered.contains(&(String::from("PATH"), String::from("/usr/bin"))));
        assert!(!filtered.contains(&(String::from("DROP"), String::from("no"))));
    }

    #[tokio::test]
    async fn execute_script_times_out_and_reports_failure() {
        let dir = temp_dir();
        let script = dir.join("slow.sh");
        fs::write(&script, "#!/bin/bash\nsleep 1\n").expect("write script");

        let config = AppConfig {
            working_dir: dir,
            timeout: Duration::from_millis(50),
            env_allowlist: default_env_allowlist(),
        };

        let execution = execute_script(&config, "slow", &[]).await;
        assert_eq!(execution.exit_code, 124);
        assert!(execution.stderr.contains("timed out"));
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
