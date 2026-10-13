#![allow(dead_code)]

#[derive(Clone, Debug, PartialEq)]
pub enum MarkdownBlock {
    Heading {
        level: u8,
        content: String,
    },
    Paragraph {
        content: String,
        inline: Vec<InlineElement>,
    },
    CodeBlock {
        language: Option<String>,
        code: String,
    },
    List {
        items: Vec<ListItem>,
        ordered: bool,
    },
    BlockQuote {
        content: String,
    },
    ThematicBreak,
    Table {
        headers: Vec<String>,
        rows: Vec<Vec<String>>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum InlineElement {
    Plain(String),
    Bold(String),
    Italic(String),
    BoldItalic(String),
    Code(String),
    Link { text: String, url: String },
    Strikethrough(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct ListItem {
    pub content: String,
    pub inline: Vec<InlineElement>,
    pub children: Vec<ListItem>,
}

pub fn parse_markdown(text: &str) -> Vec<MarkdownBlock> {
    let mut blocks = Vec::new();
    let mut lines = text.lines().peekable();
    let mut in_code_block = false;
    let mut code_block_content = String::new();
    let mut code_block_lang: Option<String> = None;

    while let Some(line) = lines.next() {
        if line.starts_with("```") {
            if in_code_block {
                blocks.push(MarkdownBlock::CodeBlock {
                    language: code_block_lang.take(),
                    code: code_block_content.trim_end().to_string(),
                });
                code_block_content.clear();
                in_code_block = false;
            } else {
                in_code_block = true;
                code_block_lang = line.strip_prefix("```").map(|s| s.trim().to_string());
                if code_block_lang.as_ref().map_or(false, |s| s.is_empty()) {
                    code_block_lang = None;
                }
            }
            continue;
        }

        if in_code_block {
            if !code_block_content.is_empty() {
                code_block_content.push('\n');
            }
            code_block_content.push_str(line);
            continue;
        }

        let trimmed = line.trim();

        if trimmed.is_empty() {
            continue;
        }

        if trimmed.starts_with("---") || trimmed.starts_with("***") || trimmed.starts_with("___") {
            blocks.push(MarkdownBlock::ThematicBreak);
            continue;
        }

        if let Some(heading) = parse_heading(trimmed) {
            blocks.push(heading);
            continue;
        }

        if trimmed.starts_with('>') {
            let content = trimmed.trim_start_matches('>').trim().to_string();
            blocks.push(MarkdownBlock::BlockQuote { content });
            continue;
        }

        if trimmed.starts_with("- ") || trimmed.starts_with("* ") || trimmed.starts_with("+ ") {
            let items = parse_list(&mut lines, trimmed, false);
            blocks.push(MarkdownBlock::List {
                items,
                ordered: false,
            });
            continue;
        }

        if let Some(items) = try_parse_ordered_list(trimmed) {
            let more_items = collect_ordered_list(&mut lines);
            let mut all_items = items;
            all_items.extend(more_items);
            blocks.push(MarkdownBlock::List {
                items: all_items,
                ordered: true,
            });
            continue;
        }

        let inline = parse_inline(trimmed);
        blocks.push(MarkdownBlock::Paragraph {
            content: trimmed.to_string(),
            inline,
        });
    }

    blocks
}

fn parse_heading(line: &str) -> Option<MarkdownBlock> {
    let hash_count = line.chars().take_while(|&c| c == '#').count() as u8;
    if hash_count > 0 && hash_count <= 6 {
        let content = line[hash_count as usize..].trim().to_string();
        Some(MarkdownBlock::Heading {
            level: hash_count,
            content,
        })
    } else {
        None
    }
}

fn parse_list(
    lines: &mut std::iter::Peekable<std::str::Lines>,
    first_line: &str,
    _ordered: bool,
) -> Vec<ListItem> {
    let mut items = Vec::new();

    let content = first_line
        .trim_start_matches(|c| c == '-' || c == '*' || c == '+')
        .trim()
        .to_string();
    let inline = parse_inline(&content);
    items.push(ListItem {
        content,
        inline,
        children: Vec::new(),
    });

    while let Some(&next_line) = lines.peek() {
        let trimmed = next_line.trim();
        if trimmed.is_empty() {
            lines.next();
            continue;
        }
        if trimmed.starts_with("- ") || trimmed.starts_with("* ") || trimmed.starts_with("+ ") {
            lines.next();
            let content = trimmed
                .trim_start_matches(|c| c == '-' || c == '*' || c == '+')
                .trim()
                .to_string();
            let inline = parse_inline(&content);
            items.push(ListItem {
                content,
                inline,
                children: Vec::new(),
            });
        } else {
            break;
        }
    }

    items
}

fn try_parse_ordered_list(line: &str) -> Option<Vec<ListItem>> {
    let chars: Vec<char> = line.chars().collect();
    if chars.len() < 3 {
        return None;
    }

    let mut i = 0;
    while i < chars.len() && chars[i].is_ascii_digit() {
        i += 1;
    }

    if i > 0 && i < chars.len() && (chars[i] == '.' || chars[i] == ')') {
        let content = line[i + 1..].trim().to_string();
        let inline = parse_inline(&content);
        Some(vec![ListItem {
            content,
            inline,
            children: Vec::new(),
        }])
    } else {
        None
    }
}

fn collect_ordered_list(lines: &mut std::iter::Peekable<std::str::Lines>) -> Vec<ListItem> {
    let mut items = Vec::new();

    while let Some(&next_line) = lines.peek() {
        let trimmed = next_line.trim();
        if trimmed.is_empty() {
            lines.next();
            continue;
        }
        if let Some(item) = try_parse_ordered_list(trimmed) {
            lines.next();
            items.extend(item);
        } else {
            break;
        }
    }

    items
}

pub fn parse_inline(text: &str) -> Vec<InlineElement> {
    let mut elements = Vec::new();
    let mut current = String::new();
    let mut chars = text.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '`' {
            if !current.is_empty() {
                elements.push(InlineElement::Plain(current.clone()));
                current.clear();
            }

            let mut code = String::new();
            while let Some(&next) = chars.peek() {
                if next == '`' {
                    chars.next();
                    break;
                }
                code.push(chars.next().unwrap());
            }
            elements.push(InlineElement::Code(code));
        } else if ch == '*' || ch == '_' {
            let marker = ch;
            let double = chars.peek() == Some(&marker);

            if double {
                chars.next();
                let mut content = String::new();
                let mut found_end = false;

                while let Some(&next) = chars.peek() {
                    if next == marker {
                        chars.next();
                        if chars.peek() == Some(&marker) {
                            chars.next();
                            found_end = true;
                            break;
                        }
                        content.push(marker);
                    } else {
                        content.push(chars.next().unwrap());
                    }
                }

                if found_end && !content.is_empty() {
                    if !current.is_empty() {
                        elements.push(InlineElement::Plain(current.clone()));
                        current.clear();
                    }
                    elements.push(InlineElement::Bold(content));
                } else {
                    current.push(marker);
                    current.push(marker);
                    current.push_str(&content);
                }
            } else {
                let mut content = String::new();
                let mut found_end = false;

                while let Some(&next) = chars.peek() {
                    if next == marker {
                        chars.next();
                        found_end = true;
                        break;
                    }
                    content.push(chars.next().unwrap());
                }

                if found_end && !content.is_empty() {
                    if !current.is_empty() {
                        elements.push(InlineElement::Plain(current.clone()));
                        current.clear();
                    }
                    elements.push(InlineElement::Italic(content));
                } else {
                    current.push(marker);
                    current.push_str(&content);
                }
            }
        } else if ch == '[' {
            if !current.is_empty() {
                elements.push(InlineElement::Plain(current.clone()));
                current.clear();
            }

            let mut link_text = String::new();
            while let Some(&next) = chars.peek() {
                if next == ']' {
                    chars.next();
                    break;
                }
                link_text.push(chars.next().unwrap());
            }

            if chars.peek() == Some(&'(') {
                chars.next();
                let mut url = String::new();
                while let Some(&next) = chars.peek() {
                    if next == ')' {
                        chars.next();
                        break;
                    }
                    url.push(chars.next().unwrap());
                }
                elements.push(InlineElement::Link {
                    text: link_text,
                    url,
                });
            } else {
                current.push('[');
                current.push_str(&link_text);
                current.push(']');
            }
        } else {
            current.push(ch);
        }
    }

    if !current.is_empty() {
        elements.push(InlineElement::Plain(current));
    }

    if elements.is_empty() {
        vec![InlineElement::Plain(text.to_string())]
    } else {
        elements
    }
}
