use std::path::Path;
use std::process::Command;

use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum WorktreeError {
    #[error("Not a git repository")]
    NotGitRepo,
    #[error("Git command failed: {0}")]
    GitError(String),
    #[error("Worktree not found: {0}")]
    NotFound(String),
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorktreeInfo {
    pub path: String,
    pub branch: String,
    pub head: String,
}

fn run_git(args: &[&str], cwd: &Path) -> Result<String, WorktreeError> {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .map_err(|e| WorktreeError::GitError(e.to_string()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(WorktreeError::GitError(stderr.to_string()));
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn is_git_repo(path: &Path) -> bool {
    path.join(".git").exists() || run_git(&["rev-parse", "--git-dir"], path).is_ok()
}

pub fn list_worktrees(repo_path: &Path) -> Result<Vec<WorktreeInfo>, WorktreeError> {
    if !is_git_repo(repo_path) {
        return Err(WorktreeError::NotGitRepo);
    }

    let output = run_git(&["worktree", "list", "--porcelain"], repo_path)?;

    let mut worktrees = Vec::new();
    let mut current_path: Option<String> = None;
    let mut current_branch: Option<String> = None;
    let mut current_head: Option<String> = None;

    for line in output.lines() {
        if line.starts_with("worktree ") {
            if let (Some(path), Some(branch), Some(head)) =
                (&current_path, &current_branch, &current_head)
            {
                worktrees.push(WorktreeInfo {
                    path: path.clone(),
                    branch: branch.clone(),
                    head: head.clone(),
                });
            }
            current_path = Some(line["worktree ".len()..].to_string());
            current_branch = None;
            current_head = None;
        } else if line.starts_with("HEAD ") {
            current_head = Some(line["HEAD ".len()..].to_string());
        } else if line.starts_with("branch ") {
            let branch_full = &line["branch ".len()..];
            let branch = branch_full
                .strip_prefix("refs/heads/")
                .unwrap_or(branch_full);
            current_branch = Some(branch.to_string());
        }
    }

    if let (Some(path), Some(branch), Some(head)) = (&current_path, &current_branch, &current_head)
    {
        worktrees.push(WorktreeInfo {
            path: path.clone(),
            branch: branch.clone(),
            head: head.clone(),
        });
    } else if let (Some(path), Some(head)) = (&current_path, &current_head) {
        worktrees.push(WorktreeInfo {
            path: path.clone(),
            branch: "HEAD".to_string(),
            head: head.clone(),
        });
    }

    Ok(worktrees)
}

pub fn create_worktree(
    repo_path: &Path,
    branch: Option<&str>,
    target_path: Option<&str>,
) -> Result<WorktreeInfo, WorktreeError> {
    if !is_git_repo(repo_path) {
        return Err(WorktreeError::NotGitRepo);
    }

    let default_branch_name = format!("worktree-{}", chrono::Utc::now().format("%Y%m%d-%H%M%S"));
    let branch_name = branch.unwrap_or(&default_branch_name);

    let repo_name = repo_path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "repo".to_string());
    let default_worktree_path = format!("{}-{}", repo_name, branch_name);

    let worktree_path = target_path
        .map(|s| s.to_string())
        .unwrap_or(default_worktree_path);

    let full_path = if Path::new(&worktree_path).is_absolute() {
        worktree_path.clone()
    } else {
        repo_path
            .parent()
            .map(|p| p.join(&worktree_path))
            .unwrap_or_else(|| repo_path.join(&worktree_path))
            .to_string_lossy()
            .to_string()
    };

    let branch_exists = run_git(&["branch", "--list", branch_name], repo_path)
        .map(|s| !s.is_empty())
        .unwrap_or(false);

    if branch_exists {
        run_git(
            &[
                "worktree",
                "add",
                "-b",
                branch_name,
                &full_path,
                branch_name,
            ],
            repo_path,
        )?;
    } else {
        let default_branch = run_git(&["symbolic-ref", "--short", "HEAD"], repo_path)
            .unwrap_or_else(|_| "main".to_string());

        run_git(
            &[
                "worktree",
                "add",
                "-b",
                branch_name,
                &full_path,
                &default_branch,
            ],
            repo_path,
        )?;
    }

    let head = run_git(&["rev-parse", "HEAD"], Path::new(&full_path))?;

    Ok(WorktreeInfo {
        path: full_path,
        branch: branch_name.to_string(),
        head,
    })
}

pub fn remove_worktree(
    repo_path: &Path,
    worktree_path: &str,
    force: bool,
) -> Result<(), WorktreeError> {
    if !is_git_repo(repo_path) {
        return Err(WorktreeError::NotGitRepo);
    }

    let mut args = vec!["worktree", "remove", worktree_path];
    if force {
        args.push("--force");
    }

    run_git(&args, repo_path)?;
    Ok(())
}

pub fn prune_worktrees(repo_path: &Path) -> Result<(), WorktreeError> {
    if !is_git_repo(repo_path) {
        return Err(WorktreeError::NotGitRepo);
    }

    run_git(&["worktree", "prune"], repo_path)?;
    Ok(())
}
