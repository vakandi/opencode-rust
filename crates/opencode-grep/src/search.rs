use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchResult {
    pub path: String,
    pub line_number: usize,
    pub lines: String,
    pub absolute_offset: usize,
    pub submatches: Vec<SubMatch>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubMatch {
    pub text: String,
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Stats {
    pub elapsed: String,
    pub searches: usize,
    pub bytes_searched: usize,
    pub matched_lines: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileSearchOptions {
    pub glob: Vec<String>,
    pub hidden: bool,
    pub follow: bool,
    pub max_depth: Option<usize>,
}

impl Default for FileSearchOptions {
    fn default() -> Self {
        Self {
            glob: vec![],
            hidden: true,
            follow: false,
            max_depth: None,
        }
    }
}

pub struct Ripgrep;

impl Ripgrep {
    pub fn search<P: AsRef<Path>>(
        path: P,
        pattern: &str,
    ) -> Result<Vec<MatchResult>, Box<dyn std::error::Error>> {
        Self::search_with_limit(path, pattern, usize::MAX)
    }

    pub fn search_with_limit<P: AsRef<Path>>(
        path: P,
        pattern: &str,
        limit: usize,
    ) -> Result<Vec<MatchResult>, Box<dyn std::error::Error>> {
        let regex = regex::Regex::new(pattern)?;
        let path = path.as_ref();
        let mut matches = Vec::new();

        if path.is_file() {
            search_file(path, &regex, &mut matches, limit)?;
        } else if path.is_dir() {
            let files = Self::files(path, FileSearchOptions::default())?;
            for file in files {
                if matches.len() >= limit {
                    break;
                }
                let _ = search_file(&file, &regex, &mut matches, limit);
            }
        }

        Ok(matches)
    }

    pub fn files<P: AsRef<Path>>(
        path: P,
        options: FileSearchOptions,
    ) -> Result<Vec<PathBuf>, io::Error> {
        let path = path.as_ref();
        if !path.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("No such directory: '{}'", path.display()),
            ));
        }

        let mut result = Vec::new();
        let mut walk = WalkDir::new(path);

        if let Some(depth) = options.max_depth {
            walk = walk.max_depth(depth);
        }

        for entry in walk.into_iter().filter_map(|e| e.ok()) {
            let entry_path = entry.path();

            if !entry.file_type().is_file() {
                continue;
            }

            let file_name = entry.file_name().to_string_lossy();
            if !options.hidden && file_name.starts_with('.') {
                continue;
            }

            if file_name == ".git" || entry_path.to_string_lossy().contains(".git/") {
                continue;
            }

            if !options.glob.is_empty() {
                let matches_glob = options.glob.iter().any(|g| {
                    if g.starts_with("!") {
                        return !glob_match::glob_match(&g[1..], entry_path);
                    }
                    glob_match::glob_match(g, entry_path)
                });
                if !matches_glob {
                    continue;
                }
            }

            result.push(entry_path.to_path_buf());
        }

        Ok(result)
    }

    pub fn tree<P: AsRef<Path>>(path: P, limit: Option<usize>) -> Result<String, io::Error> {
        let path = path.as_ref();
        let files = Self::files(path, FileSearchOptions::default())?;

        let mut root: BTreeMap<String, TreeNode> = BTreeMap::new();

        for file in &files {
            let rel_path = file.strip_prefix(path).unwrap_or(file);
            let rel_str = rel_path.to_string_lossy();

            if rel_str.contains(".opencode") {
                continue;
            }

            let parts: Vec<&str> = rel_str.split(std::path::MAIN_SEPARATOR).collect();
            if parts.len() < 2 {
                continue;
            }

            let mut current = &mut root;
            for part in parts.iter().take(parts.len() - 1) {
                let node = current.entry(part.to_string()).or_insert(TreeNode {
                    name: part.to_string(),
                    children: BTreeMap::new(),
                });
                current = &mut node.children;
            }
        }

        let total = count_nodes(&root);
        let limit = limit.unwrap_or(total);
        let mut lines: Vec<String> = Vec::new();
        let mut queue: Vec<(String, String)> = Vec::new();

        for (_, node) in root.iter() {
            let node_json = serde_json::to_string(node).unwrap_or_default();
            queue.push((node.name.clone(), node_json));
        }

        let mut used = 0;
        let mut i = 0;
        while i < queue.len() && used < limit {
            let (path_str, node_json) = &queue[i];
            let node: TreeNode = match serde_json::from_str(node_json) {
                Ok(n) => n,
                Err(_) => {
                    i += 1;
                    continue;
                }
            };
            lines.push(path_str.clone());
            used += 1;

            let children_to_add: Vec<(String, String)> = node
                .children
                .iter()
                .map(|(_, child)| {
                    let child_json = serde_json::to_string(child).unwrap_or_default();
                    (format!("{}/{}", path_str, child.name), child_json)
                })
                .collect();

            queue.extend(children_to_add);
            i += 1;
        }

        if total > used {
            lines.push(format!("[{} truncated]", total - used));
        }

        Ok(lines.join("\n"))
    }
}

