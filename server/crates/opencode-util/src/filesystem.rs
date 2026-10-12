use std::path::{Path, PathBuf};
use tokio::fs;
use walkdir::WalkDir;

pub struct Filesystem;

impl Filesystem {
    pub async fn exists<P: AsRef<Path>>(p: P) -> bool {
        fs::metadata(p).await.is_ok()
    }

    pub async fn is_dir<P: AsRef<Path>>(p: P) -> bool {
        match fs::metadata(p).await {
            Ok(m) => m.is_dir(),
            Err(_) => false,
        }
    }

    pub async fn is_file<P: AsRef<Path>>(p: P) -> bool {
        match fs::metadata(p).await {
            Ok(m) => m.is_file(),
            Err(_) => false,
        }
    }

    #[cfg(windows)]
    pub fn normalize_path(p: &str) -> std::io::Result<String> {
        use std::fs;
        fs::canonicalize(p).map(|p| p.to_string_lossy().to_string())
    }

    #[cfg(not(windows))]
    pub fn normalize_path(p: &str) -> String {
        p.to_string()
    }

    pub fn overlaps(a: &str, b: &str) -> bool {
        let path_a = Path::new(a);
        let path_b = Path::new(b);

        let rel_a = path_a.strip_prefix(path_b).unwrap_or(path_a);
        let rel_b = path_b.strip_prefix(path_a).unwrap_or(path_b);

        let rel_a_str = rel_a.to_string_lossy();
        let rel_b_str = rel_b.to_string_lossy();

        !rel_a_str.starts_with("..")
            || !rel_b_str.starts_with("..")
            || rel_a_str.is_empty()
            || rel_b_str.is_empty()
    }

    pub fn contains(parent: &str, child: &str) -> bool {
        let parent_path = Path::new(parent);
        let child_path = Path::new(child);

        child_path.strip_prefix(parent_path).is_ok()
    }

    pub async fn find_up<P: AsRef<Path>>(target: &str, start: P, stop: Option<P>) -> Vec<PathBuf> {
        let mut current = start.as_ref().to_path_buf();
        let stop = stop.map(|s| s.as_ref().to_path_buf());
        let mut result = Vec::new();

        loop {
            let search = current.join(target);
            if Self::exists(&search).await {
                result.push(search);
            }
            if stop.as_ref() == Some(&current) {
                break;
            }
            let parent = current.parent().map(|p| p.to_path_buf());
            if let Some(parent) = parent {
                if parent == current {
                    break;
                }
                current = parent;
            } else {
                break;
            }
        }
        result
    }

    pub async fn glob_up<P: AsRef<Path>>(pattern: &str, start: P, stop: Option<P>) -> Vec<PathBuf> {
        let mut current = start.as_ref().to_path_buf();
        let stop = stop.map(|s| s.as_ref().to_path_buf());
        let mut result = Vec::new();

        loop {
            let glob = match glob::Pattern::new(pattern) {
                Ok(g) => g,
                Err(_) => {
                    if let Some(ref s) = stop {
                        if &current == s {
                            break;
                        }
                    }
                    if let Some(parent) = current.parent() {
                        if parent == current {
                            break;
                        }
                        current = parent.to_path_buf();
                    }
                    continue;
                }
            };

            for entry in WalkDir::new(&current)
                .max_depth(1)
                .follow_links(true)
                .into_iter()
                .filter_map(|e| e.ok())
            {
                let path = entry.path();
                if path.is_file() {
                    if let Ok(rel) = path.strip_prefix(&current) {
                        if glob.matches(&rel.to_string_lossy()) {
                            result.push(path.to_path_buf());
                        }
                    }
                }
            }

            if stop.as_ref() == Some(&current) {
                break;
            }
            if let Some(parent) = current.parent() {
                if parent == current {
                    break;
                }
                current = parent.to_path_buf();
            } else {
                break;
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[tokio::test]
    async fn test_exists() {
        assert!(Filesystem::exists(".").await);
        assert!(!Filesystem::exists("/nonexistent_path_12345").await);
    }

    #[tokio::test]
    async fn test_is_dir() {
        assert!(Filesystem::is_dir(".").await);
        assert!(!Filesystem::is_dir("Cargo.toml").await);
    }

    #[tokio::test]
    async fn test_find_up() {
        let results = Filesystem::find_up("Cargo.toml", ".", None).await;
        assert!(!results.is_empty());
    }
}
