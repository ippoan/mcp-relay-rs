//! Emit TypeScript declarations for every shared DTO into `ts-bindings/`.
//!
//! Run locally with `cargo run --bin gen-ts`. CI (`ref-files-worker
//! .github/workflows/sync-types.yml`) runs the same command, copies the
//! output into `ref-files-worker/src/types/`, and fails if `git diff` is
//! non-empty — that drift gate is what makes the Rust crate the single
//! source of truth.
//!
//! `ts-rs`'s `export_to = "ts-bindings/"` attribute on each struct controls
//! the destination directory; this bin just calls `T::export()` for every
//! type so the writes actually happen.
//!
//! ts-rs prefixes `export_to` with `bindings/` by default, so the natural
//! output would be `bindings/ts-bindings/`. We force the prefix to `./` via
//! `TS_RS_EXPORT_DIR` if the env var isn't already set, which lands every
//! `.ts` file directly under `./ts-bindings/`. The worker's
//! `sync-types.yml` relies on that exact path.

use ref_files_mcp_server_rs::types::*;
use ts_rs::TS;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Override ts-rs's default `bindings/` prefix unless the caller already
    // set one. Safe to mutate before the first export() call.
    if std::env::var_os("TS_RS_EXPORT_DIR").is_none() {
        // SAFETY: single-threaded main, no other thread can observe env yet.
        unsafe {
            std::env::set_var("TS_RS_EXPORT_DIR", ".");
        }
    }

    // Repo
    Repo::export()?;
    RepoInitArgs::export()?;

    // Folder
    Folder::export()?;
    FolderCreateArgs::export()?;
    FolderListArgs::export()?;
    FolderListing::export()?;

    // File
    File::export()?;
    FilePutArgs::export()?;
    FileGetArgs::export()?;
    FileGetResponse::export()?;
    FileHistoryArgs::export()?;
    FileMoveArgs::export()?;
    FileDeleteArgs::export()?;
    FileSearchArgs::export()?;
    FileSearchResult::export()?;

    // Revision
    Revision::export()?;
    RevisionList::export()?;

    eprintln!("ts-bindings/ regenerated.");
    Ok(())
}
