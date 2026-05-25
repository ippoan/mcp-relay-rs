//! ref-files-mcp-server-rs — binary entry.
//!
//! Phase 2 (issue #4) で subcommand 構造に再編。github-mcp-server-rs と同様、
//! auth-worker device flow / 1-click pair / outbound WS relay を共有 crate
//! `mcp-relay` 経由で実装する。
//!
//! Subcommands:
//!   - `auth`   RFC 8628 device flow を実行して token を `~/.config/.../token-{env}.json` に保存
//!   - `whoami` cache から token を読み introspect (ref-files-worker JWT の検証)
//!   - `logout` token cache を削除
//!   - `doctor` 効果的な config (URLs, cache path) を secrets 抜きで表示
//!   - `relay`  outbound WS で auth-worker `mcp(-staging).ippoan.org` に接続
//!     (`/u/<login>/connect`) し、`POST /u/<login>/mcp` を Frame::Req として受けて
//!     rmcp `StreamableHttpService` に dispatch
//!   - `pair`   1-click pair flow (POST /mcp/pair/new -> pair_url 印字 -> WS upgrade polling)
//!   - `serve`  Phase 1 互換: 127.0.0.1 に Streamable HTTP を直接 bind
//!     (local dev / `--worker-base` + `--jwt` 固定運用)
//!
//! 共通 flag (`--env staging|prod`, `--auth-base`, `--relay-base`, `--worker-base`,
//! `--internal-shared-secret`, `--client-id`, `--scope`) は全 subcommand で共有。
//! `internal_shared_secret` 解決順:
//!   1. `--internal-shared-secret <S>` (CLI)
//!   2. env `REF_FILES_MCP_INTERNAL_SHARED_SECRET`
//!   3. build-time embed `MCP_INTERNAL_SECRET` (release binary に焼き込み — build.rs)
//!   4. dev fallback `"dev-secret-do-not-use"` (本物 auth-worker は 401)

use anyhow::{anyhow, Context, Result};
use clap::{Parser, Subcommand};
use reqwest::Client;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

use mcp_relay::config::{AuthEnv, Config};
use mcp_relay::relay::{self, PairRelayContext, RelayContext};
use mcp_relay::token_cache::TokenSet;
use mcp_relay::{auth, pair};

use ref_files_mcp_server_rs::mcp_server::RefFilesMcp;
use ref_files_mcp_server_rs::worker_client::WorkerClient;
use rmcp::transport::streamable_http_server::{
    session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
};

/// `BUILD_RELEASE_TAG` が build.rs で埋まっていれば `0.x.y (vA.B.C)` の
/// 形で `--version` に出す。tag 外の `cargo build` では空文字列なので素の `CARGO_PKG_VERSION`。
/// build.rs が常に env var を出力する (空文字列含む) のでこの env!() は失敗しない。
const VERSION: &str = if env!("BUILD_RELEASE_TAG").is_empty() {
    env!("CARGO_PKG_VERSION")
} else {
    concat!(
        env!("CARGO_PKG_VERSION"),
        " (",
        env!("BUILD_RELEASE_TAG"),
        ")"
    )
};

#[derive(Parser, Debug)]
#[command(
    version = VERSION,
    about = "Reference file storage MCP server with auth-worker device flow / pair / relay"
)]
struct Cli {
    /// Target environment (URL preset for auth-worker / relay-worker)
    #[arg(long, value_enum, default_value_t = AuthEnv::Staging, global = true)]
    env: AuthEnv,

    /// Override auth-worker base URL (e.g. https://xxx.trycloudflare.com for wt-quick).
    #[arg(long, global = true)]
    auth_base: Option<String>,

    /// Override MCP relay base URL (default: https://mcp(-staging).ippoan.org from env).
    #[arg(long, global = true)]
    relay_base: Option<String>,

    /// Base URL of ref-files-worker (D1/R2 facade)。Serve / Relay / Pair すべてで使う。
    /// 省略時は env preset (staging / prod とも `https://ref-files.ippoan.org`)。
    /// ref-files-worker は 2026-05-25 以降 single-env 運用 (Refs ippoan/ref-files-worker#6)。
    #[arg(long, env = "REF_FILES_WORKER_BASE", global = true)]
    worker_base: Option<String>,

