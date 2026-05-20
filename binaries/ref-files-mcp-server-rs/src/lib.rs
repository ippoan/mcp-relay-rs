//! ref-files-mcp-server-rs — library root.
//!
//! Split into a thin binary (`src/main.rs`) and a type-generator binary
//! (`src/bin/gen-ts.rs`) that share this library. Phase 1 adds the
//! `worker_client`, `mcp_server`, and `tools::*` modules — `gen-ts` only
//! needs `types` and stays cheap.

pub mod mcp_server;
pub mod tools;
pub mod types;
pub mod worker_client;
