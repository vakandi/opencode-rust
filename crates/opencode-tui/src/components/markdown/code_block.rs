use ratatui::{
    style::{Color, Modifier, Style},
    text::Span,
};

use super::syntax;

#[derive(Clone, Debug)]
pub struct CodeBlock {
    pub language: Option<String>,
    pub code: String,
}

impl CodeBlock {
    pub fn new(language: Option<String>, code: String) -> Self {
        Self { language, code }
    }

    pub fn to_lines(&self, theme: &CodeTheme) -> Vec<Vec<Span<'static>>> {
        if let Some(lines) = syntax::highlight_code(
            &self.code,
            self.language.as_deref().unwrap_or_default(),
            theme,
        ) {
            return lines;
        }

        let mut lines = Vec::new();

        for line in self.code.lines() {
            let highlighted = self.highlight_line(line, theme);
            lines.push(highlighted);
        }

        lines
    }

    fn highlight_line(&self, line: &str, theme: &CodeTheme) -> Vec<Span<'static>> {
        let language = self.language.as_deref().unwrap_or("");

        match language {
            "rust" => self.highlight_rust(line, theme),
            "python" | "py" => self.highlight_python(line, theme),
            "javascript" | "js" | "typescript" | "ts" => self.highlight_js(line, theme),
            "json" => self.highlight_json(line, theme),
            "bash" | "sh" | "shell" => self.highlight_bash(line, theme),
            _ => vec![Span::styled(
                line.to_string(),
                Style::default().fg(theme.text),
            )],
        }
    }

    fn highlight_rust(&self, line: &str, theme: &CodeTheme) -> Vec<Span<'static>> {
        let keywords = [
            "fn", "let", "mut", "const", "static", "pub", "mod", "use", "crate", "self", "Self",
            "super", "struct", "enum", "impl", "trait", "type", "where", "for", "loop", "while",
            "if", "else", "match", "return", "break", "continue", "async", "await", "move", "ref",
            "as", "in",
        ];

        highlight_with_keywords(line, theme, &keywords)
    }

    fn highlight_python(&self, line: &str, theme: &CodeTheme) -> Vec<Span<'static>> {
        let keywords = [
            "def", "class", "if", "elif", "else", "for", "while", "try", "except", "finally",
            "with", "as", "import", "from", "return", "yield", "raise", "pass", "break",
            "continue", "lambda", "and", "or", "not", "in", "is", "True", "False", "None", "async",
            "await",
        ];

        highlight_with_keywords(line, theme, &keywords)
    }

    fn highlight_js(&self, line: &str, theme: &CodeTheme) -> Vec<Span<'static>> {
        let keywords = [
            "function",
            "const",
            "let",
            "var",
            "if",
            "else",
            "for",
            "while",
            "return",
            "class",
            "extends",
            "import",
            "export",
            "from",
            "default",
            "async",
            "await",
            "try",
            "catch",
            "finally",
            "throw",
            "new",
            "this",
            "true",
            "false",
            "null",
            "undefined",
            "typeof",
            "instanceof",
        ];

        highlight_with_keywords(line, theme, &keywords)
    }

    fn highlight_json(&self, line: &str, theme: &CodeTheme) -> Vec<Span<'static>> {
        let mut spans = Vec::new();
        let mut current = String::new();
        let mut in_string = false;
        let chars: Vec<char> = line.chars().collect();
        let mut i = 0;

        while i < chars.len() {
            let ch = chars[i];

            if ch == '"' && (i == 0 || chars[i - 1] != '\\') {
                if !current.is_empty() && !in_string {
                    let trimmed = current.trim();
                    if trimmed == ":"
                        || trimmed == "{"
                        || trimmed == "}"
                        || trimmed == "["
                        || trimmed == "]"
                        || trimmed == ","
                    {
                        spans.push(Span::styled(
                            current.clone(),
                            Style::default().fg(theme.punctuation),
                        ));
                    } else if trimmed == "true" || trimmed == "false" || trimmed == "null" {
                        spans.push(Span::styled(
                            current.clone(),
                            Style::default().fg(theme.keyword),
                        ));
                    } else if trimmed.parse::<f64>().is_ok() {
                        spans.push(Span::styled(
                            current.clone(),
                            Style::default().fg(theme.number),
                        ));
                    } else {
                        spans.push(Span::styled(
                            current.clone(),
                            Style::default().fg(theme.text),
                        ));
                    }
                    current.clear();
                }

                in_string = !in_string;
                current.push(ch);
            } else {
                current.push(ch);
            }
            i += 1;
        }

        if !current.is_empty() {
            if in_string {
                spans.push(Span::styled(
                    current.clone(),
                    Style::default().fg(theme.string),
                ));
            } else {
                let trimmed = current.trim();
                if trimmed.parse::<f64>().is_ok() {
                    spans.push(Span::styled(
                        current.clone(),
                        Style::default().fg(theme.number),
                    ));
                } else if trimmed == "true" || trimmed == "false" || trimmed == "null" {
                    spans.push(Span::styled(
                        current.clone(),
                        Style::default().fg(theme.keyword),
                    ));
                } else {
                    spans.push(Span::styled(
                        current.clone(),
                        Style::default().fg(theme.text),
                    ));
                }
            }
        }

        if spans.is_empty() {
            vec![Span::styled(
                line.to_string(),
                Style::default().fg(theme.text),
            )]
        } else {
            spans
        }
    }

    fn highlight_bash(&self, line: &str, theme: &CodeTheme) -> Vec<Span<'static>> {
        let keywords = [
            "if", "then", "else", "elif", "fi", "for", "do", "done", "while", "until", "case",
            "esac", "function", "return", "exit", "export", "source", "echo", "printf", "read",
            "cd", "pwd", "ls", "mkdir", "rm", "cp", "mv", "cat", "grep", "sed", "awk", "find",
            "chmod", "chown", "sudo", "apt", "yum", "dnf", "pacman", "brew", "pip", "cargo",
        ];

        highlight_with_keywords(line, theme, &keywords)
    }
}

