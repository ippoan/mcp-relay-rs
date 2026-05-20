//! `WorkerClient` integration tests against a `mockito` server. Each test
//! asserts both the wire shape (URL + method + Authorization header + body)
//! and that the returned DTO deserializes correctly. The MCP tool handlers
//! are pure pass-throughs, so this is also the de-facto test of the tools
//! layer — the only thing the tool wrappers add is wrapping the DTO in a
//! `CallToolResult`.

use ref_files_mcp_server_rs::types::{
    FileDeleteArgs, FileGetArgs, FileHistoryArgs, FileMoveArgs, FilePutArgs, FileSearchArgs,
    FolderCreateArgs, FolderListArgs, RepoInitArgs,
};
use ref_files_mcp_server_rs::worker_client::{WorkerClient, WorkerConfig};

fn client(url: String) -> WorkerClient {
    WorkerClient::new(WorkerConfig {
        base_url: url,
        jwt: "test-jwt".into(),
    })
}

#[tokio::test]
async fn repo_init_posts_to_v1_repos() {
    let mut server = mockito::Server::new_async().await;
    let m = server
        .mock("POST", "/v1/repos")
        .match_header("authorization", "Bearer test-jwt")
        .match_body(mockito::Matcher::JsonString(r#"{"name":"nb"}"#.into()))
        .with_status(201)
        .with_header("content-type", "application/json")
        .with_body(
            r#"{"id":"r1","owner_login":"alice","name":"nb","created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z"}"#,
        )
        .create_async()
        .await;

    let c = client(server.url());
    let repo = c
        .repo_init(&RepoInitArgs { name: "nb".into() })
        .await
        .expect("repo_init");
    assert_eq!(repo.id, "r1");
    assert_eq!(repo.name, "nb");
    m.assert_async().await;
}

#[tokio::test]
async fn folder_create_posts_to_v1_folders() {
    let mut server = mockito::Server::new_async().await;
    let m = server
        .mock("POST", "/v1/folders")
        .with_status(201)
        .with_header("content-type", "application/json")
        .with_body(
            r#"{"id":"f1","repo_id":"r1","parent_id":null,"name":"a","path":"a","created_at":"2026-01-01T00:00:00Z"}"#,
        )
        .create_async()
        .await;
    let c = client(server.url());
    let folder = c
        .folder_create(&FolderCreateArgs {
            repo_id: "r1".into(),
            path: "a".into(),
        })
        .await
        .expect("folder_create");
    assert_eq!(folder.path, "a");
    m.assert_async().await;
}

#[tokio::test]
async fn folder_list_passes_query_params() {
    let mut server = mockito::Server::new_async().await;
    let m = server
        .mock("GET", "/v1/folders")
        .match_query(mockito::Matcher::AllOf(vec![
            mockito::Matcher::UrlEncoded("repo_id".into(), "r1".into()),
            mockito::Matcher::UrlEncoded("path".into(), "docs".into()),
            mockito::Matcher::UrlEncoded("recursive".into(), "true".into()),
        ]))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"folders":[],"files":[]}"#)
        .create_async()
        .await;
    let c = client(server.url());
    let listing = c
        .folder_list(&FolderListArgs {
            repo_id: "r1".into(),
            path: "docs".into(),
            recursive: true,
        })
        .await
        .expect("folder_list");
    assert!(listing.folders.is_empty());
    assert!(listing.files.is_empty());
    m.assert_async().await;
}

#[tokio::test]
async fn folder_list_omits_path_when_empty() {
    let mut server = mockito::Server::new_async().await;
    let m = server
        .mock("GET", "/v1/folders")
        .match_query(mockito::Matcher::Regex(r"^repo_id=r1$".into()))
        .with_status(200)
        .with_body(r#"{"folders":[],"files":[]}"#)
        .create_async()
        .await;
    let c = client(server.url());
    let _ = c
        .folder_list(&FolderListArgs {
            repo_id: "r1".into(),
            path: "".into(),
            recursive: false,
        })
        .await
        .expect("folder_list");
    m.assert_async().await;
}

#[tokio::test]
async fn file_put_returns_revision() {
    let mut server = mockito::Server::new_async().await;
    let m = server
        .mock("POST", "/v1/files")
        .with_status(201)
        .with_header("content-type", "application/json")
        .with_body(
            r#"{"file":{"id":"fi","repo_id":"r1","folder_id":null,"name":"a.txt","path":"a.txt","current_revision_id":"rv","current_revision_number":1,"size":2,"mime":null,"created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z","deleted_at":null},"revision":{"id":"rv","file_id":"fi","rev_number":1,"blob_key":"files/r1/fi/1","size":2,"sha256":"deadbeef","mime":null,"author_login":"alice","message":null,"created_at":"2026-01-01T00:00:00Z"},"content_base64":""}"#,
        )
        .create_async()
        .await;
    let c = client(server.url());
    let resp = c
        .file_put(&FilePutArgs {
            repo_id: "r1".into(),
            path: "a.txt".into(),
            content_base64: "aGk=".into(),
            mime: None,
            message: None,
        })
        .await
        .expect("file_put");
    assert_eq!(resp.revision.rev_number, 1);
    assert_eq!(resp.file.path, "a.txt");
    m.assert_async().await;
}

#[tokio::test]
async fn file_get_includes_revision_when_specified() {
    let mut server = mockito::Server::new_async().await;
    let m = server
        .mock("GET", "/v1/files")
        .match_query(mockito::Matcher::AllOf(vec![
            mockito::Matcher::UrlEncoded("repo_id".into(), "r1".into()),
            mockito::Matcher::UrlEncoded("path".into(), "a.txt".into()),
            mockito::Matcher::UrlEncoded("revision".into(), "2".into()),
        ]))
        .with_status(200)
        .with_body(
            r#"{"file":{"id":"fi","repo_id":"r1","folder_id":null,"name":"a.txt","path":"a.txt","current_revision_id":"rv","current_revision_number":2,"size":2,"mime":null,"created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z","deleted_at":null},"revision":{"id":"rv","file_id":"fi","rev_number":2,"blob_key":"files/r1/fi/2","size":2,"sha256":"x","mime":null,"author_login":"alice","message":null,"created_at":"2026-01-01T00:00:00Z"},"content_base64":"aGk="}"#,
        )
        .create_async()
        .await;
    let c = client(server.url());
    let r = c
        .file_get(&FileGetArgs {
            repo_id: "r1".into(),
            path: "a.txt".into(),
            revision: Some(2),
        })
        .await
        .expect("file_get");
    assert_eq!(r.revision.rev_number, 2);
    assert_eq!(r.content_base64, "aGk=");
    m.assert_async().await;
}

#[tokio::test]
async fn file_history_passes_limit() {
    let mut server = mockito::Server::new_async().await;
    let m = server
        .mock("GET", "/v1/files/history")
        .match_query(mockito::Matcher::AllOf(vec![
            mockito::Matcher::UrlEncoded("repo_id".into(), "r1".into()),
            mockito::Matcher::UrlEncoded("path".into(), "a.txt".into()),
            mockito::Matcher::UrlEncoded("limit".into(), "5".into()),
        ]))
        .with_status(200)
        .with_body(r#"{"revisions":[]}"#)
        .create_async()
        .await;
    let c = client(server.url());
    let r = c
        .file_history(&FileHistoryArgs {
            repo_id: "r1".into(),
            path: "a.txt".into(),
            limit: Some(5),
        })
        .await
        .expect("file_history");
    assert!(r.revisions.is_empty());
    m.assert_async().await;
}

#[tokio::test]
async fn file_move_posts_to_move_endpoint() {
    let mut server = mockito::Server::new_async().await;
    let m = server
        .mock("POST", "/v1/files/move")
        .with_status(200)
        .with_body(
            r#"{"id":"fi","repo_id":"r1","folder_id":null,"name":"b.txt","path":"b.txt","current_revision_id":"rv","current_revision_number":1,"size":2,"mime":null,"created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-02T00:00:00Z","deleted_at":null}"#,
        )
        .create_async()
        .await;
    let c = client(server.url());
    let f = c
        .file_move(&FileMoveArgs {
            repo_id: "r1".into(),
            from_path: "a.txt".into(),
            to_path: "b.txt".into(),
        })
        .await
        .expect("file_move");
    assert_eq!(f.path, "b.txt");
    m.assert_async().await;
}

#[tokio::test]
async fn file_delete_sends_delete_with_query() {
    let mut server = mockito::Server::new_async().await;
    let m = server
        .mock("DELETE", "/v1/files")
        .match_query(mockito::Matcher::AllOf(vec![
            mockito::Matcher::UrlEncoded("repo_id".into(), "r1".into()),
            mockito::Matcher::UrlEncoded("path".into(), "a.txt".into()),
        ]))
        .with_status(200)
        .with_body(
            r#"{"id":"fi","repo_id":"r1","folder_id":null,"name":"a.txt","path":"a.txt","current_revision_id":"rv","current_revision_number":1,"size":2,"mime":null,"created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-02T00:00:00Z","deleted_at":"2026-01-02T00:00:00Z"}"#,
        )
        .create_async()
        .await;
    let c = client(server.url());
    let f = c
        .file_delete(&FileDeleteArgs {
            repo_id: "r1".into(),
            path: "a.txt".into(),
        })
        .await
        .expect("file_delete");
    assert!(f.deleted_at.is_some());
    m.assert_async().await;
}

#[tokio::test]
async fn file_search_passes_all_optional_params() {
    let mut server = mockito::Server::new_async().await;
    let m = server
        .mock("GET", "/v1/files/search")
        .match_query(mockito::Matcher::AllOf(vec![
            mockito::Matcher::UrlEncoded("repo_id".into(), "r1".into()),
            mockito::Matcher::UrlEncoded("query".into(), "readme".into()),
            mockito::Matcher::UrlEncoded("under_path".into(), "docs".into()),
            mockito::Matcher::UrlEncoded("include_deleted".into(), "true".into()),
            mockito::Matcher::UrlEncoded("limit".into(), "10".into()),
        ]))
        .with_status(200)
        .with_body(r#"{"files":[]}"#)
        .create_async()
        .await;
    let c = client(server.url());
    let r = c
        .file_search(&FileSearchArgs {
            repo_id: "r1".into(),
            query: "readme".into(),
            under_path: Some("docs".into()),
            include_deleted: true,
            limit: Some(10),
        })
        .await
        .expect("file_search");
    assert!(r.files.is_empty());
    m.assert_async().await;
}

#[tokio::test]
async fn worker_4xx_maps_to_invalid_params_with_reason() {
    let mut server = mockito::Server::new_async().await;
    let _m = server
        .mock("GET", "/v1/files")
        .match_query(mockito::Matcher::Any)
        .with_status(404)
        .with_body(r#"{"error":"not_found","reason":"file"}"#)
        .create_async()
        .await;
    let c = client(server.url());
    let err = c
        .file_get(&FileGetArgs {
            repo_id: "r1".into(),
            path: "missing".into(),
            revision: None,
        })
        .await
        .expect_err("should fail");
    let msg = err.message.to_string();
    assert!(msg.contains("not_found"), "msg = {msg}");
    assert!(msg.contains("\"reason\":\"file\""), "msg = {msg}");
}

#[tokio::test]
async fn worker_5xx_maps_to_internal_error() {
    let mut server = mockito::Server::new_async().await;
    let _m = server
        .mock("POST", "/v1/repos")
        .with_status(500)
        .with_body(r#"{"error":"internal_error"}"#)
        .create_async()
        .await;
    let c = client(server.url());
    let err = c
        .repo_init(&RepoInitArgs { name: "x".into() })
        .await
        .expect_err("should fail");
    assert!(err.message.contains("internal_error"));
}
