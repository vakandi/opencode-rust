use ratatui::style::Color;

use crate::theme::Theme;

pub fn user_message_bg(theme: &Theme) -> Color {
    theme.background_panel
}

pub fn thinking_message_bg(theme: &Theme) -> Color {
    // Thinking sits between panel and menu tones for subtle separation.
    blend(theme.background_menu, theme.background_panel, 1, 3)
}

pub fn assistant_border_color(theme: &Theme) -> Color {
    theme.info
}

pub fn thinking_border_color(theme: &Theme) -> Color {
    theme.background_element
}

fn blend(foreground: Color, background: Color, num: u16, den: u16) -> Color {
    if den == 0 || num >= den {
        return foreground;
    }

    match (foreground, background) {
        (Color::Rgb(fr, fg, fb), Color::Rgb(br, bg, bb)) => {
            let inv = den - num;
            let r = ((u16::from(fr) * inv) + (u16::from(br) * num)) / den;
            let g = ((u16::from(fg) * inv) + (u16::from(bg) * num)) / den;
            let b = ((u16::from(fb) * inv) + (u16::from(bb) * num)) / den;
            Color::Rgb(r as u8, g as u8, b as u8)
        }
        _ => foreground,
    }
}
