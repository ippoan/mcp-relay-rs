//! `revisions` table — append-only history of a file's bytes.
//!
//! Each `file_put` allocates a new row + R2 object at
//! `files/{repo_id}/{file_id}/{rev_number}`. Rows are never updated; deletes
//! flip `files.deleted_at`, leaving the revision chain intact for
//! `file_history` walks.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export, export_to = "ts-bindings/")]
#[serde(rename_all = "snake_case")]
pub struct Revision {
    pub id: String,
    pub file_id: String,
    /// 1-indexed, monotonic per file.
    pub rev_number: u32,
    /// R2 key. Format: `files/{repo_id}/{file_id}/{rev_number}`.
    pub blob_key: String,
    pub size: u64,
    /// Lowercase hex SHA-256 of the raw (pre-base64) bytes.
    pub sha256: String,
    pub mime: Option<String>,
    /// GitHub login that produced this revision (resolved from MCP JWT).
    pub author_login: String,
    pub message: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export, export_to = "ts-bindings/")]
#[serde(rename_all = "snake_case")]
pub struct RevisionList {
    pub revisions: Vec<Revision>,
}
