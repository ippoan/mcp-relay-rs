//! `repos` table — top-level container that scopes folders + files.
//!
//! One repo per `(owner_login, name)` tuple. `owner_login` is the GitHub login
//! that auth-worker resolved from the MCP JWT; the worker never trusts a
//! `owner_login` value sent from the client.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export, export_to = "ts-bindings/")]
#[serde(rename_all = "snake_case")]
pub struct Repo {
    pub id: String,
    pub owner_login: String,
    pub name: String,
    /// RFC 3339. Set by the worker on insert.
    pub created_at: String,
    /// RFC 3339. Updated by the worker on rename / metadata edit.
    pub updated_at: String,
}

/// Args for `repo_init` (MCP tool, Phase 1).
///
/// Idempotent: if a repo with the same `(owner_login, name)` already exists,
/// the worker returns the existing row instead of erroring.
#[derive(Debug, Clone, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export, export_to = "ts-bindings/")]
#[serde(rename_all = "snake_case")]
pub struct RepoInitArgs {
    /// Repository slug. `[a-z0-9][a-z0-9._-]{0,62}`, validated server-side.
    pub name: String,
}
