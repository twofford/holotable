//! Shared core for Holotable.
//!
//! This crate must stay free of I/O, async runtimes and the server crate, so that
//! it compiles both natively (for the server) and to WebAssembly (for the client).
//! Later milestones add a `dice` module here.

pub mod api;
pub mod campaign;
pub mod geometry;
pub mod protocol;
pub mod scene;
pub mod uvtt;
