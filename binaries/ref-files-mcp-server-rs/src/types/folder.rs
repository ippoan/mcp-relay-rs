//! `folders` table — hierarchical directory tree inside a repo.
//!
//! Each row stores both `parent_id` (for tree walks) and `path` (denormalized
//! POSIX-style "/a/b/c", root = ""). `path` is the lookup key used by
//! `folder_list` / `file_put`; the worker keeps it in sync on rename + move.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export, export_to = "ts-bindings/")]
#[serde(rename_all = "snake_case")]
pub struct Folder {
    pub id: String,
    pub repo_id: String,
    /// `None` only for the implicit root folder (`path == ""`).
    pub parent_id: Option<String>,
    /// Single-segment name. Cannot contain `/` or `\0`.
    pub name: String,
    /// Full POSIX-style path from repo root, no leading slash. Root = `""`.
    pub path: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export, export_to = "ts-bindings/")]
#[serde(rename_all = "snake_case")]
pub struct FolderCreateArgs {
    pub repo_id: String,
    /// POSIX-style path. Missing intermediate folders are created (mkdir -p).
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export, export_to = "ts-bindings/")]
#[serde(rename_all = "snake_case")]
pub struct FolderListArgs {
    pub repo_id: String,
    /// Empty string = repo root. Otherwise the canonical `path` of the parent.
    #[serde(default)]
    pub path: String,
    /// Recurse into subfolders. Default false.
    #[serde(default)]
    pub recursive: bool,
}

/// Response shape for `folder_list` — folders + files at (or under) `path`.
#[derive(Debug, Clone, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export, export_to = "ts-bindings/")]
#[serde(rename_all = "snake_case")]
pub struct FolderListing {
    pub folders: Vec<Folder>,
    pub files: Vec<super::file::File>,
}