fn highlight_with_keywords(line: &str, theme: &CodeTheme, keywords: &[&str]) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut current = String::new();
    let mut in_string = false;
    let in_comment = false;
    let string_chars = ['"', '\''];

    let trimmed = line.trim_start();
    if trimmed.starts_with("//") || trimmed.starts_with('#') {
        return vec![Span::styled(
            line.to_string(),
            Style::default().fg(theme.comment),
        )];
    }

    for ch in line.chars() {
        if in_comment {
            current.push(ch);
            continue;
        }

        if string_chars.contains(&ch) {
            if !in_string {
                if !current.is_empty() {
                    add_keyword_span(&mut spans, &current, theme, keywords);
                    current.clear();
                }
                in_string = true;
                current.push(ch);
            } else {
                current.push(ch);
                spans.push(Span::styled(
                    current.clone(),
                    Style::default().fg(theme.string),
                ));
                current.clear();
                in_string = false;
            }
        } else if in_string {
            current.push(ch);
        } else if ch.is_whitespace() || is_punctuation(ch) {
            if !current.is_empty() {
                add_keyword_span(&mut spans, &current, theme, keywords);
                current.clear();
            }
            spans.push(Span::styled(
                ch.to_string(),
                Style::default().fg(theme.punctuation),
            ));
        } else {
            current.push(ch);
        }
    }

    if !current.is_empty() {
        if in_string || in_comment {
            spans.push(Span::styled(
                current.clone(),
                Style::default().fg(if in_comment {
                    theme.comment
                } else {
                    theme.string
                }),
            ));
        } else {
            add_keyword_span(&mut spans, &current, theme, keywords);
        }
    }

    if spans.is_empty() {
        vec![Span::styled(
            line.to_string(),
            Style::default().fg(theme.text),
        )]
    } else {
        spans
    }
}

fn add_keyword_span(
    spans: &mut Vec<Span<'static>>,
    text: &str,
    theme: &CodeTheme,
    keywords: &[&str],
) {
    let trimmed = text.trim();

    if keywords.contains(&trimmed) {
        spans.push(Span::styled(
            text.to_string(),
            Style::default()
                .fg(theme.keyword)
                .add_modifier(Modifier::BOLD),
        ));
    } else if trimmed.parse::<f64>().is_ok() {
        spans.push(Span::styled(
            text.to_string(),
            Style::default().fg(theme.number),
        ));
    } else {
        spans.push(Span::styled(
            text.to_string(),
            Style::default().fg(theme.text),
        ));
    }
}

fn is_punctuation(ch: char) -> bool {
    matches!(
        ch,
        '(' | ')'
            | '{'
            | '}'
            | '['
            | ']'
            | ';'
            | ','
            | '.'
            | ':'
            | '='
            | '+'
            | '-'
            | '*'
            | '/'
            | '<'
            | '>'
            | '!'
            | '&'
            | '|'
    )
}

#[derive(Clone, Debug)]
pub struct CodeTheme {
    pub text: Color,
    pub keyword: Color,
    pub string: Color,
    pub number: Color,
    pub comment: Color,
    pub punctuation: Color,
    pub function: Color,
}

impl Default for CodeTheme {
    fn default() -> Self {
        Self {
            text: Color::Rgb(220, 220, 220),
            keyword: Color::Rgb(255, 121, 198),
            string: Color::Rgb(230, 219, 116),
            number: Color::Rgb(174, 129, 255),
            comment: Color::Rgb(117, 113, 94),
            punctuation: Color::Rgb(248, 248, 242),
            function: Color::Rgb(102, 217, 239),
        }
    }
}

impl CodeTheme {
    pub fn from_app_theme(theme: &crate::theme::Theme) -> Self {
        Self {
            text: theme.text,
            keyword: theme.primary,
            string: theme.success,
            number: theme.warning,
            comment: theme.text_muted,
            punctuation: theme.border,
            function: theme.info,
        }
    }
}
