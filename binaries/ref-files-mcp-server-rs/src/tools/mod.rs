//! `#[tool_router]` modules, one per resource. Each defines a small router
//! the entry-point in `mcp_server::RefFilesMcp::new` adds with the `+`
//! operator — same convention as `github-mcp-server-rs/src/mcp_server.rs`.

pub mod file;
pub mod folder;
pub mod repo;
