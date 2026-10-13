use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::Paragraph,
    Frame,
};

const LOGO_LEFT: [&str; 4] = ["          ", "█▀▀▄ █▀▀█ ", "█▀█▀ █__█ ", "▀ ▀▀ ▀▀▀▀ "];

const LOGO_RIGHT: [&str; 4] = [
    "             ▄     ",
    "█▀▀▀ █▀▀█ █▀▀█ █▀▀▀",
    "█___ █__█ █__█ █▀▀▀",
    "▀▀▀▀ ▀▀▀▀ ▀▀▀▀ ▀▀▀▀",
];

const MARKS: &str = "_^~";
const SHADOW_TINT_FACTOR: f32 = 0.25;

fn blend_colors(fg: Color, bg: Color) -> Color {
    match (fg, bg) {
        (Color::Rgb(r1, g1, b1), Color::Rgb(r2, g2, b2)) => {
            let r = ((1.0 - SHADOW_TINT_FACTOR) * r2 as f32 + SHADOW_TINT_FACTOR * r1 as f32) as u8;
            let g = ((1.0 - SHADOW_TINT_FACTOR) * g2 as f32 + SHADOW_TINT_FACTOR * g1 as f32) as u8;
            let b = ((1.0 - SHADOW_TINT_FACTOR) * b2 as f32 + SHADOW_TINT_FACTOR * b1 as f32) as u8;
            Color::Rgb(r, g, b)
        }
        _ => bg,
    }
}

/// Parse a single logo line, replacing marker characters with styled shadow spans.
/// `bold` controls whether non-marker characters get the BOLD modifier (used for the right half).
fn parse_logo_line(line: &str, fg_color: Color, bg_color: Color, bold: bool) -> Vec<Span<'_>> {
    let shadow_color = blend_colors(fg_color, bg_color);
    let base_style = if bold {
        Style::default().fg(fg_color).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(fg_color)
    };
    let mut spans = Vec::new();

    for c in line.chars() {
        let span = if MARKS.contains(c) {
            match c {
                '_' => Span::styled(
                    " ",
                    if bold {
                        Style::default()
                            .fg(fg_color)
                            .bg(shadow_color)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(fg_color).bg(shadow_color)
                    },
                ),
                '^' => Span::styled(
                    "▀",
                    if bold {
                        Style::default()
                            .fg(fg_color)
                            .bg(shadow_color)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(fg_color).bg(shadow_color)
                    },
                ),
                '~' => Span::styled(
                    "▀",
                    if bold {
                        Style::default()
                            .fg(shadow_color)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(shadow_color)
                    },
                ),
                _ => Span::styled(c.to_string(), base_style),
            }
        } else {
            Span::styled(c.to_string(), base_style)
        };
        spans.push(span);
    }

    spans
}

pub struct Logo {
    primary_color: Color,
    muted_color: Color,
    background_color: Color,
}

impl Logo {
    pub fn new(text_color: Color, text_muted_color: Color, bg_color: Color) -> Self {
        Self {
            primary_color: text_color,
            muted_color: text_muted_color,
            background_color: bg_color,
        }
    }

    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let mut lines = Vec::new();

        for i in 0..4 {
            let left_line = LOGO_LEFT[i];
            let right_line = LOGO_RIGHT[i];

            let left_spans =
                parse_logo_line(left_line, self.muted_color, self.background_color, false);
            let gap = Span::styled(" ", Style::default().fg(self.muted_color));
            let right_spans =
                parse_logo_line(right_line, self.primary_color, self.background_color, true);

            let mut combined = left_spans;
            combined.push(gap);
            combined.extend(right_spans);

            lines.push(Line::from(combined));
        }

        let paragraph =
            Paragraph::new(Text::from(lines)).alignment(ratatui::layout::Alignment::Center);

        frame.render_widget(paragraph, area);
    }
}
