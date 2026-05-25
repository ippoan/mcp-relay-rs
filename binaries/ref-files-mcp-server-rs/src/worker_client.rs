//! HTTP client for `ref-files-worker`.
//!
//! Every MCP tool ultimately makes one or two HTTP calls into the worker.
//! Centralising URL building, JSON (de)serialisation, error mapping, and
//! `Authorization: Bearer` forwarding here means each tool stays a thin
//! adapter between rmcp's argument struct and the worker's wire shape.
//!
//! Phase 2 (issue #4): JWT は `TokenSource` で 2 mode を区別する。
//!  - `Static`: Phase 1 互換 (`--jwt` / `$REF_FILES_MCP_JWT` で固定 JWT を渡す)。
//!  - `Refreshable`: `mcp_relay::auth::refresh` を使った自動更新 (`Relay` /
//!    `Pair` subcommand)。401 を受け取ったら 1 回だけ refresh + retry する。
//!
//! Errors surface as `rmcp::ErrorData` because that's what `#[tool]` handlers
//! return — `worker_error()` shapes the body as
//! `{ status, error, reason? }` so the LLM caller sees the worker's
//! structured failure (e.g. `not_found` / `forbidden` / `conflict`).

use reqwest::{Client, Method, StatusCode};
use rmcp::{model::ErrorCode, ErrorData};
use serde::{de::DeserializeOwned, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::types::{
    File, FileDeleteArgs, FileGetArgs, FileGetResponse, FileHistoryArgs, FileMoveArgs, FilePutArgs,
    FileSearchArgs, FileSearchResult, Folder, FolderCreateArgs, FolderListArgs, FolderListing,
    Repo, RepoInitArgs, RevisionList,
};

/// Configuration baked into the MCP server at startup. Phase 1 compat shape:
/// `Serve` subcommand と既存 test がそのまま使う。Phase 2 の `Relay` / `Pair`
/// では `WorkerClient::with_refreshable_token` を直接呼ぶ。
#[derive(Debug, Clone)]
pub struct WorkerConfig {
    /// e.g. `https://ref-files.ippoan.org` or `http://127.0.0.1:8787` for vitest/dev.
    pub base_url: String,
    /// Phase 1: 固定 JWT を caller (CLI / env) が渡す。Phase 2 では空文字列が
    /// 入っていても `with_refreshable_token` 経由なら無視される。
    pub jwt: String,
}

/// `WorkerClient` が JWT を解決する方法。`Static` は Phase 1、`Refreshable` は
/// Phase 2 (auth-worker 連動)。
#[derive(Clone)]
pub enum TokenSource {
    /// 固定 JWT (Serve subcommand)。caller が起動時に焼き付ける。401 でも refresh しない。
    Static(String),
    /// `Arc<RwLock<TokenSet>>` を共有し、401 で `mcp_relay::auth::refresh` を 1 回試行する。
    /// `Relay` / `Pair` subcommand 用。
    Refreshable {
        cache: Arc<RwLock<mcp_relay::token_cache::TokenSet>>,
        /// `~/.config/ref-files-mcp-server-rs/token-{env}.json` 上書き先。
        cache_path: PathBuf,
        /// auth-worker 接続情報 (refresh URL は `cfg.url("/mcp/refresh")` 経由)。
        cfg: Arc<mcp_relay::config::Config>,
    },
}

#[derive(Clone)]
pub struct WorkerClient {
    base_url: String,
    http: Client,
    token: TokenSource,
}

impl WorkerClient {
    /// Phase 1 互換 constructor (test と Serve subcommand)。
    pub fn new(cfg: WorkerConfig) -> Self {
        Self::with_static_token(cfg.base_url, cfg.jwt)
    }

    /// 任意の `reqwest::Client` を inject する Phase 1 constructor (test 用)。
    pub fn with_client(cfg: WorkerConfig, http: Client) -> Self {
        Self {
            base_url: cfg.base_url,
            http,
            token: TokenSource::Static(cfg.jwt),
        }
    }

    /// 固定 JWT を持つ client を組み立てる (Serve subcommand)。
    pub fn with_static_token(base_url: String, jwt: String) -> Self {
        Self {
            base_url,
            http: Client::new(),
            token: TokenSource::Static(jwt),
        }
    }

    /// Phase 2: `Arc<RwLock<TokenSet>>` を共有 state として持つ client を組み立てる。
    /// 401 受信時に 1 回だけ `mcp_relay::auth::refresh` を試行し、新 token を
    /// `cache_path` に save する。relay loop 側も同じ `Arc<RwLock<TokenSet>>` を
    /// 持つので refresh 結果は即座に共有される。
    pub fn with_refreshable_token(
        base_url: String,
        cache: Arc<RwLock<mcp_relay::token_cache::TokenSet>>,
        cache_path: PathBuf,
        cfg: Arc<mcp_relay::config::Config>,
    ) -> Self {
        Self {
            base_url,
            http: Client::new(),
            token: TokenSource::Refreshable {
                cache,
                cache_path,
                cfg,
            },
        }
    }

    /// Useful for tests that want to inject a `mockito::Server` URL after
    /// construction.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// 現在の access_token を読み出す。`Refreshable` mode では RwLock を read
    /// lock して clone するだけなので await は短時間。
    async fn current_token(&self) -> String {
        match &self.token {
            TokenSource::Static(s) => s.clone(),
            TokenSource::Refreshable { cache, .. } => cache.read().await.access_token.clone(),
        }
    }

    /// 401 を受けた時に呼び出される refresh path。`Static` は no-op (false 返却)。
    /// `Refreshable` は `mcp_relay::auth::refresh` を 1 回試行し、成功時に
    /// cache に書き戻して true、失敗時に false。
    async fn try_refresh(&self) -> bool {
        match &self.token {
            TokenSource::Static(_) => false,
            TokenSource::Refreshable {
                cache,
                cache_path,
                cfg,
            } => {
                let refresh_token = cache.read().await.refresh_token.clone();
                if refresh_token.is_empty() {
                    return false;
                }
                let new_token =
                    match mcp_relay::auth::refresh(&self.http, cfg, &refresh_token).await {
                        Ok(t) => t,
                        Err(e) => {
                            tracing::warn!("worker_client: refresh failed: {e}");
                            return false;
                        }
                    };
                if let Err(e) = new_token.save(cache_path) {
                    tracing::warn!("worker_client: refresh save failed: {e}");
                }
                *cache.write().await = new_token;
                true
            }
        }
    }

    async fn request<B: Serialize + ?Sized, R: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<&B>,
        query: &[(&str, String)],
    ) -> Result<R, ErrorData> {
        // 401 で 1 回だけ refresh + retry する。Static mode は refresh 失敗
        // (try_refresh が false) で抜けて即 401 を返す。
        for attempt in 0..2 {
            let url = format!("{}{}", self.base_url.trim_end_matches('/'), path);
            let token = self.current_token().await;
            let mut req = self
                .http
                .request(method.clone(), &url)
                .bearer_auth(&token)
                .query(query);
            if let Some(b) = body {
                req = req.json(b);
            }
            let resp = req
                .send()
                .await
                .map_err(|e| ErrorData::new(ErrorCode::INTERNAL_ERROR, e.to_string(), None))?;
            let status = resp.status();
            let bytes = resp
                .bytes()
                .await
                .map_err(|e| ErrorData::new(ErrorCode::INTERNAL_ERROR, e.to_string(), None))?;
            if status == StatusCode::UNAUTHORIZED && attempt == 0 {
                if self.try_refresh().await {
                    continue;
                }
                return Err(worker_error(status, &bytes));
            }
            if !status.is_success() {
                return Err(worker_error(status, &bytes));
            }
            return serde_json::from_slice::<R>(&bytes).map_err(|e| {
                ErrorData::new(
                    ErrorCode::INTERNAL_ERROR,
                    format!("worker_response_parse: {e}"),
                    None,
                )
            });
        }
        // unreachable: ループは 401 + refresh 失敗時に return 済み
        Err(ErrorData::new(
            ErrorCode::INTERNAL_ERROR,
            "worker_client: retry loop exhausted".to_string(),
            None,
        ))
    }

    // ── tool-shaped wrappers ───────────────────────────────────────────────

    pub async fn repo_init(&self, args: &RepoInitArgs) -> Result<Repo, ErrorData> {
        self.request::<_, Repo>(Method::POST, "/v1/repos", Some(args), &[])
            .await
    }

    pub async fn folder_create(&self, args: &FolderCreateArgs) -> Result<Folder, ErrorData> {
        self.request::<_, Folder>(Method::POST, "/v1/folders", Some(args), &[])
            .await
    }

    pub async fn folder_list(&self, args: &FolderListArgs) -> Result<FolderListing, ErrorData> {
        let mut q: Vec<(&str, String)> = vec![("repo_id", args.repo_id.clone())];
        if !args.path.is_empty() {
            q.push(("path", args.path.clone()));
        }
        if args.recursive {
            q.push(("recursive", "true".into()));
        }
        self.request::<(), FolderListing>(Method::GET, "/v1/folders", None, &q)
            .await
    }

    pub async fn file_put(&self, args: &FilePutArgs) -> Result<FileGetResponse, ErrorData> {
        self.request::<_, FileGetResponse>(Method::POST, "/v1/files", Some(args), &[])
            .await
    }

    pub async fn file_get(&self, args: &FileGetArgs) -> Result<FileGetResponse, ErrorData> {
        let mut q: Vec<(&str, String)> = vec![
            ("repo_id", args.repo_id.clone()),
            ("path", args.path.clone()),
        ];
        if let Some(rev) = args.revision {
            q.push(("revision", rev.to_string()));
        }
        self.request::<(), FileGetResponse>(Method::GET, "/v1/files", None, &q)
            .await
    }

    pub async fn file_history(&self, args: &FileHistoryArgs) -> Result<RevisionList, ErrorData> {
        let mut q: Vec<(&str, String)> = vec![
            ("repo_id", args.repo_id.clone()),
            ("path", args.path.clone()),
        ];
        if let Some(l) = args.limit {
            q.push(("limit", l.to_string()));
        }
        self.request::<(), RevisionList>(Method::GET, "/v1/files/history", None, &q)
            .await
    }

    pub async fn file_move(&self, args: &FileMoveArgs) -> Result<File, ErrorData> {
        self.request::<_, File>(Method::POST, "/v1/files/move", Some(args), &[])
            .await
    }

    pub async fn file_delete(&self, args: &FileDeleteArgs) -> Result<File, ErrorData> {
        let q = [
            ("repo_id", args.repo_id.clone()),
            ("path", args.path.clone()),
        ];
        self.request::<(), File>(Method::DELETE, "/v1/files", None, &q)
            .await
    }

    pub async fn file_search(&self, args: &FileSearchArgs) -> Result<FileSearchResult, ErrorData> {
        let mut q: Vec<(&str, String)> = vec![
            ("repo_id", args.repo_id.clone()),
            ("query", args.query.clone()),
        ];
        if let Some(p) = &args.under_path {
            q.push(("under_path", p.clone()));
        }
        if args.include_deleted {
            q.push(("include_deleted", "true".into()));
        }
        if let Some(l) = args.limit {
            q.push(("limit", l.to_string()));
        }
        self.request::<(), FileSearchResult>(Method::GET, "/v1/files/search", None, &q)
            .await
    }
}

fn worker_error(status: StatusCode, body: &[u8]) -> ErrorData {
    // The worker emits `{ error: string, reason?: string }` on non-2xx —
    // preserve that shape so the LLM caller can distinguish (e.g.)
    // `not_found` vs `conflict` without parsing the prose.
    let parsed: Result<serde_json::Value, _> = serde_json::from_slice(body);
    let (error, reason) = match parsed {
        Ok(v) => (
            v.get("error")
                .and_then(|x| x.as_str())
                .unwrap_or("worker_error")
                .to_string(),
            v.get("reason")
                .and_then(|x| x.as_str())
                .map(|s| s.to_string()),
        ),
        Err(_) => ("worker_error".into(), None),
    };
    let body = serde_json::json!({
        "status": status.as_u16(),
        "error": error,
        "reason": reason,
    });
    // 4xx → INVALID_PARAMS (caller can fix it). 5xx and anything else → INTERNAL_ERROR.
    let code = if status.is_client_error() {
        ErrorCode::INVALID_PARAMS
    } else {
        ErrorCode::INTERNAL_ERROR
    };
    ErrorData::new(code, body.to_string(), None)
}
