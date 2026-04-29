//! Regression coverage for bind safety validation.
//!
//! These tests verify that public binds are rejected unless authentication
//! is enabled, without opening any network listeners.

use roko_core::config::schema::RokoConfig;
use roko_serve::validate_bind_safety;

fn config_with_bind(bind: &str, auth_enabled: bool) -> RokoConfig {
    let mut config = RokoConfig::default();
    config.server.bind = bind.to_string();
    config.server.port = 0;
    config.serve.auth.enabled = auth_enabled;
    config.serve.acknowledge_public_risk = false;
    config
}

fn bind_addr(config: &RokoConfig) -> String {
    format!("{}:{}", config.server.bind, config.server.port)
}

#[test]
fn loopback_bind_without_auth_succeeds() {
    let config = config_with_bind("127.0.0.1", false);
    let addr = bind_addr(&config);

    let result = validate_bind_safety(&addr, &config.serve);
    assert!(result.is_ok(), "loopback bind should always succeed");
}

#[test]
fn public_bind_without_auth_fails() {
    let config = config_with_bind("0.0.0.0", false);
    let addr = bind_addr(&config);

    let err_msg = validate_bind_safety(&addr, &config.serve)
        .expect_err("public bind without auth must fail")
        .to_string();
    assert!(
        err_msg.contains("auth") || err_msg.contains("unsafe") || err_msg.contains("public"),
        "error should mention auth or unsafe-public, got: {}",
        err_msg
    );
}

#[test]
fn public_bind_with_auth_succeeds() {
    let config = config_with_bind("0.0.0.0", true);
    let addr = bind_addr(&config);

    let result = validate_bind_safety(&addr, &config.serve);
    assert!(result.is_ok(), "public bind with auth should succeed");
}

#[test]
fn ipv6_wildcard_without_auth_fails() {
    let config = config_with_bind("::", false);
    let addr = bind_addr(&config);

    let result = validate_bind_safety(&addr, &config.serve);
    assert!(result.is_err(), "IPv6 wildcard bind without auth must fail");
}

#[test]
fn localhost_string_without_auth_succeeds() {
    let config = config_with_bind("localhost", false);
    let addr = bind_addr(&config);

    let result = validate_bind_safety(&addr, &config.serve);
    assert!(result.is_ok(), "localhost bind should always succeed");
}
