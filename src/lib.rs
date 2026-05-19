//! `mcp-relay` — auth-worker (`ippoan/auth-worker`) と組む MCP binary の
//! 共通ロジック (RFC 8628 device flow / 1-click pair / WS relay frame schema /
//! relay loop) を提供する crate。
//!
//! 利用 binary:
//! - `ippoan/github-mcp-server-rs`
//! - `ippoan/ref-files-mcp-server-rs`
//!
//! ## 設計判断
//!
//! - **Binary 名のパラメタライズ**: `Config::project_name` で `ProjectDirs` の
//!   3rd 引数 (token cache の per-binary 保存先) を切り替える。
//! - **Hello frame の `service` field** (Phase 2 / option C multiplex): auth-worker
//!   1 DO 内で複数 binary を区別するための discriminator。v1 sender (`service` 省略)
//!   は serde default で `"github-mcp-server-rs"` に fallback して後方互換を保つ。
//! - **`tower::Service` で generic**: rmcp `StreamableHttpService` を直接受けるので
//!   binary 側 tools 実装は本 crate から見えない。
//!
//! Out of scope: tools/types/binary 固有 client (`worker_client.rs`/`admin_exec.rs` 等)。
//!
//! Release channels (see `.github/workflows/{dev-release,tag-release}.yml`):
//! - `dev-N` (counter, auto from main push) — consumer の `Cargo.toml`
//!   `tag = "dev-N"` で pin する用途
//! - `v0.0.X` (semver, manual via workflow_dispatch) — stable

pub mod auth;
pub mod config;
pub mod pair;
pub mod relay;
pub mod token_cache;
