//! Resource-limit helpers for spawned agent processes.
//!
//! Unix targets install `setrlimit` calls in the child just before `exec`.
//! Other platforms compile to a no-op so callers can keep one code path.

use tokio::process::Command;

/// Optional resource caps for agent subprocesses.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResourceLimits {
    /// Maximum CPU seconds before the OS delivers the platform limit signal.
    pub max_cpu_secs: Option<u64>,
    /// Maximum resident/address-space bytes, depending on platform support.
    pub max_rss_bytes: Option<u64>,
}

/// Apply resource limits to a command before spawning it.
#[cfg(unix)]
#[allow(unsafe_code, clippy::cast_possible_truncation)]
pub fn apply_resource_limits(cmd: &mut Command, limits: &ResourceLimits) {
    if limits.max_cpu_secs.is_none() && limits.max_rss_bytes.is_none() {
        return;
    }

    let limits = limits.clone();
    // SAFETY: `setrlimit` is called in the child process after fork and before
    // exec. The closure only captures plain integers and does not touch shared
    // process state.
    unsafe {
        cmd.pre_exec(move || {
            if let Some(cpu_secs) = limits.max_cpu_secs {
                let limit = libc::rlimit {
                    rlim_cur: cpu_secs as libc::rlim_t,
                    rlim_max: cpu_secs as libc::rlim_t,
                };
                if libc::setrlimit(libc::RLIMIT_CPU, &limit) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }

            if let Some(max_bytes) = limits.max_rss_bytes {
                let limit = libc::rlimit {
                    rlim_cur: max_bytes as libc::rlim_t,
                    rlim_max: max_bytes as libc::rlim_t,
                };
                #[cfg(target_os = "linux")]
                let resource = libc::RLIMIT_AS;
                #[cfg(not(target_os = "linux"))]
                let resource = libc::RLIMIT_RSS;

                if libc::setrlimit(resource, &limit) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }

            Ok(())
        });
    }
}

/// Resource limits are not currently implemented on non-Unix platforms.
#[cfg(not(unix))]
pub fn apply_resource_limits(_cmd: &mut Command, _limits: &ResourceLimits) {}
