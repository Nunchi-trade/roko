#![allow(missing_docs)]
/// Build script for roko-cli: captures git hash and rustc version at compile time.
use std::process::Command;

fn main() {
    // Git short hash
    let git_hash = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                String::from_utf8(o.stdout).ok()
            } else {
                None
            }
        })
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=ROKO_GIT_HASH={git_hash}");

    // rustc version
    let rustc_version = Command::new("rustc")
        .args(["--version"])
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                String::from_utf8(o.stdout).ok()
            } else {
                None
            }
        })
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=ROKO_RUSTC_VERSION={rustc_version}");

    // Target triple
    if let Ok(target) = std::env::var("TARGET") {
        println!("cargo:rustc-env=ROKO_TARGET={target}");
    } else {
        println!("cargo:rustc-env=ROKO_TARGET=unknown");
    }

    // Rebuild when git HEAD changes (so hash stays fresh).
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../.git/refs/");
    println!("cargo:rerun-if-changed=build.rs");
}
