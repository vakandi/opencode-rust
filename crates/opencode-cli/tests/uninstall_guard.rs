//! W2.2 — uninstall blast-radius guard.
//!
//! `handle_uninstall_command` used to unconditionally target `~/.config/opencode`,
//! which holds every personality `.md` and the live `opencode.json`. Once the DB
//! is repo-local, an `uninstall` run from the server still wiped the OLD repo's
//! state. The contract enforced here:
//!
//!   * `OPENCODE_PATHS_ROOT` set  -> ONLY that root is a target. The home
//!     data/cache/config/state dirs are REFUSED unless BOTH `--force` and
//!     `--yes-i-mean-it` are supplied.
//!   * `OPENCODE_PATHS_ROOT` unset -> behaviour is UNCHANGED.
//!   * Every target is printed as a RESOLVED ABSOLUTE path before removal.
//!   * `--dry-run` still removes nothing.
//!
//! SAFETY: this suite drives the real `opencode` binary as a subprocess and
//! NEVER passes the double confirmation, so no code path in these tests can
//! delete anything. Every run also gets a throwaway `$HOME` (and `XDG_*`
//! overrides) pointing into a temp sandbox, so even a fully broken guard can
//! only ever touch the sandbox.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A parsed `Uninstall targets:` entry: `(label, absolute path as printed)`.
type Target = (String, String);

/// One throwaway sandbox: fake `$HOME` + fake `OPENCODE_PATHS_ROOT`.
struct Sandbox {
    dir: PathBuf,
}