    /// auth-worker INTERNAL_SHARED_SECRET (release binary は build-time embed されているので通常未指定)。
    #[arg(long, env = "REF_FILES_MCP_INTERNAL_SHARED_SECRET", global = true)]
    internal_shared_secret: Option<String>,

    /// MCP client_id sent to auth-worker
    #[arg(
        long,
        env = "REF_FILES_MCP_CLIENT_ID",
        default_value = "ref-files-mcp-server-rs",
        global = true
    )]
    client_id: String,

    /// MCP scope
    #[arg(long, default_value = "mcp.read mcp.write", global = true)]
    scope: String,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug, Clone)]
enum Command {
    /// Run device authorization grant flow, save token to cache
    Auth,
    /// Use cached token (auto-refresh if expired) to call `/mcp/introspect`
    /// and print github_login + scope. Verifies the JWT works end-to-end.
    Whoami,
    /// Delete the cached token for the selected env
    Logout,
    /// Show effective config (URLs, cache path) without secrets
    Doctor,
    /// Run as outbound WS relay client connecting to auth-worker
    /// (`/u/<github_login>/connect`). Claude Code Web `POST /u/<login>/mcp`
    /// が Frame::Req として届く。
    Relay {
        /// `--user` で github_login を明示。省略時は `/mcp/introspect` で resolve。
        #[arg(long)]
        user: Option<String>,
        /// State directory (install-mcp.sh `$STATE_DIR`)。設定すると
        /// `<state-dir>/url` に固定 URL を書き出す。
        #[arg(long)]
        state_dir: Option<PathBuf>,
        /// Status sentinel を stderr に出力する (install-mcp.sh が grep)。
        #[arg(long, default_value_t = true)]
        print_status: bool,
    },
    /// 1-click pair flow (POST /mcp/pair/new → pair_url 印字 → WS upgrade polling)
    Pair {
        /// github_login を明示。省略時は `$GITHUB_LOGIN` env を読む。両方未設定なら error。
        #[arg(long, env = "GITHUB_LOGIN")]
        user: Option<String>,
        /// State directory (install-mcp.sh `$STATE_DIR`)。
        #[arg(long)]
        state_dir: Option<PathBuf>,
        /// Status sentinel を stderr に出力する。
        #[arg(long, default_value_t = true)]
        print_status: bool,
    },
    /// Phase 1 互換: 127.0.0.1 に Streamable HTTP を直接 bind する local-only mode。
    /// `--jwt` で固定 JWT を渡すこと (Relay/Pair と違って auth-worker を経由しない)。
    Serve {
        /// MCP JWT (Authorization: Bearer)。`$REF_FILES_MCP_JWT` でも可。
        #[arg(long, env = "REF_FILES_MCP_JWT")]
        jwt: String,
        /// Bind address.
        #[arg(long, default_value = "127.0.0.1:7457")]
        bind: String,
        /// 設定済 config と tool docstring を出して exit。
        #[arg(long, default_value_t = false)]
        introspect: bool,
    },
}

/// ref-files-worker base URL (env を問わず単一 endpoint)。
/// ref-files-worker は 2026-05-25 以降 single-env 運用に切替済
/// (Refs ippoan/ref-files-worker#6 / PR #8) — staging / prod とも
/// `ref-files.ippoan.org` を共有する。auth-worker / relay-worker は
/// 引き続き env で staging / prod に分岐するため `AuthEnv` 自体は残す。
fn default_worker_base(_env: AuthEnv) -> &'static str {
    "https://ref-files.ippoan.org"
}

