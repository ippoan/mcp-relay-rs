//! Shared DTOs between this crate and ref-files-worker.
//!
//! Rust here is the single source of truth: every type derives `ts_rs::TS`
//! and `bin/gen-ts.rs` writes the corresponding TypeScript declarations to
//! `ts-bindings/`, which CI (see `ref-files-worker/.github/workflows/sync-types.yml`)
//! copies into the worker repo and asserts is drift-free.
//!
//! All IDs are stringly-typed UUIDv4 — they get generated worker-side
//! (`crypto.randomUUID()`) so the Rust binary can pass them through opaquely.

pub mod file;
pub mod folder;
pub mod repo;
pub mod revision;

pub use file::*;
pub use folder::*;
pub use repo::*;
pub use revision::*;
