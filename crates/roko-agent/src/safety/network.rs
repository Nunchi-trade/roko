//! Network destination allowlist policy (§36.48).
//!
//! Gates outbound URLs for network-capable tools (`web_fetch`,
//! `web_search`, and any future network tool). Every URL the dispatcher
//! is about to hand to a network tool runs through
//! [`check_url_with_policy`] first. Violations surface as
//! [`ToolError::NetworkBlocked`] and short-circuit dispatch.
//!
//! # Policy dimensions
//!
//! - **Scheme** — only URL schemes in [`NetworkPolicy::allow_schemes`]
//!   pass. Default: HTTPS-only.
//! - **Private networks** — when
//!   [`NetworkPolicy::block_private_networks`] is true (the default),
//!   loopback / RFC1918-private / link-local / unspecified IP literal
//!   hosts are rejected. This defeats SSRF probes at `127.0.0.1`,
//!   `169.254.169.254` (cloud metadata), and the `10/8`/`172.16/12`/
//!   `192.168/16` private ranges.
//! - **Deny list** — hostnames in [`NetworkPolicy::deny_hosts`] are
//!   rejected via exact-or-suffix match (e.g. `".internal"` rejects
//!   every host ending in `.internal`).
//! - **Allow list** — when [`NetworkPolicy::allow_hosts`] is non-empty,
//!   only hosts matching an entry (exact or suffix) are permitted.
//!
//! # Example
//!
//! ```ignore
//! use roko_agent::safety::network::{check_url, check_url_with_policy, NetworkPolicy};
//!
//! // Default policy: HTTPS-only, no private networks.
//! assert!(check_url("https://api.github.com/zen").is_ok());
//! assert!(check_url("http://example.com").is_err());
//! assert!(check_url("https://127.0.0.1/x").is_err());
//!
//! // Custom policy: allow only *.github.com.
//! let policy = NetworkPolicy {
//!     allow_hosts: vec![".github.com".to_string()],
//!     ..NetworkPolicy::default()
//! };
//! assert!(check_url_with_policy("https://api.github.com/zen", &policy).is_ok());
//! assert!(check_url_with_policy("https://evil.com", &policy).is_err());
//! ```

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use roko_core::tool::ToolError;
use url::Url;

/// A set of rules describing which outbound URLs a network tool may reach.
///
/// Construct via [`NetworkPolicy::default`] for the common "HTTPS-only,
/// block private networks, no allowlist" configuration, then override
/// fields as needed.
#[derive(Debug, Clone)]
pub struct NetworkPolicy {
    /// Allowed URL schemes. Default: `["https"]` (HTTPS-only).
    ///
    /// An empty vec is treated as "any scheme permitted" — prefer
    /// listing the schemes you want over leaving this empty.
    pub allow_schemes: Vec<String>,

    /// If non-empty, only these hostnames (matched exactly or as a
    /// dotted suffix, e.g. `.github.com`) are allowed. Empty means
    /// "any host not on the deny list".
    pub allow_hosts: Vec<String>,

    /// Blocked hostnames, matched exactly or as a dotted suffix
    /// (e.g. `.internal` blocks `server.internal`).
    pub deny_hosts: Vec<String>,

    /// If `true` (the default), loopback / private (RFC1918) /
    /// link-local / unspecified IP literal hosts are rejected.
    pub block_private_networks: bool,
}

impl Default for NetworkPolicy {
    fn default() -> Self {
        Self {
            allow_schemes: vec!["https".to_string()],
            allow_hosts: Vec::new(),
            deny_hosts: Vec::new(),
            block_private_networks: true,
        }
    }
}

