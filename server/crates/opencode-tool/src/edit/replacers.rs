//! Advanced string matching and replacement strategies for the edit tool.
//!
//! Sources:
//! - https://github.com/cline/cline/blob/main/evals/diff-edits/diff-apply/diff-06-23-25.ts
//! - https://github.com/google-gemini/gemini-cli/blob/main/packages/core/src/utils/editCorrector.ts

use std::ops::Range;

const SINGLE_CANDIDATE_SIMILARITY_THRESHOLD: f64 = 0.0;
const MULTIPLE_CANDIDATES_SIMILARITY_THRESHOLD: f64 = 0.3;

fn levenshtein(a: &str, b: &str) -> usize {
    if a.is_empty() || b.is_empty() {
        return a.len().max(b.len());
    }

    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let a_len = a_chars.len();
    let b_len = b_chars.len();

    let mut matrix = vec![vec![0; b_len + 1]; a_len + 1];

    for i in 0..=a_len {
        matrix[i][0] = i;
    }
    for j in 0..=b_len {
        matrix[0][j] = j;
    }

    for i in 1..=a_len {
        for j in 1..=b_len {
            let cost = if a_chars[i - 1] == b_chars[j - 1] {
                0
            } else {
                1
            };
            matrix[i][j] = (matrix[i - 1][j] + 1)
                .min(matrix[i][j - 1] + 1)
                .min(matrix[i - 1][j - 1] + cost);
        }
    }

    matrix[a_len][b_len]
}

#[derive(Debug, Clone)]
pub struct Replacement {
    pub matched: String,
    pub range: Range<usize>,
}

pub trait Replacer: Send + Sync {
    fn name(&self) -> &str;
    fn find<'a>(&'a self, content: &'a str, find: &'a str)
        -> Box<dyn Iterator<Item = String> + 'a>;
}

pub struct SimpleReplacer;

impl Replacer for SimpleReplacer {
    fn name(&self) -> &str {
        "SimpleReplacer"
    }

    fn find<'a>(
        &'a self,
        _content: &'a str,
        find: &'a str,
    ) -> Box<dyn Iterator<Item = String> + 'a> {
        Box::new(std::iter::once(find.to_string()))
    }
}

pub struct LineTrimmedReplacer;

impl Replacer for LineTrimmedReplacer {
    fn name(&self) -> &str {
        "LineTrimmedReplacer"
    }

    fn find<'a>(
        &'a self,
        content: &'a str,
        find: &'a str,
    ) -> Box<dyn Iterator<Item = String> + 'a> {
        let mut results = Vec::new();
        let original_lines: Vec<&str> = content.split('\n').collect();
        let mut search_lines: Vec<&str> = find.split('\n').collect();

        if search_lines.last().map(|l| l.is_empty()) == Some(true) {
            search_lines.pop();
        }

        let search_len = search_lines.len();
        if search_len == 0 {
            return Box::new(results.into_iter());
        }

        for i in 0..=(original_lines.len().saturating_sub(search_len)) {
            let mut matches = true;
            for j in 0..search_len {
                if original_lines[i + j].trim() != search_lines[j].trim() {
                    matches = false;
                    break;
                }
            }

            if matches {
                let match_start_index: usize =
                    original_lines[..i].iter().map(|l| l.len() + 1).sum();
                let match_end_index = match_start_index
                    + original_lines[i..i + search_len]
                        .iter()
                        .enumerate()
                        .map(|(idx, l)| {
                            if idx < search_len - 1 {
                                l.len() + 1
                            } else {
                                l.len()
                            }
                        })
                        .sum::<usize>();
                results.push(content[match_start_index..match_end_index].to_string());
            }
        }

        Box::new(results.into_iter())
    }
}

pub struct BlockAnchorReplacer;

impl Replacer for BlockAnchorReplacer {
    fn name(&self) -> &str {
        "BlockAnchorReplacer"
    }

