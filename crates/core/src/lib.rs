//! Shared core for Holotable.
//!
//! This crate must stay free of I/O, async runtimes and the server crate, so that
//! it compiles both natively (for the server) and to WebAssembly (for the client).
//! Later milestones add `geometry`, `uvtt` and `dice` modules here.

pub mod protocol;
