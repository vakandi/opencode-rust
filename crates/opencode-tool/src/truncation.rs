use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::fs;
use tokio::io::AsyncWriteExt;

pub const MAX_LINES: usize = 2000;
pub const MAX_BYTES: usize = 50 * 1024;
pub const CLEANUP_RETENTION_DAYS: u64 = 7;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TruncationResult {
    pub truncated: bool,
    pub original_lines: usize,
    pub original_bytes: usize,
    pub truncated_lines: usize,
    pub truncated_bytes: usize,
    pub saved_to: Option<PathBuf>,
}

pub struct Truncate;

impl Truncate {
    pub async fn output(content: &str, save_dir: &Path) -> std::io::Result<TruncationResult> {
        let original_bytes = content.len();
        let original_lines = content.lines().count();

        let needs_truncation = original_bytes > MAX_BYTES || original_lines > MAX_LINES;

        if !needs_truncation {
            return Ok(TruncationResult {
                truncated: false,
                original_lines,
                original_bytes,
                truncated_lines: original_lines,
                truncated_bytes: original_bytes,
                saved_to: None,
            });
        }

        let lines: Vec<&str> = content.lines().collect();
        let mut truncated_lines = lines.clone();

        if truncated_lines.len() > MAX_LINES {
            truncated_lines = truncated_lines.split_off(truncated_lines.len() - MAX_LINES);
        }

        let mut truncated_content = truncated_lines.join("\n");

        if truncated_content.len() > MAX_BYTES {
            let bytes_to_take = MAX_BYTES.saturating_sub(100);
            let start = truncated_content.len().saturating_sub(bytes_to_take);
            truncated_content = format!("... [truncated]\n{}", &truncated_content[start..]);
        }

        let truncated_bytes = truncated_content.len();
        let truncated_line_count = truncated_content.lines().count();

        let saved_to = Self::save_truncated(&truncated_content, save_dir).await?;

        Ok(TruncationResult {
            truncated: true,
            original_lines,
            original_bytes,
            truncated_lines: truncated_line_count,
            truncated_bytes,
            saved_to: Some(saved_to),
        })
    }

    async fn save_truncated(content: &str, save_dir: &Path) -> std::io::Result<PathBuf> {
        fs::create_dir_all(save_dir).await?;

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);

        let filename = format!("truncated_{}.txt", timestamp);
        let filepath = save_dir.join(filename);

        let mut file = fs::File::create(&filepath).await?;
        file.write_all(content.as_bytes()).await?;
        file.flush().await?;

        Ok(filepath)
    }

    pub async fn cleanup(save_dir: &Path) -> std::io::Result<usize> {
        if !save_dir.exists() {
            return Ok(0);
        }

        let mut entries = fs::read_dir(save_dir).await?;
        let mut deleted = 0;
        let cutoff = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() - CLEANUP_RETENTION_DAYS * 24 * 60 * 60)
            .unwrap_or(0);

        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();

            if path.extension().map(|e| e == "txt").unwrap_or(false) {
                if let Ok(metadata) = entry.metadata().await {
                    if let Ok(modified) = metadata.modified() {
                        if let Ok(modified_ts) = modified.duration_since(UNIX_EPOCH) {
                            if modified_ts.as_secs() < cutoff {
                                fs::remove_file(&path).await?;
                                deleted += 1;
                            }
                        }
                    }
                }
            }
        }

        Ok(deleted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_no_truncation_needed() {
        let content = "Hello World\n".repeat(10);
        let dir = tempdir().unwrap();

        let result = Truncate::output(&content, dir.path()).await.unwrap();

        assert!(!result.truncated);
        assert_eq!(result.original_lines, 10);
    }

    #[tokio::test]
    async fn test_truncation_by_lines() {
        let content = (0..3000)
            .map(|i| format!("Line {}", i))
            .collect::<Vec<_>>()
            .join("\n");
        let dir = tempdir().unwrap();

        let result = Truncate::output(&content, dir.path()).await.unwrap();

        assert!(result.truncated);
        assert!(result.truncated_lines <= MAX_LINES);
        assert!(result.saved_to.is_some());
    }
}
