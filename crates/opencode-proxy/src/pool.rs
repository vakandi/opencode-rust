//! Pool file: parse, and write the `|last:` stamp back.
//!
//! Format, one entry per line:
//!   `host:port:user:pass |last:YYYY-MM-DD HH:MM:SS |dur:1h 5m`
//!
//! Three defects in the reference are fixed here rather than ported:
//!
//! 1. **Timezone.** The reference writes `toISOString()` (UTC) and parses back with
//!    `new Date("YYYY-MM-DD HH:MM:SS")`, which JS reads as LOCAL time. On this machine
//!    that is a 60-minute skew, which silently reorders the LRU queue. Here the stamp
//!    is parsed and written as explicit UTC.
//! 2. **`|dur:` clobber.** The reference hardcodes `|dur:0h 0m` on every write, so the
//!    duration is destroyed after one rotation. It is preserved verbatim here.
//! 3. **Non-atomic write.** The reference calls `fs.writeFileSync` directly; a crash
//!    mid-write truncates the whole pool. Writes go to a temp file, are fsynced, and
//!    are `os.replace`d into position.
//!
//! The pool file lives inside this repo, never at
//! `~/.config/opencode/plugins/proxies.txt`: that file carries the live fleet's
//! rotation history and overwriting it resets every other opencode instance.

use std::collections::HashSet;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use chrono::{NaiveDateTime, TimeZone as _, Utc};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PoolEntry {
    pub index: usize,
    pub host: String,
    pub port: u16,
    pub user: String,
    /// May contain `:`; the reference reads it that way and then wrote it back
    /// truncated, losing anything after the fourth colon.
    pub pass: String,
    /// Epoch SECONDS, UTC. `None` means never used, which sorts FIRST.
    pub last: Option<i64>,
    pub dur: Option<String>,
    pub trailing: Vec<String>,
    pub raw_line: String,
}

impl PoolEntry {
    pub fn key(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }

    pub fn upstream(&self) -> crate::Upstream {
        crate::Upstream::new(self.host.clone(), self.port, self.user.clone(), self.pass.clone())
    }

    /// Epoch seconds, `0` when never used. Sorting by this puts never-used entries
    /// ahead of everything, which is what makes a fresh pool rotate evenly.
    pub fn ts(&self) -> i64 {
        self.last.unwrap_or(0)
    }
}

pub fn parse_stamp(raw: &str) -> Option<i64> {
    let replaced = raw.trim().replace("last:", "");
    let trimmed = replaced.trim();
    if trimmed.is_empty() {
        return None;
    }
    NaiveDateTime::parse_from_str(&trimmed, "%Y-%m-%d %H:%M:%S")
        .ok()
        .and_then(|naive| Some(Utc.from_utc_datetime(&naive).timestamp()))
}

pub fn format_stamp(epoch_secs: i64) -> String {
    Utc.timestamp_opt(epoch_secs, 0)
        .single()
        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_default()
}

pub fn parse(text: &str) -> Vec<PoolEntry> {
    let mut out = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let cells: Vec<&str> = trimmed.split(" |").collect();
        let parts: Vec<&str> = cells[0].trim().split(':').collect();
        if parts.len() < 2 {
            tracing::debug!(line = index + 1, "skipping malformed pool line");
            continue;
        }
        let port = match parts[1].parse::<u16>() {
            Ok(p) if p > 0 => p,
            _ => {
                tracing::debug!(line = index + 1, "skipping pool line with bad port");
                continue;
            }
        };
        if parts[0].is_empty() {
            continue;
        }
        // Dispatch metadata cells by KEY, not position. Positional parsing assumed
        // `last` is always cells[1] and `dur` always cells[2], so a line carrying
        // `|dur:` without `|last:` lost its duration on both read and write.
        let mut last = None;
        let mut dur = None;
        let mut trailing = Vec::new();
        for cell in cells.iter().skip(1) {
            let cell = cell.trim();
            match cell.split_once(':') {
                Some(("last", value)) => last = parse_stamp(value),
                Some(("dur", _)) => dur = Some(cell.to_string()),
                _ => trailing.push(cell.to_string()),
            }
        }
        out.push(PoolEntry {
            index,
            host: parts[0].to_string(),
            port,
            user: parts.get(2).map(|s| s.to_string()).unwrap_or_default(),
            pass: if parts.len() > 3 { parts[3..].join(":") } else { String::new() },
            last,
            dur,
            trailing,
            raw_line: line.to_string(),
        });
    }
    out
}