    fn find<'a>(
        &'a self,
        content: &'a str,
        find: &'a str,
    ) -> Box<dyn Iterator<Item = String> + 'a> {
        let mut results = Vec::new();
        let original_lines: Vec<&str> = content.split('\n').collect();
        let mut search_lines: Vec<&str> = find.split('\n').collect();

        if search_lines.len() < 3 {
            return Box::new(results.into_iter());
        }

        if search_lines.last().map(|l| l.is_empty()) == Some(true) {
            search_lines.pop();
        }

        let first_line_search = search_lines[0].trim();
        let last_line_search = search_lines[search_lines.len() - 1].trim();
        let search_block_size = search_lines.len();

        let mut candidates: Vec<(usize, usize)> = Vec::new();
        for i in 0..original_lines.len() {
            if original_lines[i].trim() != first_line_search {
                continue;
            }
            for j in (i + 2)..original_lines.len() {
                if original_lines[j].trim() == last_line_search {
                    candidates.push((i, j));
                    break;
                }
            }
        }

        if candidates.is_empty() {
            return Box::new(results.into_iter());
        }

        let line_start_positions: Vec<usize> = std::iter::once(0)
            .chain(original_lines.iter().scan(0, |pos, line| {
                *pos += line.len() + 1;
                Some(*pos)
            }))
            .collect();

        let extract_block = |start_line: usize, end_line: usize| -> String {
            let start_byte = line_start_positions[start_line];
            let end_byte = if end_line + 1 < line_start_positions.len() {
                line_start_positions[end_line + 1] - 1
            } else {
                content.len()
            };
            content[start_byte..end_byte].to_string()
        };

        if candidates.len() == 1 {
            let (start_line, end_line) = candidates[0];
            let actual_block_size = end_line - start_line + 1;
            let lines_to_check = (search_block_size - 2).min(actual_block_size - 2);

            let similarity = if lines_to_check > 0 {
                let mut sim = 0.0;
                for j in 1..(search_block_size - 1).min(actual_block_size - 1) {
                    let original_line = original_lines[start_line + j].trim();
                    let search_line = search_lines[j].trim();
                    let max_len = original_line.len().max(search_line.len());
                    if max_len == 0 {
                        continue;
                    }
                    let distance = levenshtein(original_line, search_line);
                    sim += (1.0 - distance as f64 / max_len as f64) / lines_to_check as f64;
                    if sim >= SINGLE_CANDIDATE_SIMILARITY_THRESHOLD {
                        break;
                    }
                }
                sim
            } else {
                1.0
            };

            if similarity >= SINGLE_CANDIDATE_SIMILARITY_THRESHOLD {
                results.push(extract_block(start_line, end_line));
            }
            return Box::new(results.into_iter());
        }

        let mut best_match: Option<(usize, usize)> = None;
        let mut max_similarity = -1.0;

        for (start_line, end_line) in candidates {
            let actual_block_size = end_line - start_line + 1;
            let lines_to_check = (search_block_size - 2).min(actual_block_size - 2);

            let similarity = if lines_to_check > 0 {
                let mut sim = 0.0;
                for j in 1..(search_block_size - 1).min(actual_block_size - 1) {
                    let original_line = original_lines[start_line + j].trim();
                    let search_line = search_lines[j].trim();
                    let max_len = original_line.len().max(search_line.len());
                    if max_len == 0 {
                        continue;
                    }
                    let distance = levenshtein(original_line, search_line);
                    sim += 1.0 - distance as f64 / max_len as f64;
                }
                sim / lines_to_check as f64
            } else {
                1.0
            };

            if similarity > max_similarity {
                max_similarity = similarity;
                best_match = Some((start_line, end_line));
            }
        }

        if max_similarity >= MULTIPLE_CANDIDATES_SIMILARITY_THRESHOLD {
            if let Some((start_line, end_line)) = best_match {
                results.push(extract_block(start_line, end_line));
            }
        }

        Box::new(results.into_iter())
    }
}

pub struct WhitespaceNormalizedReplacer;

impl Replacer for WhitespaceNormalizedReplacer {
    fn name(&self) -> &str {
        "WhitespaceNormalizedReplacer"
    }

    fn find<'a>(
        &'a self,
        content: &'a str,
        find: &'a str,
    ) -> Box<dyn Iterator<Item = String> + 'a> {
        let mut results = Vec::new();
        let normalize_whitespace =
            |text: &str| text.split_whitespace().collect::<Vec<_>>().join(" ");
        let normalized_find = normalize_whitespace(find);
        let lines: Vec<&str> = content.split('\n').collect();

        for line in &lines {
            if normalize_whitespace(line) == normalized_find {
                results.push(line.to_string());
            } else {
                let normalized_line = normalize_whitespace(line);
                if normalized_line.contains(&normalized_find) {
                    let words: Vec<&str> = find.split_whitespace().collect();
                    if !words.is_empty() {
                        let pattern = words
                            .iter()
                            .map(|w| regex::escape(w))
                            .collect::<Vec<_>>()
                            .join(r"\s+");
                        if let Ok(re) = regex::Regex::new(&pattern) {
                            if let Some(m) = re.find(line) {
                                results.push(m.as_str().to_string());
                            }
                        }
                    }
                }
            }
        }

        let find_lines: Vec<&str> = find.split('\n').collect();
        if find_lines.len() > 1 {
            for i in 0..=(lines.len().saturating_sub(find_lines.len())) {
                let block = lines[i..i + find_lines.len()].join("\n");
                if normalize_whitespace(&block) == normalized_find {
                    results.push(block);
                }
            }
        }

        Box::new(results.into_iter())
    }
}

