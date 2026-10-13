use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use tracing::warn;

pub const AGENTS_MD: &str = "AGENTS.md";
pub const CLAUDE_MD: &str = "CLAUDE.md";
pub const CONTEXT_MD: &str = "CONTEXT.md"; // deprecated
pub const CURSOR_MD: &str = ".cursorrules";
pub const COPILOT_MD: &str = ".github/copilot-instructions.md";

/// Well-known instruction file names searched in project directories.
const PROJECT_FILES: &[&str] = &[AGENTS_MD, CLAUDE_MD, CONTEXT_MD];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstructionFile {
    pub path: String,
    pub content: String,
    pub source: InstructionSource,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum InstructionSource {
    AgentsMd,
    ClaudeMd,
    ContextMd,
    CursorRules,
    CopilotInstructions,
    Global(String),
    ConfigInstruction(String),
    Url(String),
    Custom(String),
}

// ---------------------------------------------------------------------------
// Environment helpers
// ---------------------------------------------------------------------------

fn env_truthy(name: &str) -> bool {
    match std::env::var(name) {
        Ok(v) => matches!(v.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"),
        Err(_) => false,
    }
}

fn is_project_config_disabled() -> bool {
    env_truthy("OPENCODE_DISABLE_PROJECT_CONFIG")
}
fn opencode_config_dir_env() -> Option<String> {
    std::env::var("OPENCODE_CONFIG_DIR").ok()
}

// ---------------------------------------------------------------------------
// Path helpers
// ---------------------------------------------------------------------------

/// Return the XDG config directory for opencode (e.g. `~/.config/opencode`).
fn global_config_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("opencode"))
}

/// Build the list of global instruction file paths to probe.
fn global_files() -> Vec<PathBuf> {
    let mut files = Vec::new();

    // OPENCODE_CONFIG_DIR override comes first
    if let Some(dir) = opencode_config_dir_env() {
        files.push(PathBuf::from(&dir).join(AGENTS_MD));
    }

    // ~/.config/opencode/AGENTS.md
    if let Some(cfg) = global_config_dir() {
        files.push(cfg.join(AGENTS_MD));
    }

    // ~/.claude/CLAUDE.md
    if !env_truthy("OPENCODE_DISABLE_CLAUDE_CODE_PROMPT") {
        if let Some(home) = dirs::home_dir() {
            files.push(home.join(".claude").join(CLAUDE_MD));
        }
    }

    files
}

/// Walk from `start` up to `stop` (inclusive), collecting every existing
/// occurrence of `target` (a relative path like "AGENTS.md").
fn find_up(target: &str, start: &Path, stop: &Path) -> Vec<PathBuf> {
    let mut current = normalize(start);
    let stop = normalize(stop);
    let mut result = Vec::new();
    loop {
        let candidate = current.join(target);
        if candidate.exists() {
            result.push(candidate);
        }
        if current == stop {
            break;
        }
        match current.parent() {
            Some(p) if p != current => current = p.to_path_buf(),
            _ => break,
        }
    }
    result
}
/// Detect the worktree root by walking up from `start` looking for `.git`.
/// Returns the directory containing `.git`, or the filesystem root.
fn detect_worktree_root(start: &Path) -> PathBuf {
    let mut current = normalize(start);
    loop {
        if current.join(".git").exists() {
            return current;
        }
        match current.parent() {
            Some(p) if p != current => current = p.to_path_buf(),
            _ => return current,
        }
    }
}

