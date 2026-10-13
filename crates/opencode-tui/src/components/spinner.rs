use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

const DEFAULT_WIDTH: usize = 8;
const DEFAULT_HOLD_START: usize = 30;
const DEFAULT_HOLD_END: usize = 9;
const DEFAULT_TRAIL_STEPS: usize = 6;
const KNIGHT_RIDER_FRAME_INTERVAL_MS: u64 = 40;
const BRAILLE_FRAME_INTERVAL_MS: u64 = 80;
const BRAILLE_FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TaskKind {
    #[default]
    LlmRequest,
    LlmResponse,
    FileRead,
    FileWrite,
    CommandExec,
    ToolCall,
}

impl TaskKind {
    pub fn shape(self) -> &'static str {
        match self {
            Self::LlmRequest => "◆",
            Self::LlmResponse => "▶",
            Self::FileRead => "◈",
            Self::FileWrite => "◇",
            Self::CommandExec => "⬡",
            Self::ToolCall => "●",
        }
    }

    pub fn inactive_shape() -> &'static str {
        "·"
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SpinnerMode {
    #[default]
    Braille,
    KnightRider,
}

#[derive(Clone, Copy, Debug)]
struct ScannerState {
    active_position: usize,
    is_holding: bool,
    hold_progress: usize,
    is_moving_forward: bool,
}

#[derive(Clone, Debug)]
pub struct KnightRiderSpinner {
    width: usize,
    frame_index: usize,
    knight_total_frames: usize,
    hold_start: usize,
    hold_end: usize,
    task_kind: TaskKind,
    mode: SpinnerMode,
    base_color: Color,
    trail_colors: Vec<Color>,
    inactive_color: Color,
    active: bool,
    frame_interval_ms: u64,
    tick_accumulator_ms: u64,
}

pub type Spinner = KnightRiderSpinner;

impl KnightRiderSpinner {
    pub fn new() -> Self {
        Self::with_color(Color::Rgb(255, 0, 0))
    }

    pub fn with_color(base_color: Color) -> Self {
        let hold_start = DEFAULT_HOLD_START;
        let hold_end = DEFAULT_HOLD_END;
        let width = DEFAULT_WIDTH;
        let knight_total_frames = width + hold_end + (width.saturating_sub(1)) + hold_start;
        let trail_colors = derive_trail_colors(base_color, DEFAULT_TRAIL_STEPS);
        let inactive_color = derive_inactive_color(base_color, 0.6);

        Self {
            width,
            frame_index: 0,
            knight_total_frames,
            hold_start,
            hold_end,
            task_kind: TaskKind::default(),
            mode: SpinnerMode::Braille,
            base_color,
            trail_colors,
            inactive_color,
            active: false,
            frame_interval_ms: BRAILLE_FRAME_INTERVAL_MS,
            tick_accumulator_ms: 0,
        }
    }