fn build_config(cli: &Cli) -> Result<Config> {
    let auth_base = cli
        .auth_base
        .clone()
        .unwrap_or_else(|| cli.env.default_base().to_string());
    let relay_base = cli
        .relay_base
        .clone()
        .unwrap_or_else(|| cli.env.default_relay_base().to_string());
    let internal_shared_secret = resolve_internal_secret(cli.internal_shared_secret.as_deref());
    Ok(Config {
        env: cli.env,
        auth_base,
        relay_base,
        internal_shared_secret,
        client_id: cli.client_id.clone(),
        scope: cli.scope.clone(),
        project_name: "ref-files-mcp-server-rs",
    })
}

fn resolve_internal_secret(cli: Option<&str>) -> String {
    if let Some(s) = cli {
        if !s.is_empty() {
            return s.to_string();
        }
    }
    let embedded = option_env!("MCP_INTERNAL_SECRET").unwrap_or("");
    if !embedded.is_empty() {
        return embedded.to_string();
    }
    "dev-secret-do-not-use".to_string()
}

fn worker_base_for(cli: &Cli) -> String {
    cli.worker_base
        .clone()
        .unwrap_or_else(|| default_worker_base(cli.env).to_string())
}

async fn run_auth(client: &Client, cfg: &Config) -> Result<()> {
    println!("→ Requesting device code from {} ...", cfg.auth_base);
    let device = auth::start_device_authorization(client, cfg).await?;

    println!();
    println!("┌────────────────────────────────────────────────────");
    println!("│ Open this URL in your browser:");
    println!("│   {}", device.verification_uri_complete);
    println!("│");
    println!("│ Or visit {} and enter:", device.verification_uri);
    println!("│   {}", device.user_code);
    println!("│");
    println!(
        "│ Expires in {} seconds. Polling every {} s ...",
        device.expires_in, device.interval
    );
    println!("└────────────────────────────────────────────────────");
    println!();

    let token = auth::poll_token(client, cfg, &device).await?;
    let path = cfg.token_cache_path()?;
    token.save(&path)?;

    println!("✓ Token saved to {}", path.display());
    println!("  scope:      {}", token.scope);
    println!("  expires_at: {} (Unix epoch)", token.expires_at);
    Ok(())
}

async fn run_whoami(client: &Client, cfg: &Config) -> Result<()> {
    let path = cfg.token_cache_path()?;
    let mut token = TokenSet::load(&path)?.ok_or_else(|| {
        anyhow!(
            "no cached token for env={} — run `auth` first",
            cfg.env.as_str()
        )
    })?;

    if token.is_expired(60) {
        println!("→ Access token expired, refreshing ...");
        token = auth::refresh(client, cfg, &token.refresh_token).await?;
        token.save(&path)?;
    }

    println!("→ Calling /mcp/introspect ...");
    let active = mcp_relay_introspect(client, cfg, &token.access_token)
        .await?
        .ok_or_else(|| anyhow!("introspect returned active:false — token may have been revoked"))?;
    println!("✓ Introspect OK:");
    println!("  sub:          {}", active.sub);
    println!("  github_login: {}", active.github_login);
    println!("  scope:        {}", active.scope);
    println!("  exp:          {} (Unix epoch)", active.exp);
    Ok(())
}

/// `/mcp/introspect` を `internal_shared_secret` 付きで叩く軽量 client。
/// github-mcp の `introspect.rs` の縮小版 (`github_token` field は ref-files の
/// 文脈では使わないので除外)。
///
/// auth-worker の `/mcp/introspect` は **raw secret** を `Authorization` ヘッダに
/// (Bearer prefix なし)、body は **JSON** `{ "token": "<jwt>" }` で期待する。
/// `.bearer_auth()` + `.form()` で送ると 401 を返す (Refs #21)。
async fn mcp_relay_introspect(
    client: &Client,
    cfg: &Config,
    access_token: &str,
) -> Result<Option<IntrospectActive>> {
    #[derive(serde::Deserialize)]
    struct Resp {
        active: bool,
        #[serde(default)]
        sub: String,
        #[serde(default)]
        scope: String,
        #[serde(default)]
        exp: i64,
        #[serde(default)]
        github_login: String,
    }
    let resp = client
        .post(cfg.url("/mcp/introspect"))
        .header("Authorization", cfg.internal_shared_secret.as_str())
        .header("Content-Type", "application/json")
        .body(serde_json::json!({ "token": access_token }).to_string())
        .send()
        .await
        .context("POST /mcp/introspect")?;
    if !resp.status().is_success() {
        anyhow::bail!("introspect HTTP {}", resp.status());
    }
    let body: Resp = resp.json().await.context("introspect json")?;
    if !body.active {
        return Ok(None);
    }
    Ok(Some(IntrospectActive {
        sub: body.sub,
        scope: body.scope,
        exp: body.exp,
        github_login: body.github_login,
    }))
}

