//! `RefFilesMcp` — rmcp server that exposes the 9 ref-files tools, each
//! delegating to `WorkerClient` (HTTP into `ref-files-worker`).
//!
//! No state of its own beyond an `Arc<WorkerClient>`: the worker holds the
//! D1 + R2, and the MCP binary is a pure adapter between MCP-shaped argument
//! structs and the worker's HTTP surface.
//!
//! Router composition mirrors `github-mcp-server-rs/src/mcp_server.rs` — each
//! `tools/<resource>.rs` defines a `#[tool_router]` impl block, and `new()`
//! sums them with the `+` operator.

use rmcp::{
    handler::server::router::tool::ToolRouter,
    model::{ServerCapabilities, ServerInfo},
    tool_handler, ServerHandler,
};
use std::sync::Arc;

use crate::worker_client::WorkerClient;

#[derive(Clone)]
pub struct RefFilesMcp {
    pub(crate) worker: Arc<WorkerClient>,
    #[allow(dead_code)] // referenced by `#[tool_handler]` expansion
    tool_router: ToolRouter<Self>,
}

impl RefFilesMcp {
    pub fn new(worker: Arc<WorkerClient>) -> Self {
        let tool_router = Self::repo_router() + Self::folder_router() + Self::file_router();
        Self {
            worker,
            tool_router,
        }
    }

    pub(crate) fn worker(&self) -> &WorkerClient {
        &self.worker
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for RefFilesMcp {
    fn get_info(&self) -> ServerInfo {
        let mut info = ServerInfo::default();
        info.instructions = Some(
            "Reference file storage MCP server. Persistence lives in \
             ref-files-worker (Cloudflare Worker, D1 + R2). Tools: \
             repo_init, folder_create, folder_list, file_put, file_get, \
             file_history, file_move, file_delete, file_search. All paths \
             are POSIX-style with no leading slash; `\"\"` denotes repo \
             root. Binary content is base64-encoded in file_put / file_get."
                .into(),
        );
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worker_client::WorkerConfig;

    fn build() -> RefFilesMcp {
        let client = WorkerClient::new(WorkerConfig {
            base_url: "http://127.0.0.1:1".into(),
            jwt: "test".into(),
        });
        RefFilesMcp::new(Arc::new(client))
    }

    #[test]
    fn router_exposes_all_nine_tools() {
        let server = build();
        let mut names: Vec<String> = server
            .tool_router
            .list_all()
            .into_iter()
            .map(|t| t.name.to_string())
            .collect();
        names.sort();
        assert_eq!(
            names,
            vec![
                "file_delete",
                "file_get",
                "file_history",
                "file_move",
                "file_put",
                "file_search",
                "folder_create",
                "folder_list",
                "repo_init",
            ]
        );
    }

    #[test]
    fn get_info_lists_tool_capabilities() {
        let server = build();
        let info = ServerHandler::get_info(&server);
        assert!(info.capabilities.tools.is_some());
    }
}
