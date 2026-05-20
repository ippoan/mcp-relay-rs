//! `folder_create` / `folder_list` — pair with `ref-files-worker /v1/folders`.

use rmcp::{
    handler::server::wrapper::Parameters,
    model::{CallToolResult, Content},
    tool, tool_router,
};

use crate::mcp_server::RefFilesMcp;
use crate::types::{FolderCreateArgs, FolderListArgs};

#[tool_router(router = folder_router, vis = "pub(crate)")]
impl RefFilesMcp {
    /// mkdir -p inside a repo. Creates every missing ancestor folder.
    #[tool(description = "Create a folder (and any missing ancestors) inside a ref-files repo.")]
    async fn folder_create(
        &self,
        Parameters(args): Parameters<FolderCreateArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let folder = self.worker().folder_create(&args).await?;
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string(&folder).unwrap_or_default(),
        )]))
    }

    /// List folders + (live) files under `path`. Set `recursive=true` to walk subtrees.
    #[tool(
        description = "List folders and live files at (or recursively under) a repo path. Returns {folders:[Folder], files:[File]}."
    )]
    async fn folder_list(
        &self,
        Parameters(args): Parameters<FolderListArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let listing = self.worker().folder_list(&args).await?;
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string(&listing).unwrap_or_default(),
        )]))
    }
}
