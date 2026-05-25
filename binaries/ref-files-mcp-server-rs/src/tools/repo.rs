//! `repo_init` + `repos_list` tools — pair with `ref-files-worker`'s
//! `/v1/repos` routes.

use rmcp::{
    handler::server::wrapper::Parameters,
    model::{CallToolResult, Content},
    tool, tool_router,
};

use crate::mcp_server::RefFilesMcp;
use crate::types::{RepoInitArgs, ReposListArgs};

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

    /// List every repo the authenticated GitHub user owns. Use this to
    /// recover a repo's `id` (UUID) from its `name` — all other tools
    /// (`folder_*`, `file_*`) take the UUID, not the name.
    #[tool(
        description = "List every reference-file repo owned by the authenticated GitHub user. Returns id + name for each. Use this to look up a repo UUID before calling folder_* / file_* tools."
    )]
    async fn repos_list(
        &self,
        Parameters(_args): Parameters<ReposListArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let list = self.worker().repos_list().await?;
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string(&list).unwrap_or_default(),
        )]))
    }
}
