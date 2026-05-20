//! Build script: embed MCP_INTERNAL_SECRET into the release binary so consumers
//! can `curl | bash` the install hook with no secret registration step.
//!
//! The secret is intentionally quasi-public — the true authorization boundary
//! is the JWT signature check inside auth-worker `/mcp/introspect`.
//!
//! 解決順 (src/main.rs `resolve_internal_secret`):
//!   1. `--internal-shared-secret <S>` (CLI)
//!   2. env `REF_FILES_MCP_INTERNAL_SHARED_SECRET`
//!   3. build-time embed `MCP_INTERNAL_SECRET` (← この build.rs が焼き込む)
//!   4. dev fallback `"dev-secret-do-not-use"`

fn main() {
    println!("cargo:rerun-if-env-changed=MCP_INTERNAL_SECRET");
    let value = std::env::var("MCP_INTERNAL_SECRET").unwrap_or_default();
    println!("cargo:rustc-env=MCP_INTERNAL_SECRET={}", value);

    // GitHub Actions の tag push (release.yml on `tags: ["v*"]`) では
    // `GITHUB_REF_TYPE=tag` + `GITHUB_REF_NAME=v0.0.x` が立つので、その時だけ
    // release tag を焼き込む。install-mcp.sh は `--version` 出力で tag mismatch
    // を検出してダウングレードを検知する。
    println!("cargo:rerun-if-env-changed=GITHUB_REF_TYPE");
    println!("cargo:rerun-if-env-changed=GITHUB_REF_NAME");
    let release_tag = match (
        std::env::var("GITHUB_REF_TYPE").ok().as_deref(),
        std::env::var("GITHUB_REF_NAME").ok(),
    ) {
        (Some("tag"), Some(name)) => name,
        _ => String::new(),
    };
    println!("cargo:rustc-env=BUILD_RELEASE_TAG={}", release_tag);
}