/// Check whether `url` is allowed under the given policy.
///
/// # Errors
///
/// Returns [`ToolError::NetworkBlocked`] if the URL fails any of:
///
/// 1. URL parsing.
/// 2. Scheme check against [`NetworkPolicy::allow_schemes`].
/// 3. Missing host.
/// 4. Private-network IP-literal check (when
///    [`NetworkPolicy::block_private_networks`] is set).
/// 5. Deny-host match.
/// 6. Allow-host match (when [`NetworkPolicy::allow_hosts`] is non-empty).
pub fn check_url_with_policy(url: &str, policy: &NetworkPolicy) -> Result<(), ToolError> {
    // 1. Parse.
    let parsed =
        Url::parse(url).map_err(|_| ToolError::NetworkBlocked(format!("invalid url: {url}")))?;

    // 2. Scheme check.
    let scheme = parsed.scheme();
    if !policy.allow_schemes.is_empty() && !policy.allow_schemes.iter().any(|s| s == scheme) {
        return Err(ToolError::NetworkBlocked(format!(
            "scheme {scheme} not allowed"
        )));
    }

    // 3. Extract host.
    let host = parsed
        .host_str()
        .ok_or_else(|| ToolError::NetworkBlocked("url has no host".to_string()))?;

    // 4. Private-network check (only applies to IP-literal hosts).
    if policy.block_private_networks {
        // `url::Url::host_str` returns IPv6 literals in bracketed form
        // (e.g. `"[::1]"`), so strip the brackets before parsing.
        let ip_candidate = host
            .strip_prefix('[')
            .and_then(|s| s.strip_suffix(']'))
            .unwrap_or(host);
        if let Ok(ip) = ip_candidate.parse::<IpAddr>() {
            if is_private_ip(&ip) {
                return Err(ToolError::NetworkBlocked(
                    "private network address blocked".to_string(),
                ));
            }
        }
    }

    // 5. Deny-host check (exact or suffix).
    for entry in &policy.deny_hosts {
        if host_matches(host, entry) {
            return Err(ToolError::NetworkBlocked(format!(
                "host {host} matches deny rule {entry}"
            )));
        }
    }

    // 6. Allow-host check (only when allowlist is non-empty).
    if !policy.allow_hosts.is_empty() {
        let allowed = policy
            .allow_hosts
            .iter()
            .any(|entry| host_matches(host, entry));
        if !allowed {
            return Err(ToolError::NetworkBlocked(format!(
                "host {host} not in allowlist"
            )));
        }
    }

    Ok(())
}

/// Convenience wrapper using [`NetworkPolicy::default`] (HTTPS-only,
/// block private networks, no allow/deny lists).
///
/// # Errors
///
/// Returns [`ToolError::NetworkBlocked`] for any URL the default policy
/// rejects — notably non-HTTPS schemes and private/loopback IP literals.
pub fn check_url(url: &str) -> Result<(), ToolError> {
    check_url_with_policy(url, &NetworkPolicy::default())
}

// ─── helpers ───────────────────────────────────────────────────────────────

/// True if `host` equals `entry` exactly, or — when `entry` starts with
/// `.` — if `host` ends with `entry`.
///
/// Examples:
/// - `host_matches("api.github.com", "api.github.com")` → `true`
/// - `host_matches("api.github.com", ".github.com")` → `true`
/// - `host_matches("evil.com", ".github.com")` → `false`
/// - `host_matches("server.internal", ".internal")` → `true`
fn host_matches(host: &str, entry: &str) -> bool {
    if entry.is_empty() {
        return false;
    }
    if entry.starts_with('.') {
        // Suffix match: host must end with entry.
        // ".internal" matches "server.internal" but not "server.internal.evil.com".
        host.ends_with(entry)
    } else {
        // Exact match.
        host == entry
    }
}

/// Returns true if `ip` is a loopback, private (RFC1918), link-local,
/// or unspecified address.
const fn is_private_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_private_ipv4(*v4),
        IpAddr::V6(v6) => is_private_ipv6(v6),
    }
}

const fn is_private_ipv4(ip: Ipv4Addr) -> bool {
    ip.is_loopback() || ip.is_private() || ip.is_link_local() || ip.is_unspecified()
}