pub fn load(path: &Path) -> Vec<PoolEntry> {
    match fs::read_to_string(path) {
        Ok(text) => parse(&text),
        Err(error) => {
            tracing::warn!(%error, path = %path.display(), "proxy pool unreadable");
            Vec::new()
        }
    }
}

/// Rewrite only the `|last:` cell of the matching line. Every other byte of every
/// other line is re-emitted verbatim, so a pool carrying fields this version does
/// not understand survives a rotation untouched.
pub fn stamp(path: &Path, host: &str, port: u16, epoch_secs: i64) -> std::io::Result<bool> {
    let text = fs::read_to_string(path)?;
    let mut matched = false;
    let stamp_cell = format!("|last:{}", format_stamp(epoch_secs));

    let mut out = String::with_capacity(text.len() + stamp_cell.len());
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            out.push('\n');
            continue;
        }
        let cells: Vec<&str> = trimmed.split(" |").collect();
        let parts: Vec<&str> = cells[0].trim().split(':').collect();
        let same = parts.len() >= 2 && parts[0] == host && parts[1].parse::<u16>() == Ok(port);
        if !same {
            out.push_str(line);
            out.push('\n');
            continue;
        }
        matched = true;
        out.push_str(cells[0].trim());
        // `stamp_cell` already carries its leading `|`, so this is the single space
        // that separates it from the credential cell. Adding " |" here emitted
        // " ||last:", which is not the pool format and desyncs it from the reference.
        out.push(' ');
        out.push_str(&stamp_cell);
        // Re-emit every other metadata cell verbatim, wherever it sat. Only an
        // existing `last:` is dropped, because this call is replacing it.
        for cell in cells.iter().skip(1) {
            let cell = cell.trim();
            if cell.starts_with("last:") {
                continue;
            }
            out.push_str(" |");
            out.push_str(cell);
        }
        out.push('\n');
    }

    // Nothing matched: leave the file completely alone rather than rewriting it.
    if !matched {
        return Ok(false);
    }
    write_atomic(path, out.as_bytes())?;
    Ok(true)
}

pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension(format!("tmp.{}", std::process::id()));
    {
        let mut file = fs::File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    if let Some(parent) = path.parent() {
        if let Ok(dir) = fs::File::open(parent) {
            let _ = dir.sync_all();
        }
    }
    Ok(())
}

/// Repo-local pool path. Never the live fleet's file.
pub fn pool_file() -> PathBuf {
    opencode_paths::data_file(|| std::env::temp_dir(), "proxy-pool.txt")
}

pub fn quarantine_file() -> PathBuf {
    opencode_paths::data_file(|| std::env::temp_dir(), "proxy-quarantine.json")
}

pub fn claims_file() -> PathBuf {
    opencode_paths::data_file(|| std::env::temp_dir(), "proxy-claims.json")
}