#[derive(Debug, Clone)]
struct IntrospectActive {
    sub: String,
    scope: String,
    exp: i64,
    github_login: String,
}

fn run_logout(cfg: &Config) -> Result<()> {
    let path = cfg.token_cache_path()?;
    TokenSet::delete(&path)?;
    println!("✓ Token cache deleted: {}", path.display());
    Ok(())
}

fn run_doctor(cli: &Cli, cfg: &Config) -> Result<()> {
    let cache = cfg.token_cache_path()?;
    let cached = TokenSet::load(&cache)?;
    println!("env:              {}", cfg.env.as_str());
    println!("auth_base:        {}", cfg.auth_base);
    println!("relay_base:       {}", cfg.relay_base);
    println!("worker_base:      {}", worker_base_for(cli));
    println!("client_id:        {}", cfg.client_id);
    println!("scope:            {}", cfg.scope);
    println!(
        "internal_secret:  {}",
        if cfg.internal_shared_secret.is_empty() {
            "(not set)".to_string()
        } else {
            format!("(set, {} chars)", cfg.internal_shared_secret.len())
        }
    );
    println!("token_cache:      {}", cache.display());
    match cached {
        Some(t) => {
            println!("  scope:        {}", t.scope);
            println!("  expires_at:   {}", t.expires_at);
            println!("  expired:      {}", t.is_expired(0));
            println!("  obtained_at:  {}", t.obtained_at);
        }
        None => println!("  (no token cached)"),
    }
    Ok(())
}

/// `https://...` から `host[:port]` を抽出する。`with_allowed_hosts` 用。
fn relay_host_from_base(base: &str) -> Option<String> {
    let trimmed = base.trim();
    let after_scheme = trimmed
        .strip_prefix("https://")
        .or_else(|| trimmed.strip_prefix("http://"))
        .or_else(|| trimmed.strip_prefix("wss://"))
        .or_else(|| trimmed.strip_prefix("ws://"))?;
    let host = after_scheme.split('/').next().unwrap_or("");
    if host.is_empty() {
        None
    } else {
        Some(host.to_string())
    }
}

fn build_streamable_http_service(
    worker: Arc<WorkerClient>,
    cfg: &Config,
) -> StreamableHttpService<RefFilesMcp, LocalSessionManager> {
    let mut allowed_hosts: Vec<String> = vec!["localhost".into(), "127.0.0.1".into(), "::1".into()];
    if let Some(host) = relay_host_from_base(&cfg.relay_base) {
        allowed_hosts.push(host);
    }
    StreamableHttpService::new(
        move || Ok(RefFilesMcp::new(worker.clone())),
        Default::default(),
        StreamableHttpServerConfig::default()
            .with_stateful_mode(false)
            .with_json_response(true)
            .with_allowed_hosts(allowed_hosts),
    )
}

