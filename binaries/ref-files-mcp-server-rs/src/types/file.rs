//! `files` table — logical file pointing at its current revision.
//!
//! A file's bytes always live in R2 under
//! `files/{repo_id}/{file_id}/{rev_number}`; the `files` row tracks identity
//! (name, parent folder, current revision) and `revisions` holds the history.
//! Deletes are soft (`deleted_at`) so history-walks still succeed.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export, export_to = "ts-bindings/")]
#[serde(rename_all = "snake_case")]
pub struct File {
    pub id: String,
    pub repo_id: String,
    /// `None` → file sits at repo root.
    pub folder_id: Option<String>,
    pub name: String,
    /// Denormalized for search/index. `folder.path + "/" + name`, no leading slash.
    pub path: String,
    /// `revisions.id` of the latest non-deleted revision. May be the deleted
    /// row's id immediately after `file_delete` — clients should check
    /// `deleted_at` first.
    pub current_revision_id: String,
    /// `1` for the initial revision, monotonically incremented per `file_put`.
    pub current_revision_number: u32,
    pub size: u64,
    pub mime: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    /// RFC 3339 if soft-deleted; `None` for live files.
    pub deleted_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export, export_to = "ts-bindings/")]
#[serde(rename_all = "snake_case")]
pub struct FilePutArgs {
    pub repo_id: String,
    /// POSIX-style full path inside the repo (e.g. `notes/2026/may.md`).
    /// Intermediate folders auto-created.
    pub path: String,
    /// Base64-encoded payload. `gen-ts` emits this as `string` in TS;
    /// callers (MCP, worker) agree on base64 for binary safety.
    pub content_base64: String,
    pub mime: Option<String>,
    /// Optional commit message attached to the new revision.
    #[serde(default)]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export, export_to = "ts-bindings/")]
#[serde(rename_all = "snake_case")]
pub struct FileGetArgs {
    pub repo_id: String,
    pub path: String,
    /// `None` → latest. `Some(n)` → that specific revision number.
    #[serde(default)]
    pub revision: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export, export_to = "ts-bindings/")]
#[serde(rename_all = "snake_case")]
pub struct FileGetResponse {
    pub file: File,
    pub revision: super::revision::Revision,
    pub content_base64: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export, export_to = "ts-bindings/")]
#[serde(rename_all = "snake_case")]
pub struct FileHistoryArgs {
    pub repo_id: String,
    pub path: String,
    /// `1..=100`, default 20.
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export, export_to = "ts-bindings/")]
#[serde(rename_all = "snake_case")]
pub struct FileMoveArgs {
    pub repo_id: String,
    pub from_path: String,
    pub to_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export, export_to = "ts-bindings/")]
#[serde(rename_all = "snake_case")]
pub struct FileDeleteArgs {
    pub repo_id: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export, export_to = "ts-bindings/")]
#[serde(rename_all = "snake_case")]
pub struct FileSearchArgs {
    pub repo_id: String,
    /// Substring match against `files.path` and `files.name`. Phase 1 is a
    /// SQL `LIKE`; Phase 2 may add full-text on `revisions.content_text`.
    pub query: String,
    /// Restrict to this folder subtree (POSIX path). Default: whole repo.
    #[serde(default)]
    pub under_path: Option<String>,
    /// Include soft-deleted files. Default false.
    #[serde(default)]
    pub include_deleted: bool,
    /// `1..=100`, default 20.
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export, export_to = "ts-bindings/")]
#[serde(rename_all = "snake_case")]
pub struct FileSearchResult {
    pub files: Vec<File>,
}
