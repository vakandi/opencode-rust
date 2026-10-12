use ratatui::{
    layout::Rect,
    style::{Color, Style},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

use crate::theme::Theme;

const TRANSITION_MS: u64 = 120;
const SLIDE_OFFSET_COLS: u16 = 4;
const MIN_FADE_ALPHA: f32 = 0.45;

pub struct Toast {
    message: Option<String>,
    variant: ToastVariant,
    duration_ms: u64,
    phase: ToastPhase,
    phase_elapsed_ms: u64,
}

#[derive(Clone, Copy)]
pub enum ToastVariant {
    Info,
    Success,
    Warning,
    Error,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ToastPhase {
    Entering,
    Visible,
    Exiting,
}

impl Toast {
    pub fn new() -> Self {
        Self {
            message: None,
            variant: ToastVariant::Info,
            duration_ms: 3000,
            phase: ToastPhase::Entering,
            phase_elapsed_ms: 0,
        }
    }

    pub fn show(&mut self, variant: ToastVariant, message: &str, duration_ms: u64) {
        self.variant = variant;
        self.message = Some(message.to_string());
        self.duration_ms = duration_ms;
        self.phase = ToastPhase::Entering;
        self.phase_elapsed_ms = 0;
    }

    pub fn tick(&mut self, delta_ms: u64) -> bool {
        if self.message.is_none() {
            return false;
        }

        let previous_phase = self.phase;
        let previous_elapsed = self.phase_elapsed_ms;
        let had_message = self.message.is_some();

        self.phase_elapsed_ms = self.phase_elapsed_ms.saturating_add(delta_ms);
        match self.phase {
            ToastPhase::Entering if self.phase_elapsed_ms >= TRANSITION_MS => {
                self.phase = ToastPhase::Visible;
                self.phase_elapsed_ms = 0;
            }
            ToastPhase::Visible if self.phase_elapsed_ms >= self.duration_ms => {
                self.phase = ToastPhase::Exiting;
                self.phase_elapsed_ms = 0;
            }
            ToastPhase::Exiting if self.phase_elapsed_ms >= TRANSITION_MS => {
                self.message = None;
                self.phase = ToastPhase::Entering;
                self.phase_elapsed_ms = 0;
            }
            _ => {}
        }

        if self.phase != previous_phase || self.message.is_some() != had_message {
            return true;
        }

        matches!(self.phase, ToastPhase::Entering | ToastPhase::Exiting)
            && self.phase_elapsed_ms != previous_elapsed
    }

    pub fn is_visible(&self) -> bool {
        self.message.is_some()
    }

    pub fn slide_offset(&self) -> u16 {
        let progress = self.animation_progress();
        let hidden = (1.0 - progress) * SLIDE_OFFSET_COLS as f32;
        hidden.round() as u16
    }

    pub fn desired_height(&self, width: u16) -> u16 {
        let Some(msg) = self.message.as_ref() else {
            return 0;
        };
        if width == 0 {
            return 0;
        }
        let text_width = usize::from(width.saturating_sub(4)).max(1);
        let lines = msg.chars().count().div_ceil(text_width).max(1) as u16;
        // 2 for vertical padding/border room.
        (lines + 2).clamp(3, 6)
    }

    pub fn render(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        if let Some(msg) = &self.message {
            let progress = self.animation_progress();
            let fade_alpha = MIN_FADE_ALPHA + (1.0 - MIN_FADE_ALPHA) * progress;
            let border_color = match self.variant {
                ToastVariant::Info => theme.info,
                ToastVariant::Success => theme.success,
                ToastVariant::Warning => theme.warning,
                ToastVariant::Error => theme.error,
            };
            let text_color = blend_colors(theme.text_muted, theme.text, progress);
            let panel_bg = blend_colors(theme.background, theme.background_panel, fade_alpha);
            let border_color = blend_colors(theme.background_panel, border_color, fade_alpha);

            let paragraph = Paragraph::new(msg.clone())
                .style(Style::default().fg(text_color).bg(panel_bg))
                .block(
                    Block::default()
                        .borders(Borders::LEFT | Borders::RIGHT)
                        .border_style(Style::default().fg(border_color))
                        .style(Style::default().bg(panel_bg)),
                )
                .wrap(Wrap { trim: true });
            frame.render_widget(paragraph, area);
        }
    }

    fn animation_progress(&self) -> f32 {
        if self.message.is_none() {
            return 0.0;
        }

        match self.phase {
            ToastPhase::Entering => progress_ratio(self.phase_elapsed_ms, TRANSITION_MS),
            ToastPhase::Visible => 1.0,
            ToastPhase::Exiting => 1.0 - progress_ratio(self.phase_elapsed_ms, TRANSITION_MS),
        }
        .clamp(0.0, 1.0)
    }
}

impl Default for Toast {
    fn default() -> Self {
        Self::new()
    }
}

fn progress_ratio(elapsed_ms: u64, total_ms: u64) -> f32 {
    if total_ms == 0 {
        return 1.0;
    }
    (elapsed_ms as f32 / total_ms as f32).clamp(0.0, 1.0)
}

fn blend_colors(from: Color, to: Color, alpha: f32) -> Color {
    let alpha = alpha.clamp(0.0, 1.0);
    let (fr, fg, fb) = color_to_rgb(from);
    let (tr, tg, tb) = color_to_rgb(to);
    Color::Rgb(
        mix_u8(fr, tr, alpha),
        mix_u8(fg, tg, alpha),
        mix_u8(fb, tb, alpha),
    )
}

fn mix_u8(from: u8, to: u8, alpha: f32) -> u8 {
    let from = from as f32;
    let to = to as f32;
    (from + (to - from) * alpha).round().clamp(0.0, 255.0) as u8
}

fn color_to_rgb(color: Color) -> (u8, u8, u8) {
    match color {
        Color::Black => (0, 0, 0),
        Color::Red => (205, 49, 49),
        Color::Green => (13, 188, 121),
        Color::Yellow => (229, 229, 16),
        Color::Blue => (36, 114, 200),
        Color::Magenta => (188, 63, 188),
        Color::Cyan => (17, 168, 205),
        Color::Gray => (204, 204, 204),
        Color::DarkGray => (118, 118, 118),
        Color::LightRed => (241, 76, 76),
        Color::LightGreen => (35, 209, 139),
        Color::LightYellow => (245, 245, 67),
        Color::LightBlue => (59, 142, 234),
        Color::LightMagenta => (214, 112, 214),
        Color::LightCyan => (41, 184, 219),
        Color::White => (255, 255, 255),
        Color::Rgb(r, g, b) => (r, g, b),
        Color::Indexed(v) => (v, v, v),
        Color::Reset => (0, 0, 0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toast_transitions_enter_visible_exit_and_hides() {
        let mut toast = Toast::new();
        toast.show(ToastVariant::Info, "hello", 120);
        assert!(toast.is_visible());

        toast.tick(TRANSITION_MS);
        assert_eq!(toast.phase, ToastPhase::Visible);
        assert!(toast.is_visible());

        toast.tick(120);
        assert_eq!(toast.phase, ToastPhase::Exiting);
        assert!(toast.is_visible());

        toast.tick(TRANSITION_MS);
        assert!(!toast.is_visible());
    }

    #[test]
    fn toast_slide_offset_drops_to_zero_when_fully_visible() {
        let mut toast = Toast::new();
        toast.show(ToastVariant::Info, "hello", 3000);
        let start = toast.slide_offset();
        assert!(start > 0);
        toast.tick(TRANSITION_MS);
        assert_eq!(toast.slide_offset(), 0);
    }
}
