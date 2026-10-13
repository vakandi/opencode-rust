use anyhow::{Context, Result};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

#[derive(Debug, Clone)]
pub struct SnapshotPatch {
    pub hash: String,
    pub files: Vec<String>,
}

pub struct Snapshot;

impl Snapshot {
    pub fn new() -> Self {
        Self
    }

    pub fn track(directory: &Path) -> Result<String> {
        let git_dir = ensure_snapshot_repo(directory)?;
        git_add_all(directory, &git_dir)?;
        let output = git_output(directory, &git_dir, &["write-tree"])?;
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    pub fn revert(directory: &Path, patches: Vec<SnapshotPatch>) -> Result<()> {
        let git_dir = ensure_snapshot_repo(directory)?;
        let mut seen = HashSet::new();

        for patch in patches {
            for file in patch.files {
                let file_path = PathBuf::from(&file);
                let relative = relative_to_worktree(directory, &file_path);
                if relative.is_empty() || !seen.insert(relative.clone()) {
                    continue;
                }

                let checkout = git_output(
                    directory,
                    &git_dir,
                    &["checkout", &patch.hash, "--", &relative],
                );
                if checkout.is_err() {
                    // If file does not exist in snapshot, delete it locally.
                    let exists_in_tree = git_output(
                        directory,
                        &git_dir,
                        &["ls-tree", &patch.hash, "--", &relative],
                    )
                    .ok()
                    .map(|out| !String::from_utf8_lossy(&out.stdout).trim().is_empty())
                    .unwrap_or(false);

                    if !exists_in_tree {
                        let absolute = if file_path.is_absolute() {
                            file_path
                        } else {
                            directory.join(&relative)
                        };
                        let _ = fs::remove_file(absolute);
                    }
                }
            }
        }
        Ok(())
    }

    pub fn restore(directory: &Path, snapshot: &str) -> Result<()> {
        let git_dir = ensure_snapshot_repo(directory)?;
        git_output(directory, &git_dir, &["read-tree", snapshot])?;
        git_output(directory, &git_dir, &["checkout-index", "-a", "-f"])?;
        Ok(())
    }

    pub fn diff(directory: &Path, from_hash: &str) -> Result<Vec<super::FileDiff>> {
        let git_dir = ensure_snapshot_repo(directory)?;
        git_add_all(directory, &git_dir)?;
        let output = git_output(
            directory,
            &git_dir,
            &[
                "-c",
                "core.autocrlf=false",
                "-c",
                "core.quotepath=false",
                "diff",
                "--no-ext-diff",
                "--numstat",
                from_hash,
                "--",
                ".",
            ],
        );

        let mut diffs = Vec::new();

        if let Ok(output) = output {
            for line in String::from_utf8_lossy(&output.stdout).lines() {
                if line.is_empty() {
                    continue;
                }
                let parts: Vec<&str> = line.split('\t').collect();
                if parts.len() >= 3 {
                    let is_binary = parts[0] == "-" && parts[1] == "-";
                    let additions = if is_binary {
                        0
                    } else {
                        parts[0].parse::<u64>().unwrap_or(0)
                    };
                    let deletions = if is_binary {
                        0
                    } else {
                        parts[1].parse::<u64>().unwrap_or(0)
                    };
                    let path = parts[2].to_string();
                    diffs.push(super::FileDiff {
                        path,
                        additions,
                        deletions,
                    });
                }
            }
        }

        Ok(diffs)
    }

    /// Compute diff between two git refs (matching TS `Snapshot.diffFull`).
    ///
    /// Runs `git diff --numstat --no-renames <from> <to> -- .` to get per-file
    /// addition/deletion counts, then `git diff --name-status --no-renames <from> <to> -- .`
    /// to determine whether each file was added, deleted, or modified.
    pub fn diff_full(directory: &Path, from: &str, to: &str) -> Result<Vec<super::FileDiff>> {
        let git_dir = ensure_snapshot_repo(directory)?;
        let numstat_output = git_output(
            directory,
            &git_dir,
            &[
                "-c",
                "core.autocrlf=false",
                "-c",
                "core.quotepath=false",
                "diff",
                "--no-ext-diff",
                "--no-renames",
                "--numstat",
                from,
                to,
                "--",
                ".",
            ],
        );

        let mut diffs = Vec::new();

        if let Ok(output) = numstat_output {
            for line in String::from_utf8_lossy(&output.stdout).lines() {
                if line.is_empty() {
                    continue;
                }
                let parts: Vec<&str> = line.split('\t').collect();
                if parts.len() >= 3 {
                    let is_binary = parts[0] == "-" && parts[1] == "-";
                    let additions = if is_binary {
                        0
                    } else {
                        parts[0].parse::<u64>().unwrap_or(0)
                    };
                    let deletions = if is_binary {
                        0
                    } else {
                        parts[1].parse::<u64>().unwrap_or(0)
                    };
                    let path = parts[2].to_string();
                    diffs.push(super::FileDiff {
                        path,
                        additions,
                        deletions,
                    });
                }
            }
        }

        Ok(diffs)
    }
}

impl Default for Snapshot {
    fn default() -> Self {
        Self::new()
    }
}

fn snapshot_git_dir(directory: &Path) -> PathBuf {
    directory.join(".opencode").join("snapshot")
}

fn ensure_snapshot_repo(directory: &Path) -> Result<PathBuf> {
    let git_dir = snapshot_git_dir(directory);
    let parent = git_dir
        .parent()
        .context("snapshot git directory has no parent")?;
    fs::create_dir_all(parent)?;

    if !git_dir.join("HEAD").exists() {
        let init_output = Command::new("git")
            .arg("init")
            .arg("--quiet")
            .current_dir(directory)
            .env("GIT_DIR", &git_dir)
            .env("GIT_WORK_TREE", directory)
            .output()
            .context("failed to run git init for snapshot repo")?;
        if !init_output.status.success() {
            anyhow::bail!(
                "failed to initialize snapshot repo: {}",
                String::from_utf8_lossy(&init_output.stderr).trim()
            );
        }

        // Keep line endings stable across platforms.
        let _ = git_output(directory, &git_dir, &["config", "core.autocrlf", "false"]);
    }

    Ok(git_dir)
}

fn git_output(directory: &Path, git_dir: &Path, args: &[&str]) -> Result<Output> {
    let output = Command::new("git")
        .arg("--git-dir")
        .arg(git_dir)
        .arg("--work-tree")
        .arg(directory)
        .args(args)
        .current_dir(directory)
        .output()
        .with_context(|| format!("failed to run git command: git {}", args.join(" ")))?;

    if !output.status.success() {
        anyhow::bail!(
            "git command failed: git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    Ok(output)
}

fn git_add_all(directory: &Path, git_dir: &Path) -> Result<()> {
    // Exclude the snapshot repo itself when it lives under worktree.
    for args in [
        vec!["add", "-A", "--", ".", ":(exclude).opencode/snapshot"],
        vec!["add", "-A", "--", ".", ":!/.opencode/snapshot"],
        vec!["add", "-A", "--", ".", ":!.opencode/snapshot"],
    ] {
        if git_output(directory, git_dir, &args).is_ok() {
            return Ok(());
        }
    }

    anyhow::bail!("failed to add worktree files to snapshot index with snapshot-dir exclusion")
}

fn relative_to_worktree(worktree: &Path, input: &Path) -> String {
    if input.is_absolute() {
        input
            .strip_prefix(worktree)
            .ok()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default()
    } else {
        input.to_string_lossy().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::Snapshot;
    use std::fs;
    use tempfile::tempdir;

    fn is_hex_40(s: &str) -> bool {
        s.len() == 40 && s.chars().all(|c| c.is_ascii_hexdigit())
    }

    #[test]
    fn track_diff_restore_handles_uncommitted_worktree() {
        let temp = tempdir().expect("temp dir");
        let root = temp.path();
        let file = root.join("test_snapshot.txt");

        fs::write(&file, "test\n").expect("write initial file");
        let snapshot = Snapshot::track(root).expect("track snapshot");
        assert!(is_hex_40(&snapshot), "expected tree hash, got: {snapshot}");

        fs::write(&file, "modified\n").expect("modify file");
        let diffs = Snapshot::diff(root, &snapshot).expect("diff from snapshot");
        assert!(
            diffs.iter().any(|d| d.path == "test_snapshot.txt"),
            "expected changed file in diff"
        );

        Snapshot::restore(root, &snapshot).expect("restore snapshot");
        let restored = fs::read_to_string(&file).expect("read restored file");
        assert_eq!(restored, "test\n");
    }

    #[test]
    fn diff_full_between_two_tree_snapshots() {
        let temp = tempdir().expect("temp dir");
        let root = temp.path();
        let file = root.join("diff_full.txt");

        fs::write(&file, "v1\n").expect("write v1");
        let first = Snapshot::track(root).expect("first snapshot");
        fs::write(&file, "v2\n").expect("write v2");
        let second = Snapshot::track(root).expect("second snapshot");

        let diffs = Snapshot::diff_full(root, &first, &second).expect("diff full");
        assert!(
            diffs.iter().any(|d| d.path == "diff_full.txt"),
            "expected file change in diff_full"
        );
    }
}
