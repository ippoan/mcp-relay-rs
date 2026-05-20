//! File CRUD tools (6) — every call delegates to `ref-files-worker /v1/files*`.

use rmcp::{
    handler::server::wrapper::Parameters,
    model::{CallToolResult, Content},
    tool, tool_router,
};

use crate::mcp_server::RefFilesMcp;
use crate::types::{
    FileDeleteArgs, FileGetArgs, FileHistoryArgs, FileMoveArgs, FilePutArgs, FileSearchArgs,
};

#[tool_router(router = file_router, vis = "pub(crate)")]
impl RefFilesMcp {
    /// Append a new revision (or create the file) under `path`.
    #[tool(
        description = "Write/append a file revision in a ref-files repo. content_base64 is the raw bytes; intermediate folders are created automatically. Returns {file, revision}."
    )]
    async fn file_put(
        &self,
        Parameters(args): Parameters<FilePutArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let resp = self.worker().file_put(&args).await?;
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string(&resp).unwrap_or_default(),
        )]))
    }

    /// Fetch a file revision (default: latest).
    #[tool(
        description = "Read a file revision from a ref-files repo. Omit `revision` for the latest live revision. Returns {file, revision, content_base64}."
    )]
    async fn file_get(
        &self,
        Parameters(args): Parameters<FileGetArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let resp = self.worker().file_get(&args).await?;
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string(&resp).unwrap_or_default(),
        )]))
    }

    /// Walk revisions newest-first (default limit 20, max 100).
    #[tool(description = "List revisions of a file newest-first. Returns {revisions:[Revision]}.")]
    async fn file_history(
        &self,
        Parameters(args): Parameters<FileHistoryArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let resp = self.worker().file_history(&args).await?;
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string(&resp).unwrap_or_default(),
        )]))
    }

    /// Move/rename a file. Destination folders are auto-created. Refuses to overwrite.
    #[tool(
        description = "Rename / move a file inside a ref-files repo. Fails with conflict if to_path is occupied."
    )]
    async fn file_move(
        &self,
        Parameters(args): Parameters<FileMoveArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let resp = self.worker().file_move(&args).await?;
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string(&resp).unwrap_or_default(),
        )]))
    }

    /// Soft-delete a file. `file_history` still resolves; default `file_get` returns 404.
    #[tool(
        description = "Soft-delete a file (sets deleted_at). Revisions are kept so file_history still works; default file_get returns not_found for the path."
    )]
    async fn file_delete(
        &self,
        Parameters(args): Parameters<FileDeleteArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let resp = self.worker().file_delete(&args).await?;
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string(&resp).unwrap_or_default(),
        )]))
    }

    /// SQL `LIKE`-style substring match on name + path. Returns up to `limit` files.
    #[tool(
        description = "Substring-search files by name / path within a repo (Phase 1: SQL LIKE)."
    )]
    async fn file_search(
        &self,
        Parameters(args): Parameters<FileSearchArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let resp = self.worker().file_search(&args).await?;
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string(&resp).unwrap_or_default(),
        )]))
    }
}