const fn is_private_ipv6(ip: &Ipv6Addr) -> bool {
    if ip.is_loopback() || ip.is_unspecified() {
        return true;
    }
    // Link-local: fe80::/10. Portable bitmask check so we don't rely on
    // `Ipv6Addr::is_unicast_link_local` (stable since 1.86 but simple
    // enough to implement locally).
    let seg0 = ip.segments()[0];
    if seg0 & 0xffc0 == 0xfe80 {
        return true;
    }
    // Unique-local: fc00::/7 (fc00..fdff) — the IPv6 analogue of RFC1918.
    if seg0 & 0xfe00 == 0xfc00 {
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    // ─── default policy (HTTPS + block private) ─────────────────────────

    #[test]
    fn https_url_to_public_host_allowed() {
        assert!(check_url("https://api.github.com/zen").is_ok());
    }

    #[test]
    fn http_url_blocked_by_default_policy() {
        let err = check_url("http://example.com").expect_err("http should be blocked");
        match err {
            ToolError::NetworkBlocked(msg) => assert!(msg.contains("scheme")),
            other => panic!("expected NetworkBlocked, got {other:?}"),
        }
    }

    #[test]
    fn http_allowed_when_scheme_in_list() {
        let policy = NetworkPolicy {
            allow_schemes: vec!["https".to_string(), "http".to_string()],
            ..NetworkPolicy::default()
        };
        assert!(check_url_with_policy("http://example.com", &policy).is_ok());
    }

    #[test]
    fn ftp_url_blocked() {
        assert!(check_url("ftp://ftp.example.com").is_err());
    }

    #[test]
    fn invalid_url_blocked() {
        let err = check_url("not a url").expect_err("garbage should be blocked");
        match err {
            ToolError::NetworkBlocked(msg) => assert!(msg.contains("invalid url")),
            other => panic!("expected NetworkBlocked, got {other:?}"),
        }
    }

    #[test]
    fn url_without_host_blocked() {
        // `file:///etc/passwd` parses fine but is blocked on scheme
        // (not `https`) under the default policy. Either way it must
        // fail.
        assert!(check_url("file:///etc/passwd").is_err());
    }

    #[test]
    fn url_without_host_blocked_when_scheme_allowed() {
        // Allow the `file` scheme explicitly — the URL should still be
        // blocked because `file:///x` has no host.
        let policy = NetworkPolicy {
            allow_schemes: vec!["file".to_string()],
            block_private_networks: false,
            ..NetworkPolicy::default()
        };
        let err = check_url_with_policy("file:///etc/passwd", &policy)
            .expect_err("no-host URL should be blocked");
        match err {
            ToolError::NetworkBlocked(msg) => assert!(msg.contains("no host")),
            other => panic!("expected NetworkBlocked, got {other:?}"),
        }
    }

    // ─── private-network blocking ───────────────────────────────────────

    #[test]
    fn loopback_ipv4_blocked() {
        let err = check_url("https://127.0.0.1/x").expect_err("loopback should be blocked");
        match err {
            ToolError::NetworkBlocked(msg) => assert!(msg.contains("private network")),
            other => panic!("expected NetworkBlocked, got {other:?}"),
        }
    }

    #[test]
    fn loopback_ipv6_blocked() {
        assert!(check_url("https://[::1]/x").is_err());
    }

    #[test]
    fn private_ipv4_blocked() {
        assert!(check_url("https://192.168.1.1").is_err());
        assert!(check_url("https://10.0.0.1").is_err());
        assert!(check_url("https://172.16.0.1").is_err());
    }

    #[test]
    fn link_local_ipv4_blocked() {
        assert!(check_url("https://169.254.169.254").is_err());
    }

    #[test]
    fn link_local_ipv6_blocked() {
        assert!(check_url("https://[fe80::1]/x").is_err());
    }

    #[test]
    fn unique_local_ipv6_blocked() {
        // fc00::/7 is the IPv6 analogue of RFC1918.
        assert!(check_url("https://[fc00::1]/x").is_err());
    }

    #[test]
    fn unspecified_blocked() {
        assert!(check_url("https://0.0.0.0").is_err());
    }

    #[test]
    fn private_networks_allowed_when_flag_off() {
        let policy = NetworkPolicy {
            block_private_networks: false,
            ..NetworkPolicy::default()
        };
        assert!(check_url_with_policy("https://127.0.0.1/x", &policy).is_ok());
        assert!(check_url_with_policy("https://192.168.1.1", &policy).is_ok());
    }

    #[test]
    fn public_ipv4_literal_allowed() {
        // A genuine public IP (1.1.1.1) should pass the private-network filter.
        assert!(check_url("https://1.1.1.1").is_ok());
    }

    // ─── deny list ──────────────────────────────────────────────────────

    #[test]
    fn deny_host_exact_match_blocks() {
        let policy = NetworkPolicy {
            deny_hosts: vec!["bad.example.com".to_string()],
            ..NetworkPolicy::default()
        };
        let err = check_url_with_policy("https://bad.example.com", &policy)
            .expect_err("deny host should block");
        assert!(matches!(err, ToolError::NetworkBlocked(_)));
    }

    #[test]
    fn deny_host_suffix_match_blocks() {
        let policy = NetworkPolicy {
            deny_hosts: vec![".internal".to_string()],
            ..NetworkPolicy::default()
        };
        assert!(check_url_with_policy("https://server.internal", &policy).is_err());
        assert!(check_url_with_policy("https://db.prod.internal", &policy).is_err());
    }

    #[test]
    fn deny_host_does_not_block_unrelated_hosts() {
        let policy = NetworkPolicy {
            deny_hosts: vec!["bad.example.com".to_string()],
            ..NetworkPolicy::default()
        };
        assert!(check_url_with_policy("https://good.example.com", &policy).is_ok());
    }

    // ─── allow list ─────────────────────────────────────────────────────

    #[test]
    fn allow_hosts_enforces_allowlist() {
        let policy = NetworkPolicy {
            allow_hosts: vec!["api.github.com".to_string()],
            ..NetworkPolicy::default()
        };
        assert!(check_url_with_policy("https://api.github.com/zen", &policy).is_ok());
        let err = check_url_with_policy("https://evil.com", &policy)
            .expect_err("non-allowlisted host should be blocked");
        match err {
            ToolError::NetworkBlocked(msg) => assert!(msg.contains("not in allowlist")),
            other => panic!("expected NetworkBlocked, got {other:?}"),
        }
    }

    #[test]
    fn allow_hosts_suffix_match_works() {
        let policy = NetworkPolicy {
            allow_hosts: vec![".github.com".to_string()],
            ..NetworkPolicy::default()
        };
        assert!(check_url_with_policy("https://api.github.com", &policy).is_ok());
        assert!(check_url_with_policy("https://raw.github.com", &policy).is_ok());
        assert!(check_url_with_policy("https://api.gitlab.com", &policy).is_err());
    }

    #[test]
    fn empty_allow_hosts_means_no_restriction() {
        let policy = NetworkPolicy::default();
        assert!(policy.allow_hosts.is_empty());
        assert!(check_url_with_policy("https://example.com", &policy).is_ok());
        assert!(check_url_with_policy("https://anywhere.org", &policy).is_ok());
    }

    #[test]
    fn deny_list_beats_allow_list() {
        // A host that's on both lists must be rejected: deny is evaluated first.
        let policy = NetworkPolicy {
            allow_hosts: vec!["api.example.com".to_string()],
            deny_hosts: vec!["api.example.com".to_string()],
            ..NetworkPolicy::default()
        };
        assert!(check_url_with_policy("https://api.example.com", &policy).is_err());
    }

    // ─── convenience wrapper ────────────────────────────────────────────

    #[test]
    fn check_url_default_blocks_http() {
        assert!(check_url("http://example.com").is_err());
    }

    #[test]
    fn check_url_default_allows_public_https() {
        assert!(check_url("https://example.com/path?q=1").is_ok());
    }

    // ─── host_matches helper ─────────────────────────────────────────────

    #[test]
    fn host_matches_exact() {
        assert!(host_matches("api.github.com", "api.github.com"));
        assert!(!host_matches("api.github.com", "github.com"));
    }

    #[test]
    fn host_matches_suffix() {
        assert!(host_matches("api.github.com", ".github.com"));
        assert!(host_matches("server.internal", ".internal"));
        assert!(!host_matches("evil.com", ".github.com"));
    }

    #[test]
    fn host_matches_empty_entry_is_false() {
        assert!(!host_matches("any.host", ""));
    }
}
