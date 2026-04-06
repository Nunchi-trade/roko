//! Static registry of Roko's 16 built-in tools (§36.9, §36.b).
//!
//! [`StaticToolRegistry`] is the day-one [`ToolRegistry`] every Roko
//! deployment consults. It holds the 16 canonical built-ins
//! (`read_file`, `write_file`, `edit_file`, …) defined in
//! [`builtin`].
//!
//! The tool **definitions** live here (stub handlers ship in §36.b).
//! Each built-in module exposes a `pub fn tool_def() -> ToolDef`
//! constructor used to build the one-time
//! [`ROKO_BUILTIN_TOOLS`](builtin::ROKO_BUILTIN_TOOLS) slice.
//!
//! # Why a static registry?
//!
//! - **Zero-lock reads after init**: the slice is materialized once via
//!   [`std::sync::LazyLock`]; every subsequent read is lock-free.
//! - **Per-role filtering**: [`StaticToolRegistry::for_role`] delegates
//!   to the shared `role_allowlist` helper so hosted backends (Claude)
//!   and raw backends (Ollama) see the same tool set.
//! - **Compile-time guarantees**: the 16 built-ins are a fixed array;
//!   a runtime test asserts name uniqueness so we cannot accidentally
//!   ship a duplicate.
//!
//! # Implementation note — `LazyLock`
//!
//! [`roko_core::tool::ToolDef`] holds owned `String` fields (name,
//! description) and a `serde_json::Value` schema. None of those are
//! `const`-constructible, so the 16 definitions cannot live in a pure
//! `static` — they are materialized on first access via
//! [`std::sync::LazyLock`] (stable since Rust 1.80). Reads after first
//! access are zero-lock, which satisfies the original "static" intent
//! of §36.9 in spirit.

pub mod builtin;
pub mod expand_pointer;
pub mod handlers;
pub mod mock_dispatcher;
pub mod registry;

pub use builtin::{ROKO_BUILTIN_TOOLS, TOOL_COUNT};
pub use handlers::{handler_for, HandlerRegistry};
pub use mock_dispatcher::MockToolDispatcher;
pub use registry::StaticToolRegistry;