    pub fn set_active(&mut self, active: bool) {
        self.active = active;
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    pub fn set_task_kind(&mut self, task_kind: TaskKind) {
        self.task_kind = task_kind;
    }

    pub fn task_kind(&self) -> TaskKind {
        self.task_kind
    }

    pub fn set_mode(&mut self, mode: SpinnerMode) {
        if self.mode == mode {
            return;
        }
        self.mode = mode;
        self.frame_index = 0;
        self.tick_accumulator_ms = 0;
        self.frame_interval_ms = match mode {
            SpinnerMode::Braille => BRAILLE_FRAME_INTERVAL_MS,
            SpinnerMode::KnightRider => KNIGHT_RIDER_FRAME_INTERVAL_MS,
        };
    }

    pub fn mode(&self) -> SpinnerMode {
        self.mode
    }

    pub fn set_color(&mut self, color: Color) {
        if self.base_color == color {
            return;
        }
        self.base_color = color;
        self.trail_colors = derive_trail_colors(color, DEFAULT_TRAIL_STEPS);
        self.inactive_color = derive_inactive_color(color, 0.6);
    }

    pub fn set_frame_interval_ms(&mut self, interval_ms: u64) {
        self.frame_interval_ms = interval_ms.max(1);
    }

    pub fn tick(&mut self) -> bool {
        let prev = self.frame_index;
        let total = match self.mode {
            SpinnerMode::Braille => BRAILLE_FRAMES.len(),
            SpinnerMode::KnightRider => self.knight_total_frames,
        };
        self.frame_index = (self.frame_index + 1) % total.max(1);
        self.frame_index != prev
    }

    pub fn advance(&mut self, delta_ms: u64) -> bool {
        if !self.active || delta_ms == 0 {
            return false;
        }

        self.tick_accumulator_ms = self.tick_accumulator_ms.saturating_add(delta_ms);
        let mut changed = false;
        while self.tick_accumulator_ms >= self.frame_interval_ms {
            changed |= self.tick();
            self.tick_accumulator_ms -= self.frame_interval_ms;
        }
        changed
    }

    pub fn render(
        &self,
        frame: &mut Frame,
        area: Rect,
        animations_enabled: bool,
        background: Color,
    ) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let base_style = Style::default().bg(background);

        if matches!(self.mode, SpinnerMode::Braille) {
            self.render_braille(frame, area, animations_enabled, base_style);
            return;
        }

        if !self.active {
            let mut spans = Vec::with_capacity(self.width);
            for idx in 0..self.width {
                let symbol = if idx % 2 == 0 {
                    TaskKind::inactive_shape()
                } else {
                    "◦"
                };
                spans.push(Span::styled(
                    symbol,
                    Style::default().fg(self.inactive_color),
                ));
            }
            frame.render_widget(Paragraph::new(Line::from(spans)).style(base_style), area);
            return;
        }

        if !animations_enabled {
            let fallback = format!("{:<width$}", "[⋯]", width = self.width);
            frame.render_widget(Paragraph::new(fallback).style(base_style), area);
            return;
        }

        let scanner = self.scanner_state();
        let mut spans = Vec::with_capacity(self.width);
        for char_index in 0..self.width {
            let color_index = self.color_index(char_index, scanner);
            if let Some(index) = color_index {
                let color = self
                    .trail_colors
                    .get(index)
                    .copied()
                    .unwrap_or(self.inactive_color);
                spans.push(Span::styled(
                    self.task_kind.shape(),
                    Style::default().fg(color),
                ));
            } else {
                spans.push(Span::styled(
                    TaskKind::inactive_shape(),
                    Style::default().fg(self.inactive_color),
                ));
            }
        }

        frame.render_widget(Paragraph::new(Line::from(spans)).style(base_style), area);
    }

    fn render_braille(
        &self,
        frame: &mut Frame,
        area: Rect,
        animations_enabled: bool,
        base_style: Style,
    ) {
        let (symbol, color) = if !self.active {
            ("·", self.inactive_color)
        } else if !animations_enabled {
            ("⋯", self.inactive_color)
        } else {
            (
                BRAILLE_FRAMES[self.frame_index % BRAILLE_FRAMES.len()],
                self.base_color,
            )
        };
        let mut spans = vec![Span::styled(symbol, Style::default().fg(color))];
        let trailing = usize::from(area.width).saturating_sub(1);
        if trailing > 0 {
            spans.push(Span::styled(
                " ".repeat(trailing),
                Style::default().fg(color),
            ));
        }
        let line = Line::from(spans);
        frame.render_widget(Paragraph::new(line).style(base_style), area);
    }

