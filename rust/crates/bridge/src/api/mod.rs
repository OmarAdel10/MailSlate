//! Callable API surface exposed to Flutter.
//!
//! Modules map 1:1 to areas of the GUI; every public function here becomes
//! a Dart function in `lib/src/rust/` after codegen.

pub mod accounts;
pub mod system;