fn search_file(
    path: &Path,
    regex: &regex::Regex,
    matches: &mut Vec<MatchResult>,
    limit: usize,
) -> Result<(), io::Error> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let path_str = path.to_string_lossy().to_string();
    let mut offset = 0;

    for (line_num, line_result) in reader.lines().enumerate() {
        if matches.len() >= limit {
            break;
        }

        let line = line_result?;
        let line_len = line.len() + 1;

        if regex.is_match(&line) {
            let mut submatches = Vec::new();
            for cap in regex.find_iter(&line) {
                submatches.push(SubMatch {
                    text: cap.as_str().to_string(),
                    start: cap.start(),
                    end: cap.end(),
                });
            }

            matches.push(MatchResult {
                path: path_str.clone(),
                line_number: line_num + 1,
                lines: line,
                absolute_offset: offset,
                submatches,
            });
        }

        offset += line_len;
    }

    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TreeNode {
    name: String,
    children: BTreeMap<String, TreeNode>,
}

fn count_nodes(node: &BTreeMap<String, TreeNode>) -> usize {
    let mut total = 0;
    for child in node.values() {
        total += 1 + count_nodes(&child.children);
    }
    total
}

mod glob_match {
    use std::path::Path;

    pub fn glob_match(pattern: &str, path: &Path) -> bool {
        let path_str = path.to_string_lossy();
        let pattern = pattern.trim_start_matches("./");

        if pattern.contains('*') {
            let regex = glob_to_regex(pattern);
            regex.is_match(&path_str)
        } else {
            path_str.ends_with(pattern) || path_str.contains(pattern)
        }
    }

    fn glob_to_regex(pattern: &str) -> regex::Regex {
        let mut regex_str = String::from("^.*");
        for c in pattern.chars() {
            match c {
                '*' => regex_str.push_str("[^/]*"),
                '?' => regex_str.push_str("[^/]"),
                '.' => regex_str.push_str("\\."),
                _ => regex_str.push(c),
            }
        }
        regex_str.push('$');
        regex::Regex::new(&regex_str).unwrap_or_else(|_| regex::Regex::new(".*").unwrap())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_search() {
        let result = Ripgrep::search(".", "fn main").unwrap();
        assert!(!result.is_empty());
    }

    #[test]
    fn test_search_with_limit() {
        let result = Ripgrep::search_with_limit(".", "fn", 5).unwrap();
        assert!(result.len() <= 5);
    }

    #[test]
    fn test_files() {
        let result = Ripgrep::files(".", FileSearchOptions::default()).unwrap();
        assert!(!result.is_empty());
    }

    #[test]
    fn test_tree() {
        let result = Ripgrep::tree(".", Some(10)).unwrap();
        assert!(!result.is_empty());
    }
}