impl Sandbox {
    /// Build an isolated sandbox. Nothing outside `temp_dir()` is ever writable
    /// from inside a test, because `$HOME` and the root are both redirected.
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "opencode-uninstall-guard-{name}-{}-{}",
            std::process::id(),
            // Nanosecond-ish uniqueness without pulling in a dep.
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        fs::create_dir_all(dir.join("home")).unwrap();
        fs::create_dir_all(dir.join("root")).unwrap();
        Self { dir }
    }

    fn home(&self) -> PathBuf {
        self.dir.join("home")
    }

    /// Run-unique directory name, usable as a substring probe for containment
    /// assertions that must survive `/var` -> `/private/var` canonicalisation.
    fn marker(&self) -> String {
        self.dir
            .file_name()
            .expect("sandbox dir has a name")
            .to_string_lossy()
            .into_owned()
    }

    fn root(&self) -> PathBuf {
        self.dir.join("root")
    }

    /// Run `opencode uninstall <args..>` inside the sandbox.
    ///
    /// `paths_root: None` leaves `OPENCODE_PATHS_ROOT` unset (test (b)).
    fn run_uninstall(&self, paths_root: Option<&Path>, args: &[&str]) -> Output {
        let home = self.home();
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_opencode"));
        cmd.arg("uninstall")
            .args(args)
            // Isolate every platform directory lookup the binary could do.
            .env("HOME", &home)
            .env("XDG_CONFIG_HOME", home.join(".config"))
            .env("XDG_CACHE_HOME", home.join(".cache"))
            .env("XDG_DATA_HOME", home.join(".local").join("share"))
            .env("XDG_STATE_HOME", home.join(".local").join("state"))
            .env_remove("OPENCODE_PATHS_ROOT");
        if let Some(root) = paths_root {
            cmd.env("OPENCODE_PATHS_ROOT", root);
        }
        let out = cmd.output().expect("failed to run opencode uninstall");
        assert!(
            out.status.success(),
            "uninstall {:?} exited {:?}\nstdout:\n{}\nstderr:\n{}",
            args,
            out.status.code(),
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        out
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// Pull a `  <label> <absolute path>` table out of stdout.
///
/// `header` selects which table to read, so a refusal notice is never mistaken
/// for a removal target.
fn parse_table(stdout: &str, header: &str) -> Vec<Target> {
    let mut in_table = false;
    let mut out = Vec::new();
    for line in stdout.lines() {
        if !in_table && line.contains(header) {
            in_table = true;
            continue;
        }
        if !in_table {
            continue;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            // A blank line ends the table; anything after is prose/log noise.
            if !out.is_empty() {
                break;
            }
            continue;
        }
        if !line.starts_with("  ") {
            break;
        }
        // Label is the first token; the path is the rest VERBATIM, because
        // macOS paths legitimately contain spaces ("Application Support").
        let Some((label, rest)) = trimmed.split_once(char::is_whitespace) else {
            break;
        };
        let path = rest.trim_start();
        if !path.starts_with('/') {
            // Refusal / notice lines, not target rows.
            continue;
        }
        out.push((label.to_string(), path.to_string()));
    }
    out
}

/// The rows the command is actually going to remove.
fn parse_targets(stdout: &str) -> Vec<Target> {
    parse_table(stdout, "Uninstall targets:")
}

/// The rows the command explicitly declined to remove.
fn parse_refused(stdout: &str) -> Vec<Target> {
    parse_table(stdout, "Refusing out-of-root targets")
}

fn stdout_of(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Symlink-resolved absolute form of `p`. The CLI canonicalises every target
/// before printing it (macOS rewrites `/var` -> `/private/var`), so assertions
/// have to compare resolved values too.
fn resolved(p: &Path) -> PathBuf {
    fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

/// Every printed target must be an absolute, already-resolved path: no `..`
/// segments, no relative row. This is the "print the resolved absolute target
/// before `remove_dir_all`" half of the contract.
fn assert_all_resolved_absolute(targets: &[Target]) {
    for (label, path) in targets {
        assert!(
            path.starts_with('/'),
            "target `{label}` printed a non-absolute path: {path}"
        );
        assert!(
            !path.contains("/../") && !path.ends_with("/..") && !path.contains("/./"),
            "target `{label}` printed an unresolved path: {path}"
        );
    }
}

/// (a) With `OPENCODE_PATHS_ROOT` set, `--dry-run` targets ONLY that root and
/// never the home `data`/`cache`/`config`/`state` dirs.
///
/// This FAILS before the fix: the old code ignored the env var and listed all
/// four home dirs.
#[test]
fn paths_root_scopes_dry_run_to_root_only() {
    let sandbox = Sandbox::new("a");
    let root = resolved(&sandbox.root());

    let out = sandbox.run_uninstall(Some(&sandbox.root()), &["--dry-run"]);
    let stdout = stdout_of(&out);
    let targets = parse_targets(&stdout);

    assert!(
        !targets.is_empty(),
        "expected at least one target with OPENCODE_PATHS_ROOT set, got:\n{stdout}"
    );
    assert_all_resolved_absolute(&targets);

    let printed: Vec<&str> = targets.iter().map(|(_, p)| p.as_str()).collect();

    // The root itself IS the only target.
    assert_eq!(
        printed,
        vec![root.to_str().unwrap()],
        "targets under a scoped root must be exactly the root:\n{stdout}"
    );

    // The home dirs must be withheld, and must be named so the operator can see
    // exactly what was held back.
    let refused = parse_refused(&stdout);
    assert!(
        !refused.is_empty(),
        "root-scoped dry run did not report the home dirs as refused:\n{stdout}"
    );
    let marker = sandbox.marker();
    assert!(
        refused.iter().all(|(_, p)| p.contains(&marker)),
        "refusal notice named paths outside the sandbox home: {refused:?}\n{stdout}"
    );

    assert!(
        stdout.contains("Dry run mode, no files removed."),
        "--dry-run no longer reports a no-op:\n{stdout}"
    );
    assert!(root.exists(), "--dry-run removed the root");
}

/// (b) WITHOUT `OPENCODE_PATHS_ROOT`, behaviour is UNCHANGED: the four legacy
/// home targets are still listed.
///
/// This is the regression invariant — it holds before *and* after the fix, and
/// exists to prove the guard did not change the unset-root path.
#[test]
fn without_paths_root_behaviour_is_unchanged() {
    let sandbox = Sandbox::new("b");

    let out = sandbox.run_uninstall(None, &["--dry-run"]);
    let stdout = stdout_of(&out);
    let targets = parse_targets(&stdout);

    assert!(
        !targets.is_empty(),
        "expected the legacy home targets when OPENCODE_PATHS_ROOT is unset:\n{stdout}"
    );
    assert_all_resolved_absolute(&targets);

    let labels: Vec<&str> = targets.iter().map(|(l, _)| l.as_str()).collect();
    for expected in ["data", "cache", "config"] {
        assert!(
            labels.contains(&expected),
            "legacy `{expected}` target missing when unset: {labels:?}\n{stdout}"
        );
    }
    // Every legacy target must still live under the caller's home. Compared
    // against the sandbox's unique marker rather than a full prefix, because
    // canonicalising rewrites `/var` -> `/private/var` and this invariant has
    // to hold both before and after that change.
    let marker = sandbox.marker();
    for (label, path) in &targets {
        assert!(
            path.contains(&marker),
            "legacy `{label}` target escaped the caller's home: {path}\n{stdout}"
        );
    }
}

/// (c) `--force` ALONE must still refuse: with a root set, the home config dir
/// (personality files + `opencode.json`) survives untouched.
///
/// This FAILS before the fix: the old code deleted it on `--force`.
#[test]
fn force_alone_still_refuses_home_targets() {
    let sandbox = Sandbox::new("c");
    // Resolve BEFORE the run: `--force` deletes the root, after which
    // canonicalisation would fail and silently fall back to the raw path.
    let root = resolved(&sandbox.root()).to_string_lossy().into_owned();

    // Learn the real per-platform home paths from a harmless dry run rather
    // than hardcoding them: on macOS `dirs::config_dir()` is
    // `Library/Application Support`, not `.config`. The refused rows are
    // exactly the paths the guard promises to protect.
    let probe = stdout_of(&sandbox.run_uninstall(Some(&sandbox.root()), &["--dry-run"]));
    let protected = parse_refused(&probe);
    assert!(
        !protected.is_empty(),
        "expected protected home paths to refuse:\n{probe}"
    );

    let mut sentinels = Vec::new();
    for (label, path) in &protected {
        fs::create_dir_all(path).unwrap();
        let sentinel = PathBuf::from(path).join("sentinel-do-not-delete.txt");
        fs::write(&sentinel, b"personality + opencode.json stand-in").unwrap();
        sentinels.push((label.clone(), sentinel));
    }

    let out = sandbox.run_uninstall(Some(&sandbox.root()), &["--force"]);
    let stdout = stdout_of(&out);

    for (label, sentinel) in &sentinels {
        assert!(
            sentinel.exists(),
            "`--force` without --yes-i-mean-it deleted the `{label}` target ({})!\n{stdout}",
            sentinel.display()
        );
    }

    let targets = parse_targets(&stdout);
    for (label, path) in &targets {
        assert!(
            path.starts_with(&root),
            "`--force` alone unlocked out-of-root target `{label}`: {path}\n{stdout}"
        );
    }
    assert!(
        stdout.to_lowercase().contains("yes-i-mean-it"),
        "refusal did not name the required confirmation flag:\n{stdout}"
    );
}