fn normalize(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Expand a glob pattern rooted at `cwd`, returning absolute paths.
fn expand_glob(pattern: &str, cwd: &Path) -> Vec<PathBuf> {
    let full = if Path::new(pattern).is_absolute() {
        pattern.to_string()
    } else {
        cwd.join(pattern).to_string_lossy().to_string()
    };
    match glob::glob(&full) {
        Ok(paths) => paths
            .filter_map(|entry| entry.ok())
            .filter(|p| p.is_file())
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// Walk up from `start` to `stop`, expanding a glob pattern at each level.
fn glob_up(pattern: &str, start: &Path, stop: &Path) -> Vec<PathBuf> {
    let mut current = normalize(start);
    let stop = normalize(stop);
    let mut result = Vec::new();
    loop {
        result.extend(expand_glob(pattern, &current));
        if current == stop {
            break;
        }
        match current.parent() {
            Some(p) if p != current => current = p.to_path_buf(),
            _ => break,
        }
    }
    result
}

fn is_glob_pattern(s: &str) -> bool {
    s.contains('*') || s.contains('?')
}

fn is_url(s: &str) -> bool {
    s.starts_with("http://") || s.starts_with("https://")
}

fn instruction_source_from_filename(name: &str) -> InstructionSource {
    match name {
        AGENTS_MD => InstructionSource::AgentsMd,
        CLAUDE_MD => InstructionSource::ClaudeMd,
        CONTEXT_MD => InstructionSource::ContextMd,
        CURSOR_MD => InstructionSource::CursorRules,
        COPILOT_MD => InstructionSource::CopilotInstructions,
        other => InstructionSource::Custom(other.to_string()),
    }
}

fn first_matching_file_in_dir(dir: &Path, candidates: &[&str]) -> Option<PathBuf> {
    for file in candidates {
        let candidate = dir.join(file);
        if candidate.exists() && candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn resolve_for_file_candidates(
    file_path: &Path,
    project_root: &Path,
    candidates: &[&str],
) -> Vec<InstructionFile> {
    let target = normalize(file_path);
    let root = normalize(project_root);
    let mut current = target.parent().unwrap_or(&target).to_path_buf();
    let mut seen = HashSet::new();
    let mut results = Vec::new();

    while current.starts_with(&root) {
        if let Some(found) = first_matching_file_in_dir(&current, candidates) {
            let canonical = normalize(&found);
            let key = canonical.to_string_lossy().to_string();
            if canonical != target && seen.insert(key.clone()) {
                if let Ok(content) = std::fs::read_to_string(&found) {
                    if !content.trim().is_empty() {
                        let file_name = found
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or_default();
                        results.push(InstructionFile {
                            path: key,
                            content,
                            source: instruction_source_from_filename(file_name),
                        });
                    }
                }
            }
        }

        if current == root {
            break;
        }
        match current.parent() {
            Some(parent) if parent != current => current = parent.to_path_buf(),
            _ => break,
        }
    }

    results
}

/// Resolve instruction files relevant to a concrete file path.
///
/// Mirrors TS `InstructionPrompt.resolve(...)` behavior at a filesystem level:
/// while walking from the file's directory up to `project_root`, collect the
/// first matching instruction file in each directory (`AGENTS.md`, `CLAUDE.md`,
/// `CONTEXT.md`), skipping duplicates and the target file itself.
pub fn resolve_for_file(file_path: &Path, project_root: &Path) -> Vec<InstructionFile> {
    resolve_for_file_candidates(file_path, project_root, PROJECT_FILES)
}

/// Resolve per-file instruction context specifically from `AGENTS.md`.
///
/// This is used by file-read flows to mimic TS behavior where reading a file
/// should consult nearby AGENTS instructions in the same directory hierarchy.
pub fn resolve_agents_for_file(file_path: &Path, project_root: &Path) -> Vec<InstructionFile> {
    resolve_for_file_candidates(file_path, project_root, &[AGENTS_MD])
}

// ---------------------------------------------------------------------------
// InstructionLoader
// ---------------------------------------------------------------------------

pub struct InstructionLoader {
    /// Tracks already-loaded file paths and URLs for deduplication.
    loaded: HashSet<String>,
}

impl InstructionLoader {
    pub fn new() -> Self {
        Self {
            loaded: HashSet::new(),
        }
    }

    /// Load all instructions: global files, project files, and config entries.
    ///
    /// * `project_dir` - the project root directory (cwd)
    /// * `config_instructions` - entries from `config.instructions` (file paths,
    ///   glob patterns, or URLs)
    pub async fn load_all(
        &mut self,
        project_dir: &Path,
        config_instructions: &[String],
    ) -> Vec<InstructionFile> {
        let mut result = Vec::new();

        // 1. Project-level instruction files (findUp from cwd)
        if !is_project_config_disabled() {
            result.extend(self.load_project_instructions(project_dir));
        }

        // 2. Global instruction files
        result.extend(self.load_global_instructions());

        // 3. Config-specified instructions (files, globs, URLs)
        result.extend(
            self.load_config_instructions(project_dir, config_instructions)
                .await,
        );

        result
    }
    /// Load project-level instruction files by walking up from `project_dir`
    /// to the worktree root, looking for well-known files.
    ///
    /// Matches TS `systemPaths()`: for each file in PROJECT_FILES, findUp from
    /// project_dir to worktree root. If any matches are found for a file name,
    /// add them and stop (don't check the next file name).
    fn load_project_instructions(&mut self, project_dir: &Path) -> Vec<InstructionFile> {
        let worktree = detect_worktree_root(project_dir);
        let mut result = Vec::new();

        for &file in PROJECT_FILES {
            let matches = find_up(file, project_dir, &worktree);
            if !matches.is_empty() {
                for path in &matches {
                    let key = normalize(path).to_string_lossy().to_string();
                    if self.loaded.contains(&key) {
                        continue;
                    }
                    if let Ok(content) = std::fs::read_to_string(path) {
                        self.loaded.insert(key.clone());
                        let source = instruction_source_from_filename(file);
                        result.push(InstructionFile {
                            path: key,
                            content,
                            source,
                        });
                    }
                }
                break; // TS parity: stop after first file name that has matches
            }
        }

        result
    }
    /// Load global instruction files (first existing file wins).
    fn load_global_instructions(&mut self) -> Vec<InstructionFile> {
        let mut result = Vec::new();
        for path in global_files() {
            let key = normalize(&path).to_string_lossy().to_string();
            if self.loaded.contains(&key) {
                continue;
            }
            if path.exists() {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    self.loaded.insert(key.clone());
                    result.push(InstructionFile {
                        path: key.clone(),
                        content,
                        source: InstructionSource::Global(key),
                    });
                }
                break; // TS parity: stop after first existing global file
            }
        }
        result
    }

    /// Load instructions specified in config (file paths, globs, URLs).
    async fn load_config_instructions(
        &mut self,
        project_dir: &Path,
        instructions: &[String],
    ) -> Vec<InstructionFile> {
        let mut result = Vec::new();
        let worktree = detect_worktree_root(project_dir);

        for instruction in instructions {
            if is_url(instruction) {
                // URLs are handled separately below
                continue;
            }

            let resolved = self.resolve_config_path(instruction, project_dir, &worktree);
            for path in resolved {
                let key = normalize(&path).to_string_lossy().to_string();
                if self.loaded.contains(&key) {
                    continue;
                }
                if let Ok(content) = std::fs::read_to_string(&path) {
                    self.loaded.insert(key.clone());
                    result.push(InstructionFile {
                        path: key.clone(),
                        content,
                        source: InstructionSource::ConfigInstruction(instruction.clone()),
                    });
                }
            }
        }

        // Fetch URLs concurrently
        let urls: Vec<&String> = instructions.iter().filter(|i| is_url(i)).collect();
        for url in urls {
            if self.loaded.contains(url.as_str()) {
                continue;
            }
            match fetch_url(url).await {
                Ok(content) if !content.is_empty() => {
                    self.loaded.insert(url.clone());
                    result.push(InstructionFile {
                        path: url.clone(),
                        content,
                        source: InstructionSource::Url(url.clone()),
                    });
                }
                Ok(_) => {} // empty response, skip
                Err(e) => {
                    warn!("Failed to fetch instruction URL {}: {}", url, e);
                }
            }
        }

        result
    }
    /// Resolve a single config instruction entry to concrete file paths.
    fn resolve_config_path(
        &self,
        instruction: &str,
        project_dir: &Path,
        worktree: &Path,
    ) -> Vec<PathBuf> {
        let mut expanded = instruction.to_string();

        // Expand ~/
        if expanded.starts_with("~/") {
            if let Some(home) = dirs::home_dir() {
                expanded = home.join(&expanded[2..]).to_string_lossy().to_string();
            }
        }

        let path = Path::new(&expanded);

        if path.is_absolute() {
            if is_glob_pattern(&expanded) {
                expand_glob(&expanded, Path::new("/"))
            } else if path.is_file() {
                vec![path.to_path_buf()]
            } else {
                Vec::new()
            }
        } else if is_glob_pattern(&expanded) {
            // Relative glob: walk up from project_dir to worktree
            if !is_project_config_disabled() {
                glob_up(&expanded, project_dir, worktree)
            } else if let Some(dir) = opencode_config_dir_env() {
                glob_up(&expanded, Path::new(&dir), Path::new(&dir))
            } else {
                warn!(
                    "Skipping relative instruction \"{}\" - no OPENCODE_CONFIG_DIR set while project config is disabled",
                    instruction
                );
                Vec::new()
            }
        } else {
            // Relative plain path: walk up from project_dir to worktree
            if !is_project_config_disabled() {
                find_up(&expanded, project_dir, worktree)
            } else if let Some(dir) = opencode_config_dir_env() {
                find_up(&expanded, Path::new(&dir), Path::new(&dir))
            } else {
                warn!(
                    "Skipping relative instruction \"{}\" - no OPENCODE_CONFIG_DIR set while project config is disabled",
                    instruction
                );
                Vec::new()
            }
        }
    }
    // -----------------------------------------------------------------------
    // Backward-compatible helpers (kept from original API)
    // -----------------------------------------------------------------------

    /// Load instruction files from a single directory (original API).
    pub fn load_from_directory(dir: &Path) -> Vec<InstructionFile> {
        let mut instructions = Vec::new();

        if let Ok(content) = std::fs::read_to_string(dir.join(AGENTS_MD)) {
            instructions.push(InstructionFile {
                path: AGENTS_MD.to_string(),
                content,
                source: InstructionSource::AgentsMd,
            });
        }

        if let Ok(content) = std::fs::read_to_string(dir.join(CLAUDE_MD)) {
            instructions.push(InstructionFile {
                path: CLAUDE_MD.to_string(),
                content,
                source: InstructionSource::ClaudeMd,
            });
        }

        if let Ok(content) = std::fs::read_to_string(dir.join(CURSOR_MD)) {
            instructions.push(InstructionFile {
                path: CURSOR_MD.to_string(),
                content,
                source: InstructionSource::CursorRules,
            });
        }

        let copilot_path = dir.join(".github").join("copilot-instructions.md");
        if let Ok(content) = std::fs::read_to_string(&copilot_path) {
            instructions.push(InstructionFile {
                path: COPILOT_MD.to_string(),
                content,
                source: InstructionSource::CopilotInstructions,
            });
        }

        instructions
    }

    pub fn merge_instructions(instructions: &[InstructionFile]) -> String {
        instructions
            .iter()
            .map(|i| format!("Instructions from: {}\n{}", i.path, i.content))
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    pub fn find_instruction_files(dir: &Path) -> Vec<PathBuf> {
        let mut files = Vec::new();
        let candidates = [AGENTS_MD, CLAUDE_MD, CURSOR_MD];
        for name in candidates {
            let path = dir.join(name);
            if path.exists() {
                files.push(path);
            }
        }
        let copilot_path = dir.join(".github").join("copilot-instructions.md");
        if copilot_path.exists() {
            files.push(copilot_path);
        }
        files
    }

    /// Return the set of already-loaded paths/URLs.
    pub fn loaded_set(&self) -> &HashSet<String> {
        &self.loaded
    }

    /// Mark a path/URL as already loaded (for external dedup, e.g. tool reads).
    pub fn mark_loaded(&mut self, key: &str) {
        self.loaded.insert(key.to_string());
    }
}

impl Default for InstructionLoader {
    fn default() -> Self {
        Self::new()
    }
}
// ---------------------------------------------------------------------------
// HTTP fetching
// ---------------------------------------------------------------------------

async fn fetch_url(url: &str) -> Result<String, reqwest::Error> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()?;
    let resp = client.get(url).send().await?;
    if resp.status().is_success() {
        resp.text().await
    } else {
        Ok(String::new())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_load_from_directory_finds_agents_md() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join(AGENTS_MD), "agent instructions").unwrap();
        let files = InstructionLoader::load_from_directory(tmp.path());
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].content, "agent instructions");
    }

    #[test]
    fn test_load_from_directory_finds_multiple() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join(AGENTS_MD), "agents").unwrap();
        fs::write(tmp.path().join(CLAUDE_MD), "claude").unwrap();
        fs::write(tmp.path().join(CURSOR_MD), "cursor").unwrap();
        fs::create_dir_all(tmp.path().join(".github")).unwrap();
        fs::write(
            tmp.path().join(".github/copilot-instructions.md"),
            "copilot",
        )
        .unwrap();
        let files = InstructionLoader::load_from_directory(tmp.path());
        assert_eq!(files.len(), 4);
    }
    #[test]
    fn test_find_up_walks_parents() {
        let tmp = TempDir::new().unwrap();
        let child = tmp.path().join("a/b/c");
        fs::create_dir_all(&child).unwrap();
        fs::write(tmp.path().join(AGENTS_MD), "root agents").unwrap();
        fs::write(tmp.path().join("a").join(AGENTS_MD), "a agents").unwrap();

        // On macOS TempDir yields /var/... while find_up canonicalises to
        // /private/var/... (the same directory). Compare against the canonical
        // form or the assertion fails on a symlinked path, not on real behaviour.
        let root = tmp.path().canonicalize().unwrap();
        let found = find_up(AGENTS_MD, &child, &root);
        assert_eq!(found.len(), 2);
        // Closest first (child -> parent order)
        assert!(found[0].starts_with(root.join("a")));
        assert!(found[1].starts_with(&root));
    }

    #[test]
    fn test_find_up_stops_at_stop_dir() {
        let tmp = TempDir::new().unwrap();
        let stop = tmp.path().canonicalize().unwrap().join("repo");
        let child = stop.join("sub");
        fs::create_dir_all(&child).unwrap();
        fs::write(tmp.path().join(AGENTS_MD), "outside").unwrap();
        fs::write(stop.join(AGENTS_MD), "repo").unwrap();

        let found = find_up(AGENTS_MD, &child, &stop);
        assert_eq!(found.len(), 1);
        assert!(found[0].starts_with(&stop));
    }

    #[test]
    fn test_detect_worktree_root() {
        let tmp = TempDir::new().unwrap();
        let repo = tmp.path().join("repo");
        let child = repo.join("sub/deep");
        fs::create_dir_all(&child).unwrap();
        fs::create_dir_all(repo.join(".git")).unwrap();

        let root = detect_worktree_root(&child);
        assert_eq!(normalize(&root), normalize(&repo));
    }

    #[test]
    fn test_deduplication() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join(AGENTS_MD), "agents").unwrap();

        let mut loader = InstructionLoader::new();
        let files1 = loader.load_project_instructions(tmp.path());
        let files2 = loader.load_project_instructions(tmp.path());
        assert_eq!(files1.len(), 1);
        assert_eq!(files2.len(), 0); // deduped
    }

    #[test]
    fn test_merge_instructions_format() {
        let files = vec![
            InstructionFile {
                path: "/a.md".to_string(),
                content: "content a".to_string(),
                source: InstructionSource::AgentsMd,
            },
            InstructionFile {
                path: "/b.md".to_string(),
                content: "content b".to_string(),
                source: InstructionSource::ClaudeMd,
            },
        ];
        let merged = InstructionLoader::merge_instructions(&files);
        assert!(merged.contains("Instructions from: /a.md"));
        assert!(merged.contains("content a"));
        assert!(merged.contains("Instructions from: /b.md"));
        assert!(merged.contains("content b"));
    }

    #[test]
    fn test_glob_expansion() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("foo.md"), "foo").unwrap();
        fs::write(tmp.path().join("bar.md"), "bar").unwrap();
        fs::write(tmp.path().join("baz.txt"), "baz").unwrap();

        let matches = expand_glob("*.md", tmp.path());
        assert_eq!(matches.len(), 2);
    }

    #[test]
    fn test_project_files_first_match_wins() {
        // If AGENTS.md exists, CLAUDE.md and CONTEXT.md should not be loaded
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join(AGENTS_MD), "agents").unwrap();
        fs::write(tmp.path().join(CLAUDE_MD), "claude").unwrap();

        let mut loader = InstructionLoader::new();
        let files = loader.load_project_instructions(tmp.path());
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].content, "agents");
    }

    #[test]
    fn test_resolve_for_file_finds_same_directory_agents() {
        let tmp = TempDir::new().unwrap();
        let project_root = tmp.path().join("repo");
        let src_dir = project_root.join("src");
        fs::create_dir_all(&src_dir).unwrap();
        fs::write(src_dir.join(AGENTS_MD), "follow src rules").unwrap();
        let file_path = src_dir.join("main.rs");
        fs::write(&file_path, "fn main() {}").unwrap();

        let instructions = resolve_for_file(&file_path, &project_root);
        assert_eq!(instructions.len(), 1);
        assert!(instructions[0].content.contains("follow src rules"));
    }

    #[test]
    fn test_resolve_for_file_walks_up_to_project_root() {
        let tmp = TempDir::new().unwrap();
        let project_root = tmp.path().join("repo");
        let nested_dir = project_root.join("src/deep");
        fs::create_dir_all(&nested_dir).unwrap();
        fs::write(project_root.join(AGENTS_MD), "root rules").unwrap();
        fs::write(project_root.join("src").join(CLAUDE_MD), "src rules").unwrap();
        let file_path = nested_dir.join("lib.rs");
        fs::write(&file_path, "pub fn ok() {}").unwrap();

        let instructions = resolve_for_file(&file_path, &project_root);
        assert_eq!(instructions.len(), 2);
        assert!(instructions[0].content.contains("src rules"));
        assert!(instructions[1].content.contains("root rules"));
    }

    #[test]
    fn test_resolve_for_file_skips_target_instruction_file_itself() {
        let tmp = TempDir::new().unwrap();
        let project_root = tmp.path().join("repo");
        let nested_dir = project_root.join("nested");
        fs::create_dir_all(&nested_dir).unwrap();
        fs::write(project_root.join(AGENTS_MD), "root rules").unwrap();
        let target = nested_dir.join(AGENTS_MD);
        fs::write(&target, "should be skipped").unwrap();

        let instructions = resolve_for_file(&target, &project_root);
        assert_eq!(instructions.len(), 1);
        assert!(instructions[0].content.contains("root rules"));
        assert!(!instructions[0].content.contains("should be skipped"));
    }

    #[test]
    fn test_resolve_agents_for_file_ignores_non_agents_candidates() {
        let tmp = TempDir::new().unwrap();
        let project_root = tmp.path().join("repo");
        let nested_dir = project_root.join("src/deep");
        fs::create_dir_all(&nested_dir).unwrap();
        fs::write(project_root.join(AGENTS_MD), "root agents").unwrap();
        fs::write(project_root.join("src").join(CLAUDE_MD), "src claude").unwrap();
        let file_path = nested_dir.join("lib.rs");
        fs::write(&file_path, "pub fn ok() {}").unwrap();

        let instructions = resolve_agents_for_file(&file_path, &project_root);
        assert_eq!(instructions.len(), 1);
        assert!(instructions[0].content.contains("root agents"));
        assert!(!instructions[0].content.contains("src claude"));
    }

    #[test]
    fn test_resolve_agents_for_file_walks_up_agents_chain() {
        let tmp = TempDir::new().unwrap();
        let project_root = tmp.path().join("repo");
        let nested_dir = project_root.join("src/deep");
        fs::create_dir_all(&nested_dir).unwrap();
        fs::write(project_root.join(AGENTS_MD), "root agents").unwrap();
        fs::write(project_root.join("src").join(AGENTS_MD), "src agents").unwrap();
        let file_path = nested_dir.join("lib.rs");
        fs::write(&file_path, "pub fn ok() {}").unwrap();

        let instructions = resolve_agents_for_file(&file_path, &project_root);
        assert_eq!(instructions.len(), 2);
        assert!(instructions[0].content.contains("src agents"));
        assert!(instructions[1].content.contains("root agents"));
    }

    #[tokio::test]
    async fn test_load_all_combines_sources() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join(AGENTS_MD), "project agents").unwrap();

        let mut loader = InstructionLoader::new();
        let files = loader.load_all(tmp.path(), &[]).await;
        // At minimum, the project AGENTS.md should be loaded
        assert!(!files.is_empty());
        assert!(files.iter().any(|f| f.content == "project agents"));
    }

    #[test]
    fn test_is_glob_pattern() {
        assert!(is_glob_pattern("*.md"));
        assert!(is_glob_pattern("docs/*.md"));
        assert!(is_glob_pattern("file?.txt"));
        assert!(!is_glob_pattern("file.txt"));
        assert!(!is_glob_pattern("/absolute/path.md"));
    }

    #[test]
    fn test_is_url() {
        assert!(is_url("https://example.com/instructions.md"));
        assert!(is_url("http://example.com/instructions.md"));
        assert!(!is_url("/path/to/file.md"));
        assert!(!is_url("relative/path.md"));
    }
}
