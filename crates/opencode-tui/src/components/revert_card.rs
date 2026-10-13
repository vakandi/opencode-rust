use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};
use similar::{ChangeTag, TextDiff};

use crate::components::DiffView;
use crate::context::RevertInfo;
use crate::theme::Theme;

const DIFF_PREVIEW_LINES: usize = 8;

pub fn render_revert_card(revert: &RevertInfo, theme: &Theme) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from(vec![
            Span::styled("│ ", Style::default().fg(theme.warning)),
            Span::styled(
                "⟲ Revert",
                Style::default()
                    .fg(theme.warning)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" pending", Style::default().fg(theme.text_muted)),
        ]),
        Line::from(vec![
            Span::styled("│ ", Style::default().fg(theme.warning)),
            Span::styled(
                format!("message {}", short_id(&revert.message_id)),
                Style::default().fg(theme.text_muted),
            ),
        ]),
    ];

    if let Some(part_id) = revert
        .part_id
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        lines.push(Line::from(vec![
            Span::styled("│ ", Style::default().fg(theme.warning)),
            Span::styled(
                format!("part {}", short_id(part_id)),
                Style::default().fg(theme.text_muted),
            ),
        ]));
    }

    if let Some(snapshot) = revert
        .snapshot
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        lines.push(Line::from(vec![
            Span::styled("│ ", Style::default().fg(theme.warning)),
            Span::styled(
                format!("snapshot {}", short_id(snapshot)),
                Style::default().fg(theme.text_muted),
            ),
        ]));
    }

    if let Some(diff_preview) = resolve_diff_preview(revert) {
        let (preview_title, diff) = match diff_preview {
            DiffPreview::Provided(diff) => ("diff preview", diff),
            DiffPreview::FromSnapshot(diff) => ("snapshot diff preview", diff),
        };
        lines.push(Line::from(vec![
            Span::styled("│ ", Style::default().fg(theme.warning)),
            Span::styled(preview_title, Style::default().fg(theme.text_muted)),
        ]));

        let diff_lines = DiffView::new().with_content(&diff).to_lines(theme);
        for line in diff_lines.iter().take(DIFF_PREVIEW_LINES) {
            let mut spans = vec![Span::styled("│ ", Style::default().fg(theme.warning))];
            spans.extend(line.spans.clone());
            lines.push(Line::from(spans));
        }
        if diff_lines.len() > DIFF_PREVIEW_LINES {
            lines.push(Line::from(vec![
                Span::styled("│ ", Style::default().fg(theme.warning)),
                Span::styled(
                    format!(
                        "… {} more diff lines",
                        diff_lines.len() - DIFF_PREVIEW_LINES
                    ),
                    Style::default().fg(theme.text_muted),
                ),
            ]));
        }
    } else {
        lines.push(Line::from(vec![
            Span::styled("│ ", Style::default().fg(theme.warning)),
            Span::styled(
                "no diff preview available",
                Style::default().fg(theme.text_muted),
            ),
        ]));
    }

    lines.push(Line::from(vec![
        Span::styled("│ ", Style::default().fg(theme.warning)),
        Span::styled("Use ", Style::default().fg(theme.text_muted)),
        Span::styled("/undo", Style::default().fg(theme.primary)),
        Span::styled(" or ", Style::default().fg(theme.text_muted)),
        Span::styled("/redo", Style::default().fg(theme.primary)),
    ]));

    lines
}

fn short_id(value: &str) -> String {
    let trimmed = value.trim();
    let count = trimmed.chars().count();
    if count <= 12 {
        return trimmed.to_string();
    }
    let prefix: String = trimmed.chars().take(6).collect();
    let suffix: String = trimmed
        .chars()
        .rev()
        .take(4)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    format!("{prefix}…{suffix}")
}

enum DiffPreview {
    Provided(String),
    FromSnapshot(String),
}

fn resolve_diff_preview(revert: &RevertInfo) -> Option<DiffPreview> {
    if let Some(diff) = normalized_non_empty(revert.diff.as_deref()) {
        return Some(DiffPreview::Provided(diff.to_string()));
    }
    let snapshot = normalized_non_empty(revert.snapshot.as_deref())?;
    build_snapshot_diff(snapshot).map(DiffPreview::FromSnapshot)
}

fn build_snapshot_diff(snapshot: &str) -> Option<String> {
    if looks_like_snapshot_hash(snapshot) {
        return None;
    }

    let diff = TextDiff::from_lines("", snapshot);
    let mut rendered = String::new();
    let mut inserted_lines = 0usize;

    for change in diff.iter_all_changes() {
        if change.tag() != ChangeTag::Insert {
            continue;
        }
        for segment in change.value().split_inclusive('\n') {
            let line = segment.strip_suffix('\n').unwrap_or(segment);
            rendered.push('+');
            rendered.push_str(line);
            rendered.push('\n');
            inserted_lines += 1;
        }
    }

    if inserted_lines == 0 {
        return None;
    }

    let mut out = format!("@@ -0,0 +1,{} @@\n", inserted_lines);
    out.push_str(&rendered);
    Some(out)
}

fn normalized_non_empty(value: Option<&str>) -> Option<&str> {
    value.and_then(|v| {
        let trimmed = v.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    })
}

fn looks_like_snapshot_hash(value: &str) -> bool {
    let bytes = value.as_bytes();
    (bytes.len() == 40 || bytes.len() == 64) && value.chars().all(|ch| ch.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_hash_does_not_generate_preview_diff() {
        let hash = "a8f95eb4955f9c26ef40f6117f68f4f552e6f0f6";
        assert!(build_snapshot_diff(hash).is_none());
    }

    #[test]
    fn snapshot_text_generates_insert_diff() {
        let diff = build_snapshot_diff("line one\nline two\n")
            .expect("text snapshot should produce synthetic diff");
        assert!(diff.contains("@@ -0,0 +1,2 @@"));
        assert!(diff.contains("+line one"));
        assert!(diff.contains("+line two"));
    }

    #[test]
    fn resolve_prefers_server_diff() {
        let revert = RevertInfo {
            message_id: "m1".to_string(),
            part_id: None,
            snapshot: Some("local snapshot".to_string()),
            diff: Some("@@ -1,0 +1,1 @@\n+server".to_string()),
        };
        match resolve_diff_preview(&revert) {
            Some(DiffPreview::Provided(diff)) => assert!(diff.contains("server")),
            _ => panic!("expected provided diff"),
        }
    }
}