pub struct IndentationFlexibleReplacer;

impl Replacer for IndentationFlexibleReplacer {
    fn name(&self) -> &str {
        "IndentationFlexibleReplacer"
    }

    fn find<'a>(
        &'a self,
        content: &'a str,
        find: &'a str,
    ) -> Box<dyn Iterator<Item = String> + 'a> {
        let mut results = Vec::new();

        let remove_indentation = |text: &str| -> String {
            let lines: Vec<&str> = text.split('\n').collect();
            let non_empty_lines: Vec<&str> = lines
                .iter()
                .filter(|l| !l.trim().is_empty())
                .copied()
                .collect();
            if non_empty_lines.is_empty() {
                return text.to_string();
            }
            let min_indent = non_empty_lines
                .iter()
                .map(|line| line.chars().take_while(|c| c.is_whitespace()).count())
                .min()
                .unwrap_or(0);
            lines
                .iter()
                .map(|line| {
                    if line.trim().is_empty() {
                        line.to_string()
                    } else {
                        line.chars().skip(min_indent).collect()
                    }
                })
                .collect::<Vec<_>>()
                .join("\n")
        };

        let normalized_find = remove_indentation(find);
        let content_lines: Vec<&str> = content.split('\n').collect();
        let find_lines: Vec<&str> = find.split('\n').collect();

        for i in 0..=(content_lines.len().saturating_sub(find_lines.len())) {
            let block = content_lines[i..i + find_lines.len()].join("\n");
            if remove_indentation(&block) == normalized_find {
                results.push(block);
            }
        }

        Box::new(results.into_iter())
    }
}

pub struct EscapeNormalizedReplacer;

impl Replacer for EscapeNormalizedReplacer {
    fn name(&self) -> &str {
        "EscapeNormalizedReplacer"
    }

    fn find<'a>(
        &'a self,
        content: &'a str,
        find: &'a str,
    ) -> Box<dyn Iterator<Item = String> + 'a> {
        let mut results = Vec::new();

        let unescape_string = |s: &str| -> String {
            let mut result = String::new();
            let chars: Vec<char> = s.chars().collect();
            let mut i = 0;
            while i < chars.len() {
                if chars[i] == '\\' && i + 1 < chars.len() {
                    match chars[i + 1] {
                        'n' => {
                            result.push('\n');
                            i += 2;
                        }
                        't' => {
                            result.push('\t');
                            i += 2;
                        }
                        'r' => {
                            result.push('\r');
                            i += 2;
                        }
                        '\'' => {
                            result.push('\'');
                            i += 2;
                        }
                        '"' => {
                            result.push('"');
                            i += 2;
                        }
                        '`' => {
                            result.push('`');
                            i += 2;
                        }
                        '\\' => {
                            result.push('\\');
                            i += 2;
                        }
                        '$' => {
                            result.push('$');
                            i += 2;
                        }
                        '\n' => {
                            result.push('\n');
                            i += 2;
                        }
                        _ => {
                            result.push(chars[i]);
                            i += 1;
                        }
                    }
                } else {
                    result.push(chars[i]);
                    i += 1;
                }
            }
            result
        };

        let unescaped_find = unescape_string(find);
        if content.contains(&unescaped_find) {
            results.push(unescaped_find.clone());
        }

        let lines: Vec<&str> = content.split('\n').collect();
        let find_lines: Vec<&str> = unescaped_find.split('\n').collect();

        if find_lines.len() > 1 {
            for i in 0..=(lines.len().saturating_sub(find_lines.len())) {
                let block = lines[i..i + find_lines.len()].join("\n");
                if unescape_string(&block) == unescaped_find {
                    results.push(block);
                }
            }
        }

        Box::new(results.into_iter())
    }
}

pub struct TrimmedBoundaryReplacer;

impl Replacer for TrimmedBoundaryReplacer {
    fn name(&self) -> &str {
        "TrimmedBoundaryReplacer"
    }