/// Host keys currently in the pool, used for quarantine garbage collection.
pub fn pool_keys(entries: &[PoolEntry]) -> HashSet<String> {
    entries.iter().map(|e| e.key()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_full_line_including_duration() {
        let entries = parse("1.2.3.4:8080:user:pass |last:2026-10-07 15:51:30 |dur:1h 5m\n");
        assert_eq!(entries.len(), 1);
        let e = &entries[0];
        assert_eq!(e.host, "1.2.3.4");
        assert_eq!(e.port, 8080);
        assert_eq!(e.user, "user");
        assert_eq!(e.pass, "pass");
        assert_eq!(e.dur.as_deref(), Some("dur:1h 5m"));
    }

    #[test]
    fn password_may_contain_colons() {
        let entries = parse("1.2.3.4:8080:user:pa:ss:word\n");
        assert_eq!(entries[0].pass, "pa:ss:word");
    }

    #[test]
    fn never_used_entries_have_no_stamp_and_sort_first() {
        let entries = parse("a:1:u:p\nb:2:u:p |last:2020-01-01 00:00:00\n");
        let mut by_ts = entries.clone();
        by_ts.sort_by_key(|e| e.ts());
        assert_eq!(by_ts[0].host, "a");
        assert_eq!(by_ts[0].ts(), 0);
    }

    #[test]
    fn stamp_is_utc_regardless_of_local_timezone() {
        let stamp = "2026-10-07 15:51:30";
        let expected = NaiveDateTime::parse_from_str(stamp, "%Y-%m-%d %H:%M:%S").unwrap().and_utc().timestamp();
        assert_eq!(parse_stamp(stamp), Some(expected));
        assert_eq!(format_stamp(expected), stamp);
    }

    #[test]
    fn stamp_emits_exactly_one_pipe_separator() {
        // Regression: stamp() once pushed " |" and then a cell already beginning with
        // "|", producing " ||last:..." — not the pool format. A substring assertion
        // (line.contains("|last:")) passes on that corruption, so the whole line is
        // compared here instead.
        let dir = std::env::temp_dir().join(format!("pool-stamp-{}", std::process::id()));
        let path = dir.join("proxies.txt");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&path, "1.2.3.4:8080:user:pass\n").unwrap();

        assert!(stamp(&path, "1.2.3.4", 8080, 1_700_000_000).unwrap());

        let written = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            written.trim_end(),
            format!("1.2.3.4:8080:user:pass |last:{}", format_stamp(1_700_000_000)),
            "stamped line must match the reference pool format byte for byte"
        );
        assert!(!written.contains("||"), "double pipe means a broken pool file");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn stamp_replaces_an_existing_last_and_keeps_dur() {
        let dir = std::env::temp_dir().join(format!("pool-stamp2-{}", std::process::id()));
        let path = dir.join("proxies.txt");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            &path,
            "1.2.3.4:8080:u:p |last:2020-01-01 00:00:00 |dur:3h 7m\n",
        )
        .unwrap();

        assert!(stamp(&path, "1.2.3.4", 8080, 1_700_000_000).unwrap());

        let written = std::fs::read_to_string(&path).unwrap();
        assert!(written.contains("|dur:3h 7m"), "|dur: must survive: {written}");
        assert!(!written.contains("2020-01-01"), "old |last: must be replaced: {written}");
        assert!(!written.contains("||"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn stamp_leaves_the_file_untouched_when_the_host_is_absent() {
        let dir = std::env::temp_dir().join(format!("pool-stamp3-{}", std::process::id()));
        let path = dir.join("proxies.txt");
        std::fs::create_dir_all(&dir).unwrap();
        let original = "1.2.3.4:8080:u:p |last:2020-01-01 00:00:00\n";
        std::fs::write(&path, original).unwrap();

        assert!(!stamp(&path, "9.9.9.9", 1, 1_700_000_000).unwrap());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn stamped_lines_reparse_identically() {
        // The round trip that actually matters: write, stamp, re-parse. A malformed
        // separator shows up here as a lost or shifted field.
        let dir = std::env::temp_dir().join(format!("pool-stamp4-{}", std::process::id()));
        let path = dir.join("proxies.txt");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&path, "1.2.3.4:8080:user:pass\n5.6.7.8:3128:a:b |dur:9m\n").unwrap();

        stamp(&path, "1.2.3.4", 8080, 1_700_000_000).unwrap();
        let reloaded = parse(&std::fs::read_to_string(&path).unwrap());

        assert_eq!(reloaded.len(), 2);
        assert_eq!(reloaded[0].user, "user");
        assert_eq!(reloaded[0].pass, "pass");
        assert_eq!(reloaded[0].last, Some(1_700_000_000));
        assert_eq!(reloaded[1].dur.as_deref(), Some("dur:9m"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn malformed_lines_are_skipped_without_losing_good_ones() {
        let entries = parse("garbage\n1.2.3.4:notaport:u:p\ngood:1:u:p\n\n");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].host, "good");
    }
}