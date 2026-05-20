//! `repo_init` tool — pairs with `ref-files-worker POST /v1/repos`.

use rmcp::{
    handler::server::wrapper::Parameters,
    model::{CallToolResult, Content},
    tool, tool_router,
};

use crate::mcp_server::RefFilesMcp;
use crate::types::RepoInitArgs;

#[tool_router(router = repo_router, vis = "pub(crate)")]
impl RefFilesMcp {
    /// Create-or-fetch a repo for the authenticated GitHub user. Idempotent.
    #[tool(
        description = "Create a reference-file repo scoped to the authenticated GitHub user, or return the existing one if a repo with the same name already exists."
    )]
    async fn repo_init(
        &self,
        Parameters(args): Parameters<RepoInitArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let repo = self.worker().repo_init(&args).await?;
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string(&repo).unwrap_or_default(),
        )]))
    }
}