    fn find<'a>(
        &'a self,
        content: &'a str,
        find: &'a str,
    ) -> Box<dyn Iterator<Item = String> + 'a> {
        let mut results = Vec::new();
        let trimmed_find = find.trim();
        if trimmed_find == find {
            return Box::new(results.into_iter());
        }

        if content.contains(trimmed_find) {
            results.push(trimmed_find.to_string());
        }

        let lines: Vec<&str> = content.split('\n').collect();
        let find_lines: Vec<&str> = find.split('\n').collect();

        for i in 0..=(lines.len().saturating_sub(find_lines.len())) {
            let block = lines[i..i + find_lines.len()].join("\n");
            if block.trim() == trimmed_find {
                results.push(block);
            }
        }

        Box::new(results.into_iter())
    }
}

pub struct ContextAwareReplacer;

impl Replacer for ContextAwareReplacer {
    fn name(&self) -> &str {
        "ContextAwareReplacer"
    }

    fn find<'a>(
        &'a self,
        content: &'a str,
        find: &'a str,
    ) -> Box<dyn Iterator<Item = String> + 'a> {
        let mut results = Vec::new();
        let mut find_lines: Vec<&str> = find.split('\n').collect();
        if find_lines.len() < 3 {
            return Box::new(results.into_iter());
        }
        if find_lines.last().map(|l| l.is_empty()) == Some(true) {
            find_lines.pop();
        }

        let content_lines: Vec<&str> = content.split('\n').collect();
        let first_line = find_lines[0].trim();
        let last_line = find_lines[find_lines.len() - 1].trim();

        for i in 0..content_lines.len() {
            if content_lines[i].trim() != first_line {
                continue;
            }
            for j in (i + 2)..content_lines.len() {
                if content_lines[j].trim() == last_line {
                    let block_lines = &content_lines[i..=j];
                    let block = block_lines.join("\n");
                    if block_lines.len() == find_lines.len() {
                        let mut matching_lines = 0;
                        let mut total_non_empty_lines = 0;
                        for k in 1..(block_lines.len() - 1) {
                            let block_line = block_lines[k].trim();
                            let find_line = find_lines[k].trim();
                            if !block_line.is_empty() || !find_line.is_empty() {
                                total_non_empty_lines += 1;
                                if block_line == find_line {
                                    matching_lines += 1;
                                }
                            }
                        }
                        if total_non_empty_lines == 0
                            || matching_lines as f64 / total_non_empty_lines as f64 >= 0.5
                        {
                            results.push(block);
                            break;
                        }
                    }
                    break;
                }
            }
        }

        Box::new(results.into_iter())
    }
}

pub struct MultiOccurrenceReplacer;

impl Replacer for MultiOccurrenceReplacer {
    fn name(&self) -> &str {
        "MultiOccurrenceReplacer"
    }

    fn find<'a>(
        &'a self,
        content: &'a str,
        find: &'a str,
    ) -> Box<dyn Iterator<Item = String> + 'a> {
        let count = content.matches(find).count();
        Box::new(std::iter::repeat(find.to_string()).take(count))
    }
}

pub struct CompositeReplacer {
    replacers: Vec<Box<dyn Replacer>>,
}

impl Default for CompositeReplacer {
    fn default() -> Self {
        Self {
            replacers: vec![
                Box::new(SimpleReplacer),
                Box::new(LineTrimmedReplacer),
                Box::new(BlockAnchorReplacer),
                Box::new(WhitespaceNormalizedReplacer),
                Box::new(IndentationFlexibleReplacer),
                Box::new(EscapeNormalizedReplacer),
                Box::new(TrimmedBoundaryReplacer),
                Box::new(ContextAwareReplacer),
                Box::new(MultiOccurrenceReplacer),
            ],
        }
    }
}

impl CompositeReplacer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn replace(
        &self,
        content: &str,
        old_string: &str,
        new_string: &str,
        replace_all: bool,
    ) -> Result<String, String> {
        if old_string == new_string {
            return Err("No changes to apply: oldString and newString are identical.".to_string());
        }

        let mut not_found = true;

        for replacer in &self.replacers {
            for search in replacer.find(content, old_string) {
                if let Some(index) = content.find(&search) {
                    not_found = false;
                    if replace_all {
                        return Ok(content.replace(&search, new_string));
                    }
                    let last_index = content.rfind(&search);
                    if last_index != Some(index) {
                        continue;
                    }
                    let mut result =
                        String::with_capacity(content.len() - search.len() + new_string.len());
                    result.push_str(&content[..index]);
                    result.push_str(new_string);
                    result.push_str(&content[index + search.len()..]);
                    return Ok(result);
                }
            }
        }

        if not_found {
            Err("Could not find oldString in the file. It must match exactly, including whitespace, indentation, and line endings.".to_string())
        } else {
            Err("Found multiple matches for oldString. Provide more surrounding context to make the match unique.".to_string())
        }
    }
}

