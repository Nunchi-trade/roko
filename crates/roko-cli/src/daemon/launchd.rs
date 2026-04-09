use std::env;
use std::path::PathBuf;

/// Launchd label for the Roko daemon.
pub const LABEL: &str = "dev.nunchi.roko";

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn home_dir() -> PathBuf {
    dirs::home_dir().unwrap()
}

fn exe_path() -> PathBuf {
    env::current_exe().unwrap()
}

fn cargo_bin_path() -> PathBuf {
    home_dir().join(".cargo").join("bin")
}

fn logs_dir() -> PathBuf {
    home_dir().join(".roko").join("logs")
}

fn plist_value(value: impl AsRef<str>) -> String {
    xml_escape(value.as_ref())
}

/// Return the macOS LaunchAgents plist path for the daemon.
#[must_use]
pub fn plist_path() -> PathBuf {
    home_dir()
        .join("Library")
        .join("LaunchAgents")
        .join(format!("{LABEL}.plist"))
}

/// Generate the launchd plist for the daemon at the given port.
#[must_use]
pub fn generate_plist(port: u16) -> String {
    let exe = plist_value(exe_path().display().to_string());
    let home = plist_value(home_dir().display().to_string());
    let stdout_path = plist_value(logs_dir().join("daemon.log").display().to_string());
    let stderr_path = plist_value(logs_dir().join("daemon.err").display().to_string());
    let cargo_bin = cargo_bin_path();
    let path_value = match env::var("PATH") {
        Ok(current_path) if !current_path.is_empty() => {
            format!("{}:{}", cargo_bin.display(), current_path)
        }
        _ => cargo_bin.display().to_string(),
    };

    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{label}</string>
  <key>ProgramArguments</key>
  <array>
    <string>{exe}</string>
    <string>daemon</string>
    <string>start</string>
    <string>--foreground</string>
    <string>--port</string>
    <string>{port}</string>
  </array>
  <key>KeepAlive</key>
  <true/>
  <key>RunAtLoad</key>
  <true/>
  <key>WorkingDirectory</key>
  <string>{home}</string>
  <key>StandardOutPath</key>
  <string>{stdout_path}</string>
  <key>StandardErrorPath</key>
  <string>{stderr_path}</string>
  <key>EnvironmentVariables</key>
  <dict>
    <key>PATH</key>
    <string>{path}</string>
  </dict>
</dict>
</plist>
"#,
        label = LABEL,
        exe = exe,
        port = port,
        home = home,
        stdout_path = stdout_path,
        stderr_path = stderr_path,
        path = plist_value(path_value),
    )
}
