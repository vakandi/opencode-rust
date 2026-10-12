//! One lever for where opencode keeps its files.
//!
//! Every call site used to spell its own chain out — a `dirs::*_dir()` lookup, a
//! `.join("opencode")`, then a filename — so moving opencode's storage meant editing
//! five unrelated crates and hoping you found them all. `OPENCODE_PATHS_ROOT` is the single
//! lever: set it once and the database, provider credentials, and auth files all follow.
//!
//! Callers hand in their own fallback closure and keep it verbatim. This crate decides
//! *whether* to use the override; it never rewrites *what* a caller's fallback was. That
//! keeps the per-platform difference real: the database resolves under `data_local_dir`
//! while the MCP auth file resolves under `data_dir`, and neither is quietly unified here.

use std::path::PathBuf;

/// Environment variable that relocates every opencode path at once.
pub const ROOT_ENV: &str = "OPENCODE_PATHS_ROOT";

/// The per-platform directory that holds opencode's files, one level under the root.
pub const OPENCODE_DIR: &str = "opencode";

/// The paths root: `OPENCODE_PATHS_ROOT` when it is set to something usable, otherwise the
/// caller's own base directory.
///
/// Unset, empty, and whitespace-only all count as unset. A stray `export OPENCODE_PATHS_ROOT=`
/// must not silently relocate the database into the current working directory. A relative
/// root is anchored to the current directory so the resolved path cannot change under a
/// later `chdir`.
pub fn resolve_root(fallback: impl FnOnce() -> PathBuf) -> PathBuf {
    let Some(raw) = std::env::var(ROOT_ENV)
        .ok()
        .filter(|v| !v.trim().is_empty())
    else {
        return fallback();
    };

    let root = PathBuf::from(raw.trim());
    match root.is_absolute() {
        true => root,
        // No cwd to anchor against: hand back what was asked for rather than invent a base.
        false => std::env::current_dir().map_or_else(|_| root.clone(), |cwd| cwd.join(&root)),
    }
}

/// `<root>/opencode/<name>` — the two suffixes every call site used to append by hand.
pub fn data_file(fallback: impl FnOnce() -> PathBuf, name: &str) -> PathBuf {
    resolve_root(fallback).join(OPENCODE_DIR).join(name)
}

/// `<root>/opencode/data`, where provider and plugin credentials are kept.
pub fn provider_auth_dir(fallback: impl FnOnce() -> PathBuf) -> PathBuf {
    data_file(fallback, "data")
}

/// `<root>/opencode/opencode.db`.
pub fn db_file(fallback: impl FnOnce() -> PathBuf) -> PathBuf {
    data_file(fallback, "opencode.db")
}
