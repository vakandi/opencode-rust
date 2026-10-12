//! Contract for `opencode-paths`: the `OPENCODE_PATHS_ROOT` override, and the
//! `<root>/opencode/<file>` two-suffix shape every call site used to spell by hand.
//!
//! Each case pins one behaviour an operator would notice breaking: a root that is ignored,
//! a blank variable that relocates files, a relative root that moves under `chdir`, or a
//! suffix that quietly changes where the database lives.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};

use opencode_paths::{data_file, db_file, provider_auth_dir, resolve_root, ROOT_ENV};

/// `ROOT_ENV` is process-global, so the cases take turns instead of racing each other.
fn serial() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Writes `ROOT_ENV` and puts the previous value back on drop, so a failing assertion
/// cannot leak its value into the next case.
struct RootEnv(Option<String>);

impl RootEnv {
    fn set(value: &str) -> Self {
        Self(write(Some(value.to_string())))
    }

    fn unset() -> Self {
        Self(write(None))
    }
}

impl Drop for RootEnv {
    fn drop(&mut self) {
        write(self.0.take());
    }
}

fn write(value: Option<String>) -> Option<String> {
    let previous = std::env::var(ROOT_ENV).ok();
    match value {
        Some(v) => std::env::set_var(ROOT_ENV, v),
        None => std::env::remove_var(ROOT_ENV),
    }
    previous
}

/// Stands in for a caller's own chain: the crate must never rewrite it, only bypass it.
fn fallback() -> PathBuf {
    PathBuf::from("/opencode-paths-fallback-base")
}

const ABS_ROOT: &str = "/tmp/opencode-paths-abs-root";

#[test]
fn set_root_is_honoured() {
    let _lock = serial();
    let _env = RootEnv::set(ABS_ROOT);

    assert_eq!(resolve_root(fallback), Path::new(ABS_ROOT));
}

#[test]
fn unset_root_falls_back() {
    let _lock = serial();
    let _env = RootEnv::unset();

    assert_eq!(resolve_root(fallback), fallback());
}

#[test]
fn blank_root_falls_back() {
    let _lock = serial();

    for blank in ["", " ", "\t", "  \t "] {
        let _env = RootEnv::set(blank);
        assert_eq!(resolve_root(fallback), fallback(), "root={blank:?}");
    }
}

#[test]
fn relative_root_resolves_against_the_current_directory() {
    let _lock = serial();
    let _env = RootEnv::set("./relative-root");

    let resolved = resolve_root(fallback);
    let cwd = std::env::current_dir().unwrap();

    assert!(resolved.is_absolute(), "root left relative: {resolved:?}");
    assert_eq!(resolved, cwd.join("relative-root"));
    assert!(resolved.starts_with(&cwd), "root escaped cwd: {resolved:?}");
}

#[test]
fn derived_paths_keep_the_opencode_two_suffix_shape() {
    let _lock = serial();

    let _env = RootEnv::set(ABS_ROOT);
    let root = Path::new(ABS_ROOT);
    assert_eq!(
        provider_auth_dir(fallback),
        root.join("opencode").join("data")
    );
    assert_eq!(db_file(fallback), root.join("opencode").join("opencode.db"));
    assert_eq!(
        data_file(fallback, "mcp-auth.json"),
        root.join("opencode").join("mcp-auth.json")
    );

    // The override is only half the contract: without it the caller's own base still wins,
    // and the same two suffixes still hang off it.
    let _env = RootEnv::unset();
    let base = fallback();
    assert_eq!(
        provider_auth_dir(fallback),
        base.join("opencode").join("data")
    );
    assert_eq!(db_file(fallback), base.join("opencode").join("opencode.db"));
    assert_eq!(
        data_file(fallback, "auth.json"),
        base.join("opencode").join("auth.json")
    );
}
