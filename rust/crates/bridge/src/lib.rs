//! MailSlate FFI bridge (flutter_rust_bridge v2).
//!
//! The GUI talks to the Rust core only through this crate; see
//! [`api::system`] for the callable surface. Codegen lockstep and
//! generated-file rules live in AGENTS.md §5.

mod api;
mod frb_generated; /* AUTO INJECTED BY flutter_rust_bridge. This line may not be accurate, and you can change it according to your needs. */
