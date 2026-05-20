//! Integration tests for `RefFilesMcp` — kept outside `src/` so the
//! attribute macros in `src/mcp_server.rs` can't affect what the test
//! harness sees.

use std::sync::Arc;

use ref_files_mcp_server_rs::mcp_server::RefFilesMcp;
use ref_files_mcp_server_rs::worker_client::{WorkerClient, WorkerConfig};
use rmcp::ServerHandler;

fn build() -> RefFilesMcp {
    let client = WorkerClient::new(WorkerConfig {
        base_url: "http://127.0.0.1:1".into(),
        jwt: "test".into(),
    });
    RefFilesMcp::new(Arc::new(client))
}

#[test]
fn server_handler_get_info_smoke() {
    let server = build();
    let info = ServerHandler::get_info(&server);
    let prose = info.instructions.unwrap_or_default();
    assert!(prose.contains("ref-files-worker"));
    assert!(prose.contains("repo_init"));
}