async fn run_relay(
    cli: &Cli,
    client: &Client,
    cfg: &Config,
    user: Option<String>,
    state_dir: Option<PathBuf>,
    print_status: bool,
) -> Result<()> {
    let path = cfg.token_cache_path()?;
    let mut token = TokenSet::load(&path)?.ok_or_else(|| {
        anyhow!(
            "no cached token for env={} — run `auth` first",
            cfg.env.as_str()
        )
    })?;
    if token.is_expired(60) {
        println!("→ Access token expired, refreshing ...");
        token = auth::refresh(client, cfg, &token.refresh_token).await?;
        token.save(&path)?;
    }

    let active = mcp_relay_introspect(client, cfg, &token.access_token)
        .await?
        .ok_or_else(|| anyhow!("introspect returned active:false — token may have been revoked"))?;
    let login = match user {
        Some(u) if u != active.github_login => {
            return Err(anyhow!(
                "--user {} does not match introspected github_login={}",
                u,
                active.github_login
            ));
        }
        Some(u) => u,
        None => active.github_login.clone(),
    };

    let cfg_arc = Arc::new(cfg.clone());
    let token_lock = Arc::new(RwLock::new(token));
    let worker = Arc::new(WorkerClient::with_refreshable_token(
        worker_base_for(cli),
        token_lock.clone(),
        path.clone(),
        cfg_arc.clone(),
    ));
    let svc = build_streamable_http_service(worker, cfg);

    if print_status {
        eprintln!(
            "⇒ MCP relay starting (env={}, user={})",
            cfg.env.as_str(),
            login
        );
    }

    let relay_ctx = RelayContext {
        cfg: cfg_arc,
        http: client.clone(),
        login,
        jwt: token_lock,
        jwt_cache_path: path,
        svc,
        state_dir,
        print_status,
        service: "ref-files-mcp-server-rs",
        binary_version: env!("CARGO_PKG_VERSION"),
    };

    relay::run_relay(relay_ctx).await
}

async fn run_pair(
    cli: &Cli,
    client: &Client,
    cfg: &Config,
    user: Option<String>,
    state_dir: Option<PathBuf>,
    print_status: bool,
) -> Result<()> {
    let login = match user.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(s) => s.to_string(),
        None => {
            return Err(anyhow!(
                "pair: github login not provided.\n\
                 \n\
                 hint: pass `--user <github_login>` or set `$GITHUB_LOGIN` env."
            ));
        }
    };

    let binary_version = VERSION;
    if print_status {
        eprintln!(
            "→ pair: POST {} (claim_login={login}, binary_version=\"{binary_version}\")",
            cfg.pair_new_url()
        );
    }
    let resp = pair::pair_new(client, cfg, &login, binary_version).await?;
    println!("{}", resp.pair_url);
    if print_status {
        eprintln!(
            "⇒ pair_url surfaced (expires in {}s, pair_code len={})",
            resp.expires_in,
            resp.pair_code.len()
        );
        eprintln!("   {}", resp.pair_url);
    }

    // pair flow は WS attach 専用。tool 呼び出しは pair が成立して relay loop に
    // 入ってからは Static JWT (空) で叩くと 401 になるが、relay 側で Frame::Req
    // を受けて WorkerClient::with_refreshable_token と同じ refresh 経路を回す
    // ためには pair セッション内で TokenSet が用意できない (まだ device flow を
    // 通っていない)。pair 直後は tools/list だけ通り、tools/call は 401 になる
    // 設計 (github-mcp と同じ degraded mode)。
    let worker = Arc::new(WorkerClient::with_static_token(
        worker_base_for(cli),
        String::new(),
    ));
    let svc = build_streamable_http_service(worker, cfg);

    let deadline =
        tokio::time::Instant::now() + Duration::from_secs(resp.expires_in.clamp(60, 600));
    let pair_ctx = PairRelayContext {
        cfg: Arc::new(cfg.clone()),
        login,
        svc,
        state_dir,
        print_status,
        service: "ref-files-mcp-server-rs",
        binary_version: env!("CARGO_PKG_VERSION"),
    };
    relay::run_pair_session(pair_ctx, resp.pair_code, deadline).await
}

