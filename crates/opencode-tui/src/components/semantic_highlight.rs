use ratatui::{
    style::{Color, Style},
    text::{Line, Span},
};

use crate::theme::Theme;

/// Apply semantic highlighting to rendered lines.
/// Recognizes: shell commands, file paths, error keywords, numeric stats.
pub fn highlight_lines(lines: Vec<Line<'static>>, theme: &Theme) -> Vec<Line<'static>> {
    lines
        .into_iter()
        .map(|line| highlight_line(line, theme))
        .collect()
}

fn highlight_line(line: Line<'static>, theme: &Theme) -> Line<'static> {
    let spans: Vec<Span<'static>> = line
        .spans
        .into_iter()
        .flat_map(|span| highlight_span(span, theme))
        .collect();
    Line::from(spans)
}

/// Returns true if this span's fg is a "base text" color that semantic
/// highlighting should still process. Markdown sets fg to theme.text on
/// plain text — we want to highlight those. But headings, code, links
/// etc. use different colors and should be left alone.
fn is_base_text_color(fg: Option<Color>, theme: &Theme) -> bool {
    match fg {
        None => true,
        Some(c) => c == theme.text,
    }
}

fn highlight_span(span: Span<'static>, theme: &Theme) -> Vec<Span<'static>> {
    let text = span.content.as_ref();
    // Only highlight spans with base text color (or no color).
    // Skip spans already styled by markdown (headings, code, links, etc.)
    if !is_base_text_color(span.style.fg, theme) {
        return vec![span];
    }

    let base_style = span.style;
    let mut result = Vec::new();
    let mut remaining = text;

    while !remaining.is_empty() {
        if let Some((before, matched, after, style)) =
            find_next_highlight(remaining, base_style, theme)
        {
            if !before.is_empty() {
                result.push(Span::styled(before.to_string(), base_style));
            }
            result.push(Span::styled(matched.to_string(), style));
            remaining = after;
        } else {
            result.push(Span::styled(remaining.to_string(), base_style));
            break;
        }
    }

    if result.is_empty() {
        vec![span]
    } else {
        result
    }
}

/// Find the next highlightable pattern in text.
/// Returns (before, matched, after, style) or None.
fn find_next_highlight<'a>(
    text: &'a str,
    base: Style,
    theme: &Theme,
) -> Option<(&'a str, &'a str, &'a str, Style)> {
    let mut best: Option<(usize, usize, Style)> = None;

    // Shell command: `$ command`
    if let Some(pos) = text.find("$ ") {
        let end = text[pos..]
            .find('\n')
            .map(|n| pos + n)
            .unwrap_or(text.len());
        let candidate = (pos, end, base.fg(theme.syntax_keyword));
        best = Some(pick_earliest(best, candidate));
    }

    // File paths: /path/to/file or src/path or ./path
    for prefix in &["/home/", "/tmp/", "/etc/", "src/", "./", "../"] {
        if let Some(pos) = text.find(prefix) {
            let end = text[pos..]
                .find(|c: char| c.is_whitespace() || c == ')' || c == ']' || c == '\'')
                .map(|n| pos + n)
                .unwrap_or(text.len());
            if end > pos + prefix.len() {
                let candidate = (pos, end, base.fg(theme.info));
                best = Some(pick_earliest(best, candidate));
            }
        }
    }

    // Error keywords
    for keyword in &[
        "error", "Error", "ERROR", "failed", "Failed", "FAILED", "denied", "Denied",
    ] {
        if let Some(pos) = text.find(keyword) {
            let end = pos + keyword.len();
            let candidate = (pos, end, base.fg(theme.error));
            best = Some(pick_earliest(best, candidate));
        }
    }

    // Numeric stats: "123 lines", "2.3s", "45%"
    let bytes = text.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        if b.is_ascii_digit() {
            let end = text[i..]
                .find(|c: char| {
                    !c.is_ascii_digit() && c != '.' && c != ',' && c != '%' && c != 's' && c != 'm'
                })
                .map(|n| i + n)
                .unwrap_or(text.len());
            let segment = &text[i..end];
            // Must contain at least a unit suffix or be a percentage/time
            if segment.ends_with('%')
                || segment.ends_with('s')
                || segment.ends_with("ms")
                || segment.contains(',')
            {
                let candidate = (i, end, base.fg(theme.syntax_number));
                best = Some(pick_earliest(best, candidate));
                break;
            }
        }
    }

    let (start, end, style) = best?;
    Some((&text[..start], &text[start..end], &text[end..], style))
}

fn pick_earliest(
    current: Option<(usize, usize, Style)>,
    candidate: (usize, usize, Style),
) -> (usize, usize, Style) {
    match current {
        Some(c) if c.0 <= candidate.0 => c,
        _ => candidate,
    }
}
