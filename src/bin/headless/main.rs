//! Entry point for the `headless` binary.
//!
//! The runner is a native dev tool — it spins up a real render device and does
//! GPU readback to save screenshots — so the whole thing is disabled on wasm32.
//! On that target this compiles to an empty program, which keeps
//! `cargo build --target wasm32-unknown-unknown` working.

#[cfg(not(target_arch = "wasm32"))]
mod runner;

#[cfg(not(target_arch = "wasm32"))]
fn main() { runner::run(); }

#[cfg(target_arch = "wasm32")]
fn main() {}