async fn run_serve(cli: &Cli, jwt: String, bind: String, introspect_only: bool) -> Result<()> {
    let worker_base = worker_base_for(cli);
    let worker = Arc::new(WorkerClient::with_static_token(worker_base.clone(), jwt));
    if introspect_only {
        let server = RefFilesMcp::new(worker);
        let info = <RefFilesMcp as rmcp::ServerHandler>::get_info(&server);
        println!(
            "ref-files-mcp-server-rs: worker_base={} bind={}",
            worker_base, bind
        );
        println!("instructions: {}", info.instructions.unwrap_or_default());
        return Ok(());
    }

    let addr: SocketAddr = bind
        .parse()
        .with_context(|| format!("parse --bind {}", bind))?;
    let allowed_hosts: Vec<String> = vec![
        format!("{}", addr),
        "localhost".into(),
        "127.0.0.1".into(),
        "::1".into(),
    ];
    let svc: StreamableHttpService<RefFilesMcp, LocalSessionManager> = StreamableHttpService::new(
        move || Ok(RefFilesMcp::new(worker.clone())),
        Default::default(),
        StreamableHttpServerConfig::default()
            .with_stateful_mode(false)
            .with_json_response(true)
            .with_allowed_hosts(allowed_hosts),
    );

    let app = axum::Router::new().fallback_service(svc);
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("bind {addr}"))?;
    tracing::info!(
        addr = %addr,
        worker_base = %worker_base,
        "ref-files-mcp-server-rs (serve) listening"
    );
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("serve")?;
    Ok(())
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(false)
        .init();

    let cli = Cli::parse();
    let cfg = build_config(&cli)?;
    let client = Client::builder()
        .user_agent(concat!(
            "ref-files-mcp-server-rs/",
            env!("CARGO_PKG_VERSION")
        ))
        .timeout(Duration::from_secs(30))
        .build()
        .context("build reqwest client")?;

    match cli.command.clone() {
        Command::Auth => run_auth(&client, &cfg).await,
        Command::Whoami => run_whoami(&client, &cfg).await,
        Command::Logout => run_logout(&cfg),
        Command::Doctor => run_doctor(&cli, &cfg),
        Command::Relay {
            user,
            state_dir,
            print_status,
        } => run_relay(&cli, &client, &cfg, user, state_dir, print_status).await,
        Command::Pair {
            user,
            state_dir,
            print_status,
        } => run_pair(&cli, &client, &cfg, user, state_dir, print_status).await,
        Command::Serve {
            jwt,
            bind,
            introspect,
        } => run_serve(&cli, jwt, bind, introspect).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mcp_relay::config::AuthEnv;

    fn test_cfg(base: &str) -> Config {
        Config {
            env: AuthEnv::Staging,
            auth_base: base.to_string(),
            relay_base: base.to_string(),
            client_id: "test-client".to_string(),
            scope: "mcp.read mcp.write".to_string(),
            internal_shared_secret: "test-secret".to_string(),
            project_name: "ref-files-mcp-server-rs",
        }
    }

    /// Regression for #21: `/mcp/introspect` must be called with
    /// `Authorization: <raw-secret>` (no `Bearer ` prefix) and a JSON body —
    /// not `bearer_auth()` + form-encoded body, which auth-worker 401s.
    #[tokio::test]
    async fn introspect_uses_raw_authorization_and_json_body() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/mcp/introspect")
            .match_header("authorization", "test-secret")
            .match_header("content-type", "application/json")
            .match_body(mockito::Matcher::Json(serde_json::json!({
                "token": "test-access-token"
            })))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"active":true,"sub":"u1","scope":"mcp.read","exp":9999999999,"github_login":"alice"}"#,
            )
            .create_async()
            .await;

        let cfg = test_cfg(&server.url());
        let client = Client::new();
        let active = mcp_relay_introspect(&client, &cfg, "test-access-token")
            .await
            .expect("introspect call should succeed")
            .expect("active=true expected");

        assert_eq!(active.github_login, "alice");
        assert_eq!(active.sub, "u1");
        mock.assert_async().await;
    }

    /// 401 response → bubbles up as an error mentioning the status.
    #[tokio::test]
    async fn introspect_propagates_401() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("POST", "/mcp/introspect")
            .with_status(401)
            .with_body("Unauthorized")
            .create_async()
            .await;

        let cfg = test_cfg(&server.url());
        let client = Client::new();
        let err = mcp_relay_introspect(&client, &cfg, "tok")
            .await
            .expect_err("401 must surface as Err");
        assert!(err.to_string().contains("401"));
    }
}