    fn scanner_state(&self) -> ScannerState {
        let frame = self.frame_index % self.knight_total_frames.max(1);
        let width = self.width;
        let forward_frames = width;
        let hold_end_frames = self.hold_end;
        let backward_frames = width.saturating_sub(1);
        let hold_start_frames = self.hold_start;

        if frame < forward_frames {
            ScannerState {
                active_position: frame,
                is_holding: false,
                hold_progress: 0,
                is_moving_forward: true,
            }
        } else if frame < forward_frames + hold_end_frames {
            ScannerState {
                active_position: width.saturating_sub(1),
                is_holding: true,
                hold_progress: frame - forward_frames,
                is_moving_forward: true,
            }
        } else if frame < forward_frames + hold_end_frames + backward_frames {
            let backward_index = frame - forward_frames - hold_end_frames;
            ScannerState {
                active_position: width.saturating_sub(2).saturating_sub(backward_index),
                is_holding: false,
                hold_progress: 0,
                is_moving_forward: false,
            }
        } else if frame < forward_frames + hold_end_frames + backward_frames + hold_start_frames {
            ScannerState {
                active_position: 0,
                is_holding: true,
                hold_progress: frame - forward_frames - hold_end_frames - backward_frames,
                is_moving_forward: false,
            }
        } else {
            ScannerState {
                active_position: 0,
                is_holding: true,
                hold_progress: 0,
                is_moving_forward: false,
            }
        }
    }

    fn color_index(&self, char_index: usize, scanner: ScannerState) -> Option<usize> {
        let directional_distance = if scanner.is_moving_forward {
            scanner.active_position as i32 - char_index as i32
        } else {
            char_index as i32 - scanner.active_position as i32
        };

        if scanner.is_holding {
            let index = directional_distance + scanner.hold_progress as i32;
            return usize::try_from(index)
                .ok()
                .filter(|idx| *idx < self.trail_colors.len());
        }

        if directional_distance == 0 {
            return Some(0);
        }

        usize::try_from(directional_distance)
            .ok()
            .filter(|idx| *idx < self.trail_colors.len())
    }
}

impl Default for KnightRiderSpinner {
    fn default() -> Self {
        Self::new()
    }
}

fn derive_trail_colors(base: Color, steps: usize) -> Vec<Color> {
    (0..steps)
        .map(|idx| {
            if idx == 0 {
                return apply_brightness(base, 1.0);
            }
            if idx == 1 {
                return blend_alpha(apply_brightness(base, 1.15), 0.9);
            }
            blend_alpha(base, 0.65_f64.powi((idx as i32) - 1))
        })
        .collect()
}

fn derive_inactive_color(base: Color, factor: f64) -> Color {
    blend_alpha(base, factor)
}

fn apply_brightness(color: Color, factor: f64) -> Color {
    let (r, g, b) = color_to_rgb(color);
    Color::Rgb(
        ((r as f64) * factor).round().clamp(0.0, 255.0) as u8,
        ((g as f64) * factor).round().clamp(0.0, 255.0) as u8,
        ((b as f64) * factor).round().clamp(0.0, 255.0) as u8,
    )
}

fn blend_alpha(color: Color, alpha: f64) -> Color {
    let (r, g, b) = color_to_rgb(color);
    let a = alpha.clamp(0.0, 1.0);
    Color::Rgb(
        ((r as f64) * a).round().clamp(0.0, 255.0) as u8,
        ((g as f64) * a).round().clamp(0.0, 255.0) as u8,
        ((b as f64) * a).round().clamp(0.0, 255.0) as u8,
    )
}

fn color_to_rgb(color: Color) -> (u8, u8, u8) {
    match color {
        Color::Rgb(r, g, b) => (r, g, b),
        Color::Black => (0, 0, 0),
        Color::Red => (255, 0, 0),
        Color::Green => (0, 255, 0),
        Color::Yellow => (255, 255, 0),
        Color::Blue => (0, 0, 255),
        Color::Magenta => (255, 0, 255),
        Color::Cyan => (0, 255, 255),
        Color::Gray => (128, 128, 128),
        Color::DarkGray => (96, 96, 96),
        Color::LightRed => (255, 102, 102),
        Color::LightGreen => (102, 255, 102),
        Color::LightYellow => (255, 255, 153),
        Color::LightBlue => (102, 153, 255),
        Color::LightMagenta => (255, 153, 255),
        Color::LightCyan => (153, 255, 255),
        Color::White => (255, 255, 255),
        Color::Indexed(index) => (index, index, index),
        Color::Reset => (140, 140, 140),
    }
}
