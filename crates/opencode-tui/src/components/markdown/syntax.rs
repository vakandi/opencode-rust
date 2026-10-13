use once_cell::sync::Lazy;
use ratatui::{
    style::{Color, Modifier, Style},
    text::Span,
};
use syntect::{
    easy::HighlightLines,
    highlighting::{FontStyle, Theme, ThemeSet},
    parsing::{SyntaxReference, SyntaxSet},
    util::LinesWithEndings,
};

use super::code_block::CodeTheme;

static SYNTAX_SET: Lazy<SyntaxSet> = Lazy::new(SyntaxSet::load_defaults_newlines);
static THEME_SET: Lazy<ThemeSet> = Lazy::new(ThemeSet::load_defaults);

const DARK_THEME: &str = "base16-ocean.dark";
const LIGHT_THEME: &str = "base16-ocean.light";
const FALLBACK_THEME: &str = "InspiredGitHub";

pub fn highlight_code(
    code: &str,
    language: &str,
    code_theme: &CodeTheme,
) -> Option<Vec<Vec<Span<'static>>>> {
    let syntax = find_syntax(language)?;
    let theme = select_theme(code_theme)?;
    let mut highlighter = HighlightLines::new(syntax, theme);

    let mut output = Vec::new();
    for line in LinesWithEndings::from(code) {
        let ranges = highlighter.highlight_line(line, &SYNTAX_SET).ok()?;
        let mut spans = Vec::with_capacity(ranges.len());
        for (style, segment) in ranges {
            let segment = strip_line_endings(segment);
            if segment.is_empty() {
                continue;
            }
            spans.push(Span::styled(
                segment.to_string(),
                syntect_style_to_ratatui(style),
            ));
        }
        if spans.is_empty() {
            spans.push(Span::styled(
                String::new(),
                Style::default().fg(code_theme.text),
            ));
        }
        output.push(spans);
    }

    Some(output)
}

fn find_syntax(language: &str) -> Option<&'static SyntaxReference> {
    let trimmed = language.trim();
    if trimmed.is_empty() {
        return None;
    }

    let token = normalize_language_token(trimmed);
    SYNTAX_SET
        .find_syntax_by_token(&token)
        .or_else(|| SYNTAX_SET.find_syntax_by_extension(&token))
        .or_else(|| SYNTAX_SET.find_syntax_by_name(trimmed))
}

fn normalize_language_token(language: &str) -> String {
    match language.trim().to_ascii_lowercase().as_str() {
        "rs" => "rust".to_string(),
        "py" => "python".to_string(),
        "js" => "javascript".to_string(),
        "ts" => "typescript".to_string(),
        "yml" => "yaml".to_string(),
        "sh" | "shell" | "zsh" => "bash".to_string(),
        other => other.to_string(),
    }
}

fn select_theme(code_theme: &CodeTheme) -> Option<&'static Theme> {
    let preferred = if color_luminance(code_theme.text) >= 160 {
        DARK_THEME
    } else {
        LIGHT_THEME
    };
    THEME_SET
        .themes
        .get(preferred)
        .or_else(|| THEME_SET.themes.get(FALLBACK_THEME))
        .or_else(|| THEME_SET.themes.values().next())
}

fn strip_line_endings(segment: &str) -> &str {
    let no_newline = segment.strip_suffix('\n').unwrap_or(segment);
    no_newline.strip_suffix('\r').unwrap_or(no_newline)
}

fn syntect_style_to_ratatui(style: syntect::highlighting::Style) -> Style {
    let mut out = Style::default().fg(Color::Rgb(
        style.foreground.r,
        style.foreground.g,
        style.foreground.b,
    ));

    if style.font_style.contains(FontStyle::BOLD) {
        out = out.add_modifier(Modifier::BOLD);
    }
    if style.font_style.contains(FontStyle::ITALIC) {
        out = out.add_modifier(Modifier::ITALIC);
    }
    if style.font_style.contains(FontStyle::UNDERLINE) {
        out = out.add_modifier(Modifier::UNDERLINED);
    }

    out
}

fn color_luminance(color: Color) -> u8 {
    match color {
        Color::Black => 0,
        Color::DarkGray => 85,
        Color::Gray => 170,
        Color::White => 255,
        Color::Red | Color::LightRed => 120,
        Color::Green | Color::LightGreen => 140,
        Color::Yellow | Color::LightYellow => 200,
        Color::Blue | Color::LightBlue => 100,
        Color::Magenta | Color::LightMagenta => 130,
        Color::Cyan | Color::LightCyan => 160,
        Color::Rgb(r, g, b) => {
            let lum = 0.2126_f32 * r as f32 + 0.7152_f32 * g as f32 + 0.0722_f32 * b as f32;
            lum.round() as u8
        }
        Color::Indexed(_) | Color::Reset => 128,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn highlight_code_returns_none_for_unknown_language() {
        let theme = CodeTheme::default();
        let output = highlight_code("some text", "unknown_language_token_xyz", &theme);
        assert!(output.is_none());
    }

    #[test]
    fn highlight_code_rust_returns_spans() {
        let theme = CodeTheme::default();
        let output =
            highlight_code("fn main() {}\n", "rust", &theme).expect("rust syntax should exist");
        assert!(!output.is_empty());
        assert!(output.iter().any(|line| !line.is_empty()));
    }
}