pub fn normalize_line_endings(text: &str) -> String {
    text.replace("\r\n", "\n")
}

pub fn trim_diff(diff: &str) -> String {
    let lines: Vec<&str> = diff.split('\n').collect();

    let content_lines: Vec<&&str> = lines
        .iter()
        .filter(|line| {
            let line = line.to_string();
            (line.starts_with('+') || line.starts_with('-') || line.starts_with(' '))
                && !line.starts_with("---")
                && !line.starts_with("+++")
        })
        .collect();

    if content_lines.is_empty() {
        return diff.to_string();
    }

    let min_indent = content_lines
        .iter()
        .filter_map(|line| {
            let content = &line[1..];
            if content.trim().is_empty() {
                return None;
            }
            let indent_len = content.chars().take_while(|c| c.is_whitespace()).count();
            Some(indent_len)
        })
        .min()
        .unwrap_or(0);

    if min_indent == 0 {
        return diff.to_string();
    }

    let trimmed_lines: Vec<String> = lines
        .iter()
        .map(|line| {
            if (line.starts_with('+') || line.starts_with('-') || line.starts_with(' '))
                && !line.starts_with("---")
                && !line.starts_with("+++")
            {
                let prefix = &line[0..1];
                let content = &line[1..];
                if content.len() >= min_indent {
                    format!("{}{}", prefix, &content[min_indent..])
                } else {
                    line.to_string()
                }
            } else {
                line.to_string()
            }
        })
        .collect();

    trimmed_lines.join("\n")
}

pub fn generate_unified_diff(file_path: &str, old_content: &str, new_content: &str) -> String {
    let old_lines: Vec<&str> = old_content.lines().collect();
    let new_lines: Vec<&str> = new_content.lines().collect();

    let mut result = String::new();
    result.push_str(&format!("--- {}\n", file_path));
    result.push_str(&format!("+++ {}\n", file_path));

    let old_len = old_lines.len();
    let new_len = new_lines.len();

    result.push_str(&format!("@@ -1,{} +1,{} @@\n", old_len, new_len));

    for line in &old_lines {
        result.push_str(&format!("-{}\n", line));
    }

    for line in &new_lines {
        result.push_str(&format!("+{}\n", line));
    }

    result
}

pub struct FileDiff {
    pub path: String,
    pub additions: usize,
    pub deletions: usize,
}

impl FileDiff {
    pub fn from_contents(old_content: &str, new_content: &str) -> Self {
        let old_lines: Vec<&str> = old_content.lines().collect();
        let new_lines: Vec<&str> = new_content.lines().collect();

        let additions = new_lines.len().saturating_sub(old_lines.len());
        let deletions = old_lines.len().saturating_sub(new_lines.len());

        Self {
            path: String::new(),
            additions,
            deletions,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_levenshtein() {
        assert_eq!(levenshtein("", ""), 0);
        assert_eq!(levenshtein("a", ""), 1);
        assert_eq!(levenshtein("", "abc"), 3);
        assert_eq!(levenshtein("kitten", "sitting"), 3);
        assert_eq!(levenshtein("hello", "hello"), 0);
    }

    #[test]
    fn test_simple_replacer() {
        let replacer = SimpleReplacer;
        let results: Vec<_> = replacer.find("Hello, World!", "World").collect();
        assert_eq!(results, vec!["World"]);
    }

    #[test]
    fn test_composite_replacer_exact_match() {
        let replacer = CompositeReplacer::new();
        assert_eq!(
            replacer.replace("Hello, World!", "World", "Rust", false),
            Ok("Hello, Rust!".to_string())
        );
    }

    #[test]
    fn test_composite_replacer_not_found() {
        let replacer = CompositeReplacer::new();
        assert!(replacer
            .replace("Hello, World!", "NotFound", "Rust", false)
            .is_err());
    }

    #[test]
    fn test_composite_replacer_multiple_matches() {
        let replacer = CompositeReplacer::new();
        assert!(replacer
            .replace("foo bar foo", "foo", "baz", false)
            .is_err());
    }

    #[test]
    fn test_composite_replacer_replace_all() {
        let replacer = CompositeReplacer::new();
        assert_eq!(
            replacer.replace("foo bar foo", "foo", "baz", true),
            Ok("baz bar baz".to_string())
        );
    }
}
