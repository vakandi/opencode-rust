use ratatui::text::Span;
use unicode_width::UnicodeWidthStr;

pub fn truncate(text: &str, max_width: usize) -> String {
    let width = UnicodeWidthStr::width(text);
    if width <= max_width {
        return text.to_string();
    }

    let mut result = String::new();
    let mut current_width = 0;

    for ch in text.chars() {
        let ch_str = ch.to_string();
        let ch_width = UnicodeWidthStr::width(ch_str.as_str());
        if current_width + ch_width + 3 > max_width {
            break;
        }
        result.push(ch);
        current_width += ch_width;
    }

    result.push_str("...");
    result
}

pub fn pad_left(text: &str, width: usize) -> String {
    let text_width = text.width();
    if text_width >= width {
        return text.to_string();
    }
    format!("{}{}", " ".repeat(width - text_width), text)
}

pub fn pad_right(text: &str, width: usize) -> String {
    let text_width = text.width();
    if text_width >= width {
        return text.to_string();
    }
    format!("{}{}", text, " ".repeat(width - text_width))
}

pub fn center_text(text: &str, width: usize) -> String {
    let text_width = text.width();
    if text_width >= width {
        return text.to_string();
    }
    let left_pad = (width - text_width) / 2;
    format!("{}{}", " ".repeat(left_pad), text)
}

pub fn highlight_text<'a>(text: &'a str, color: ratatui::style::Color) -> Span<'a> {
    Span::styled(text, ratatui::style::Style::default().fg(color))
}
