//! `roko inject` subcommand — sends signals to a running session.
//!
//! Signal injection allows external tools (CI, IDEs, monitoring) to push
//! directives into an active roko session. Injected signals are appended
//! to the substrate and can influence the agent's next compose cycle.

use std::path::PathBuf;

/// The kind of signal to inject.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InjectKind {
    /// Free-form text directive to the agent.
    Directive,
    /// An abort signal that tells the session to stop.
    Abort,
    /// A context signal (additional information for the agent).
    Context,
}

impl InjectKind {
    /// Parse a string into an inject kind.
    pub fn parse(s: &str) -> Result<Self, String> {
        match s.to_ascii_lowercase().as_str() {
            "directive" | "d" => Ok(Self::Directive),
            "abort" | "a" => Ok(Self::Abort),
            "context" | "ctx" | "c" => Ok(Self::Context),
            other => Err(format!("unknown inject kind: {other} (expected: directive, abort, context)")),
        }
    }

    /// Return the string representation of this kind.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Directive => "directive",
            Self::Abort => "abort",
            Self::Context => "context",
        }
    }
}

impl std::fmt::Display for InjectKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A signal injection request.
#[derive(Debug, Clone)]
pub struct InjectRequest {
    /// Target session ID.
    pub session_id: String,
    /// Kind of signal to inject.
    pub kind: InjectKind,
    /// Payload text.
    pub payload: String,
    /// Working directory (to locate the daemon socket).
    pub workdir: PathBuf,
}

impl InjectRequest {
    /// Create a new injection request.
    #[must_use]
    pub const fn new(session_id: String, kind: InjectKind, payload: String, workdir: PathBuf) -> Self {
        Self {
            session_id,
            kind,
            payload,
            workdir,
        }
    }

    /// Compute the expected daemon socket path for this request.
    #[must_use]
    pub fn socket_path(&self) -> PathBuf {
        self.workdir
            .join(".roko")
            .join("run")
            .join(format!("roko-{}.sock", self.session_id))
    }

    /// Validate the request before sending.
    pub fn validate(&self) -> Result<(), String> {
        if self.session_id.is_empty() {
            return Err("session_id must not be empty".into());
        }
        if self.kind == InjectKind::Directive && self.payload.is_empty() {
            return Err("directive injection requires a non-empty payload".into());
        }
        if self.kind == InjectKind::Context && self.payload.is_empty() {
            return Err("context injection requires a non-empty payload".into());
        }
        Ok(())
    }

    /// Format this request for display.
    #[must_use]
    pub fn summary(&self) -> String {
        let payload_preview = if self.payload.len() > 80 {
            format!("{}...", &self.payload[..77])
        } else {
            self.payload.clone()
        };
        format!(
            "inject {} -> session {} ({}B): {}",
            self.kind,
            self.session_id,
            self.payload.len(),
            payload_preview,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_inject_kind_variants() {
        assert_eq!(InjectKind::parse("directive").unwrap(), InjectKind::Directive);
        assert_eq!(InjectKind::parse("d").unwrap(), InjectKind::Directive);
        assert_eq!(InjectKind::parse("abort").unwrap(), InjectKind::Abort);
        assert_eq!(InjectKind::parse("a").unwrap(), InjectKind::Abort);
        assert_eq!(InjectKind::parse("context").unwrap(), InjectKind::Context);
        assert_eq!(InjectKind::parse("ctx").unwrap(), InjectKind::Context);
        assert_eq!(InjectKind::parse("c").unwrap(), InjectKind::Context);
    }

    #[test]
    fn parse_inject_kind_case_insensitive() {
        assert_eq!(InjectKind::parse("DIRECTIVE").unwrap(), InjectKind::Directive);
        assert_eq!(InjectKind::parse("Abort").unwrap(), InjectKind::Abort);
    }

    #[test]
    fn parse_inject_kind_unknown() {
        assert!(InjectKind::parse("bogus").is_err());
    }

    #[test]
    fn inject_kind_display() {
        assert_eq!(InjectKind::Directive.to_string(), "directive");
        assert_eq!(InjectKind::Abort.to_string(), "abort");
        assert_eq!(InjectKind::Context.to_string(), "context");
    }

    #[test]
    fn socket_path_computed_correctly() {
        let req = InjectRequest::new(
            "sess-1".into(),
            InjectKind::Directive,
            "do something".into(),
            PathBuf::from("/project"),
        );
        assert_eq!(
            req.socket_path(),
            PathBuf::from("/project/.roko/run/roko-sess-1.sock")
        );
    }

    #[test]
    fn validate_rejects_empty_session() {
        let req = InjectRequest::new(
            String::new(),
            InjectKind::Abort,
            "stop".into(),
            PathBuf::from("/tmp"),
        );
        assert!(req.validate().is_err());
    }

    #[test]
    fn validate_rejects_empty_directive_payload() {
        let req = InjectRequest::new(
            "s1".into(),
            InjectKind::Directive,
            String::new(),
            PathBuf::from("/tmp"),
        );
        assert!(req.validate().is_err());
    }

    #[test]
    fn validate_allows_empty_abort_payload() {
        let req = InjectRequest::new(
            "s1".into(),
            InjectKind::Abort,
            String::new(),
            PathBuf::from("/tmp"),
        );
        assert!(req.validate().is_ok());
    }

    #[test]
    fn summary_truncates_long_payload() {
        let long = "x".repeat(200);
        let req = InjectRequest::new(
            "s1".into(),
            InjectKind::Directive,
            long,
            PathBuf::from("/tmp"),
        );
        let summary = req.summary();
        assert!(summary.contains("..."));
        assert!(summary.len() < 200);
    }

    #[test]
    fn summary_short_payload() {
        let req = InjectRequest::new(
            "s1".into(),
            InjectKind::Context,
            "hello".into(),
            PathBuf::from("/tmp"),
        );
        let summary = req.summary();
        assert!(summary.contains("context"));
        assert!(summary.contains("hello"));
        assert!(summary.contains("5B"));
    }
}
