use once_cell::sync::Lazy;
use ratatui::style::{Color, Modifier, Style};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;

const PRESET_SOURCES: &[(&str, &str)] = &[
    ("aura", include_str!("presets/aura.json")),
    ("ayu", include_str!("presets/ayu.json")),
    ("carbonfox", include_str!("presets/carbonfox.json")),
    ("catppuccin", include_str!("presets/catppuccin.json")),
    (
        "catppuccin-frappe",
        include_str!("presets/catppuccin-frappe.json"),
    ),
    (
        "catppuccin-macchiato",
        include_str!("presets/catppuccin-macchiato.json"),
    ),
    ("cobalt2", include_str!("presets/cobalt2.json")),
    ("cursor", include_str!("presets/cursor.json")),
    ("dracula", include_str!("presets/dracula.json")),
    ("everforest", include_str!("presets/everforest.json")),
    ("flexoki", include_str!("presets/flexoki.json")),
    ("github", include_str!("presets/github.json")),
    ("gruvbox", include_str!("presets/gruvbox.json")),
    ("kanagawa", include_str!("presets/kanagawa.json")),
    ("lucent-orng", include_str!("presets/lucent-orng.json")),
    ("material", include_str!("presets/material.json")),
    ("matrix", include_str!("presets/matrix.json")),
    ("mercury", include_str!("presets/mercury.json")),
    ("monokai", include_str!("presets/monokai.json")),
    ("nightowl", include_str!("presets/nightowl.json")),
    ("nord", include_str!("presets/nord.json")),
    ("one-dark", include_str!("presets/one-dark.json")),
    ("opencode", include_str!("presets/opencode.json")),
    ("orng", include_str!("presets/orng.json")),
    ("osaka-jade", include_str!("presets/osaka-jade.json")),
    ("palenight", include_str!("presets/palenight.json")),
    ("rosepine", include_str!("presets/rosepine.json")),
    ("solarized", include_str!("presets/solarized.json")),
    ("synthwave84", include_str!("presets/synthwave84.json")),
    ("tokyonight", include_str!("presets/tokyonight.json")),
    ("vercel", include_str!("presets/vercel.json")),
    ("vesper", include_str!("presets/vesper.json")),
    ("zenburn", include_str!("presets/zenburn.json")),
];

static PRESET_THEMES: Lazy<HashMap<String, ThemeFile>> = Lazy::new(|| {
    PRESET_SOURCES
        .iter()
        .filter_map(|(name, source)| {
            serde_json::from_str::<ThemeFile>(source)
                .ok()
                .map(|theme| ((*name).to_string(), theme))
        })
        .collect()
});

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThemeMode {
    Dark,
    Light,
}

#[derive(Clone, Debug)]
pub struct Theme {
    pub text: Color,
    pub text_muted: Color,
    pub background: Color,
    pub background_panel: Color,
    pub background_element: Color,
    pub border: Color,
    pub primary: Color,
    pub secondary: Color,
    pub success: Color,
    pub warning: Color,
    pub error: Color,
    pub info: Color,
    pub diff_added: Color,
    pub diff_removed: Color,
    pub agent_colors: Vec<Color>,
    // Extended semantic tokens
    pub background_menu: Color,
    pub border_active: Color,
    pub border_subtle: Color,
    // Markdown tokens
    pub markdown_heading: Color,
    pub markdown_link: Color,
    pub markdown_link_text: Color,
    pub markdown_code: Color,
    pub markdown_code_bg: Color,
    pub markdown_block_quote: Color,
    pub markdown_emph: Color,
    pub markdown_strong: Color,
    pub markdown_horizontal_rule: Color,
    pub markdown_list_item: Color,
    pub markdown_list_enumeration: Color,
    pub markdown_image: Color,
    pub markdown_image_text: Color,
    pub markdown_code_block: Color,
    // Syntax tokens
    pub syntax_comment: Color,
    pub syntax_keyword: Color,
    pub syntax_function: Color,
    pub syntax_string: Color,
    pub syntax_number: Color,
    // Diff background tokens
    pub diff_added_bg: Color,
    pub diff_removed_bg: Color,
    pub diff_context_bg: Color,
    // Tool semantic tokens
    pub tool_icon: Color,
    pub tool_border: Color,
}

#[derive(Debug, Deserialize)]
struct ThemeFile {
    #[serde(default)]
    defs: HashMap<String, Value>,
    theme: HashMap<String, Value>,
}

impl Default for Theme {
    fn default() -> Self {
        Self::fallback_dark()
    }
}

impl Theme {
    pub fn builtin_theme_names() -> Vec<&'static str> {
        PRESET_SOURCES.iter().map(|(name, _)| *name).collect()
    }

    pub fn dark() -> Self {
        Self::from_preset("opencode", ThemeMode::Dark).unwrap_or_else(Self::fallback_dark)
    }

    pub fn light() -> Self {
        Self::from_preset("opencode", ThemeMode::Light).unwrap_or_else(Self::fallback_light)
    }

    pub fn agent_color(&self, index: usize) -> Color {
        self.agent_colors[index % self.agent_colors.len()]
    }

    pub fn by_name(name: &str) -> Option<Self> {
        let normalized = name.trim();
        if normalized.is_empty() {
            return None;
        }

        match normalized {
            "dark" => return Some(Self::dark()),
            "light" => return Some(Self::light()),
            _ => {}
        }

        if let Some(base) = normalized
            .strip_suffix("@light")
            .or_else(|| normalized.strip_suffix(":light"))
        {
            return Self::from_preset(base, ThemeMode::Light);
        }
        if let Some(base) = normalized
            .strip_suffix("@dark")
            .or_else(|| normalized.strip_suffix(":dark"))
        {
            return Self::from_preset(base, ThemeMode::Dark);
        }

        Self::from_preset(normalized, ThemeMode::Dark)
    }

    pub fn from_preset(name: &str, mode: ThemeMode) -> Option<Self> {
        let theme = PRESET_THEMES.get(name)?;
        Some(build_theme(theme, mode))
    }

    fn fallback_dark() -> Self {
        Self {
            text: Color::Rgb(255, 255, 255),
            text_muted: Color::Rgb(128, 128, 128),
            background: Color::Reset,
            background_panel: Color::Rgb(30, 30, 30),
            background_element: Color::Rgb(45, 45, 45),
            border: Color::Rgb(80, 80, 80),
            primary: Color::Rgb(100, 200, 255),
            secondary: Color::Rgb(180, 180, 255),
            success: Color::Rgb(80, 200, 120),
            warning: Color::Rgb(255, 200, 80),
            error: Color::Rgb(255, 80, 80),
            info: Color::Rgb(100, 180, 255),
            diff_added: Color::Rgb(80, 200, 120),
            diff_removed: Color::Rgb(255, 80, 80),
            agent_colors: vec![
                Color::Rgb(255, 100, 100),
                Color::Rgb(100, 255, 100),
                Color::Rgb(100, 100, 255),
                Color::Rgb(255, 255, 100),
                Color::Rgb(255, 100, 255),
                Color::Rgb(100, 255, 255),
                Color::Rgb(255, 180, 100),
                Color::Rgb(180, 100, 255),
            ],
            background_menu: Color::Rgb(50, 50, 55),
            border_active: Color::Rgb(100, 200, 255),
            border_subtle: Color::Rgb(60, 60, 60),
            markdown_heading: Color::Rgb(100, 200, 255),
            markdown_link: Color::Rgb(100, 180, 255),
            markdown_link_text: Color::Rgb(100, 255, 255),
            markdown_code: Color::Rgb(220, 220, 170),
            markdown_code_bg: Color::Rgb(40, 40, 40),
            markdown_block_quote: Color::Rgb(255, 200, 80),
            markdown_emph: Color::Rgb(255, 200, 80),
            markdown_strong: Color::Rgb(255, 255, 255),
            markdown_horizontal_rule: Color::Rgb(80, 80, 80),
            markdown_list_item: Color::Rgb(100, 180, 255),
            markdown_list_enumeration: Color::Rgb(100, 255, 255),
            markdown_image: Color::Rgb(100, 180, 255),
            markdown_image_text: Color::Rgb(100, 255, 255),
            markdown_code_block: Color::Rgb(255, 255, 255),
            syntax_comment: Color::Rgb(106, 153, 85),
            syntax_keyword: Color::Rgb(197, 134, 192),
            syntax_function: Color::Rgb(220, 220, 170),
            syntax_string: Color::Rgb(206, 145, 120),
            syntax_number: Color::Rgb(181, 206, 168),
            diff_added_bg: Color::Rgb(30, 60, 30),
            diff_removed_bg: Color::Rgb(60, 30, 30),
            diff_context_bg: Color::Rgb(40, 40, 40),
            tool_icon: Color::Rgb(180, 180, 255),
            tool_border: Color::Rgb(60, 60, 80),
        }
    }

    fn fallback_light() -> Self {
        Self {
            text: Color::Rgb(30, 30, 30),
            text_muted: Color::Rgb(128, 128, 128),
            background: Color::Rgb(255, 255, 255),
            background_panel: Color::Rgb(245, 245, 245),
            background_element: Color::Rgb(230, 230, 230),
            border: Color::Rgb(200, 200, 200),
            primary: Color::Rgb(0, 120, 200),
            secondary: Color::Rgb(100, 100, 200),
            success: Color::Rgb(40, 160, 80),
            warning: Color::Rgb(200, 160, 40),
            error: Color::Rgb(200, 50, 50),
            info: Color::Rgb(60, 140, 220),
            diff_added: Color::Rgb(40, 160, 80),
            diff_removed: Color::Rgb(200, 50, 50),
            agent_colors: vec![
                Color::Rgb(200, 60, 60),
                Color::Rgb(60, 160, 60),
                Color::Rgb(60, 60, 200),
                Color::Rgb(200, 180, 60),
                Color::Rgb(200, 60, 180),
                Color::Rgb(60, 180, 180),
                Color::Rgb(200, 140, 60),
                Color::Rgb(140, 60, 200),
            ],
            background_menu: Color::Rgb(220, 220, 225),
            border_active: Color::Rgb(0, 120, 200),
            border_subtle: Color::Rgb(210, 210, 210),
            markdown_heading: Color::Rgb(0, 120, 200),
            markdown_link: Color::Rgb(60, 140, 220),
            markdown_link_text: Color::Rgb(0, 140, 170),
            markdown_code: Color::Rgb(100, 100, 100),
            markdown_code_bg: Color::Rgb(240, 240, 240),
            markdown_block_quote: Color::Rgb(200, 160, 40),
            markdown_emph: Color::Rgb(200, 160, 40),
            markdown_strong: Color::Rgb(30, 30, 30),
            markdown_horizontal_rule: Color::Rgb(200, 200, 200),
            markdown_list_item: Color::Rgb(60, 140, 220),
            markdown_list_enumeration: Color::Rgb(0, 140, 170),
            markdown_image: Color::Rgb(60, 140, 220),
            markdown_image_text: Color::Rgb(0, 140, 170),
            markdown_code_block: Color::Rgb(30, 30, 30),
            syntax_comment: Color::Rgb(0, 128, 0),
            syntax_keyword: Color::Rgb(175, 0, 219),
            syntax_function: Color::Rgb(121, 94, 38),
            syntax_string: Color::Rgb(163, 21, 21),
            syntax_number: Color::Rgb(9, 134, 88),
            diff_added_bg: Color::Rgb(220, 255, 220),
            diff_removed_bg: Color::Rgb(255, 220, 220),
            diff_context_bg: Color::Rgb(245, 245, 245),
            tool_icon: Color::Rgb(80, 80, 160),
            tool_border: Color::Rgb(180, 180, 200),
        }
    }
}

fn build_theme(file: &ThemeFile, mode: ThemeMode) -> Theme {
    let mut stack = Vec::new();
    let primary = resolve_theme_color(file, "primary", mode, &mut stack).unwrap_or(Color::Cyan);
    let secondary = resolve_theme_color(file, "secondary", mode, &mut stack).unwrap_or(Color::Blue);
    let info = resolve_theme_color(file, "info", mode, &mut stack).unwrap_or(primary);
    let success = resolve_theme_color(file, "success", mode, &mut stack).unwrap_or(Color::Green);
    let warning = resolve_theme_color(file, "warning", mode, &mut stack).unwrap_or(Color::Yellow);
    let error = resolve_theme_color(file, "error", mode, &mut stack).unwrap_or(Color::Red);

    Theme {
        text: resolve_theme_color(file, "text", mode, &mut stack).unwrap_or(Color::White),
        text_muted: resolve_theme_color(file, "textMuted", mode, &mut stack)
            .unwrap_or(Color::DarkGray),
        background: resolve_theme_color(file, "background", mode, &mut stack)
            .unwrap_or(Color::Reset),
        background_panel: resolve_theme_color(file, "backgroundPanel", mode, &mut stack)
            .unwrap_or(Color::Black),
        background_element: resolve_theme_color(file, "backgroundElement", mode, &mut stack)
            .unwrap_or(Color::DarkGray),
        border: resolve_theme_color(file, "border", mode, &mut stack).unwrap_or(Color::Gray),
        primary,
        secondary,
        success,
        warning,
        error,
        info,
        diff_added: resolve_theme_color(file, "diffAdded", mode, &mut stack).unwrap_or(success),
        diff_removed: resolve_theme_color(file, "diffRemoved", mode, &mut stack).unwrap_or(error),
        agent_colors: vec![
            primary,
            secondary,
            info,
            success,
            warning,
            error,
            resolve_theme_color(file, "accent", mode, &mut stack).unwrap_or(secondary),
            resolve_theme_color(file, "markdownLink", mode, &mut stack).unwrap_or(primary),
        ],
        background_menu: resolve_theme_color(file, "backgroundMenu", mode, &mut stack).unwrap_or(
            resolve_theme_color(file, "backgroundElement", mode, &mut stack).unwrap_or(
                if mode == ThemeMode::Dark {
                    Color::Rgb(50, 50, 55)
                } else {
                    Color::Rgb(220, 220, 225)
                },
            ),
        ),
        border_active: resolve_theme_color(file, "borderActive", mode, &mut stack)
            .unwrap_or(primary),
        border_subtle: resolve_theme_color(file, "borderSubtle", mode, &mut stack).unwrap_or(
            resolve_theme_color(file, "border", mode, &mut stack).unwrap_or(Color::Gray),
        ),
        markdown_heading: resolve_theme_color(file, "markdownHeading", mode, &mut stack)
            .unwrap_or(primary),
        markdown_link: resolve_theme_color(file, "markdownLink", mode, &mut stack).unwrap_or(info),
        markdown_link_text: resolve_theme_color(file, "markdownLinkText", mode, &mut stack)
            .unwrap_or(info),
        markdown_code: resolve_theme_color(file, "markdownCode", mode, &mut stack).unwrap_or(
            if mode == ThemeMode::Dark {
                Color::Rgb(220, 220, 170)
            } else {
                Color::Rgb(100, 100, 100)
            },
        ),
        markdown_code_bg: resolve_theme_color(file, "markdownCodeBg", mode, &mut stack).unwrap_or(
            if mode == ThemeMode::Dark {
                Color::Rgb(40, 40, 40)
            } else {
                Color::Rgb(240, 240, 240)
            },
        ),
        markdown_block_quote: resolve_theme_color(file, "markdownBlockQuote", mode, &mut stack)
            .unwrap_or(warning),
        markdown_emph: resolve_theme_color(file, "markdownEmph", mode, &mut stack)
            .unwrap_or(warning),
        markdown_strong: resolve_theme_color(file, "markdownStrong", mode, &mut stack).unwrap_or(
            resolve_theme_color(file, "text", mode, &mut stack).unwrap_or(
                if mode == ThemeMode::Dark {
                    Color::White
                } else {
                    Color::Black
                },
            ),
        ),
        markdown_horizontal_rule: resolve_theme_color(
            file,
            "markdownHorizontalRule",
            mode,
            &mut stack,
        )
        .unwrap_or(resolve_theme_color(file, "border", mode, &mut stack).unwrap_or(Color::Gray)),
        markdown_list_item: resolve_theme_color(file, "markdownListItem", mode, &mut stack)
            .unwrap_or(primary),
        markdown_list_enumeration: resolve_theme_color(
            file,
            "markdownListEnumeration",
            mode,
            &mut stack,
        )
        .unwrap_or(info),
        markdown_image: resolve_theme_color(file, "markdownImage", mode, &mut stack)
            .unwrap_or(primary),
        markdown_image_text: resolve_theme_color(file, "markdownImageText", mode, &mut stack)
            .unwrap_or(info),
        markdown_code_block: resolve_theme_color(file, "markdownCodeBlock", mode, &mut stack)
            .unwrap_or(
                resolve_theme_color(file, "text", mode, &mut stack).unwrap_or(
                    if mode == ThemeMode::Dark {
                        Color::White
                    } else {
                        Color::Black
                    },
                ),
            ),
        syntax_comment: resolve_theme_color(file, "syntaxComment", mode, &mut stack).unwrap_or(
            if mode == ThemeMode::Dark {
                Color::Rgb(106, 153, 85)
            } else {
                Color::Rgb(0, 128, 0)
            },
        ),
        syntax_keyword: resolve_theme_color(file, "syntaxKeyword", mode, &mut stack).unwrap_or(
            if mode == ThemeMode::Dark {
                Color::Rgb(197, 134, 192)
            } else {
                Color::Rgb(175, 0, 219)
            },
        ),
        syntax_function: resolve_theme_color(file, "syntaxFunction", mode, &mut stack).unwrap_or(
            if mode == ThemeMode::Dark {
                Color::Rgb(220, 220, 170)
            } else {
                Color::Rgb(121, 94, 38)
            },
        ),
        syntax_string: resolve_theme_color(file, "syntaxString", mode, &mut stack).unwrap_or(
            if mode == ThemeMode::Dark {
                Color::Rgb(206, 145, 120)
            } else {
                Color::Rgb(163, 21, 21)
            },
        ),
        syntax_number: resolve_theme_color(file, "syntaxNumber", mode, &mut stack).unwrap_or(
            if mode == ThemeMode::Dark {
                Color::Rgb(181, 206, 168)
            } else {
                Color::Rgb(9, 134, 88)
            },
        ),
        diff_added_bg: resolve_theme_color(file, "diffAddedBg", mode, &mut stack).unwrap_or(
            if mode == ThemeMode::Dark {
                Color::Rgb(30, 60, 30)
            } else {
                Color::Rgb(220, 255, 220)
            },
        ),
        diff_removed_bg: resolve_theme_color(file, "diffRemovedBg", mode, &mut stack).unwrap_or(
            if mode == ThemeMode::Dark {
                Color::Rgb(60, 30, 30)
            } else {
                Color::Rgb(255, 220, 220)
            },
        ),
        diff_context_bg: resolve_theme_color(file, "diffContextBg", mode, &mut stack).unwrap_or(
            if mode == ThemeMode::Dark {
                Color::Rgb(40, 40, 40)
            } else {
                Color::Rgb(245, 245, 245)
            },
        ),
        tool_icon: resolve_theme_color(file, "toolIcon", mode, &mut stack).unwrap_or(secondary),
        tool_border: resolve_theme_color(file, "toolBorder", mode, &mut stack).unwrap_or(
            resolve_theme_color(file, "border", mode, &mut stack).unwrap_or(Color::Gray),
        ),
    }
}

fn resolve_theme_color(
    file: &ThemeFile,
    key: &str,
    mode: ThemeMode,
    stack: &mut Vec<String>,
) -> Option<Color> {
    let value = file.theme.get(key)?;
    resolve_color_value(file, value, mode, stack)
}

fn resolve_color_value(
    file: &ThemeFile,
    value: &Value,
    mode: ThemeMode,
    stack: &mut Vec<String>,
) -> Option<Color> {
    match value {
        Value::String(raw) => resolve_color_string(file, raw, mode, stack),
        Value::Number(number) => number.as_u64().map(ansi_to_color),
        Value::Object(map) => {
            let key = match mode {
                ThemeMode::Dark => "dark",
                ThemeMode::Light => "light",
            };
            map.get(key)
                .and_then(|v| resolve_color_value(file, v, mode, stack))
        }
        _ => None,
    }
}

fn resolve_color_string(
    file: &ThemeFile,
    value: &str,
    mode: ThemeMode,
    stack: &mut Vec<String>,
) -> Option<Color> {
    let trimmed = value.trim();
    if trimmed.eq_ignore_ascii_case("transparent")
        || trimmed.eq_ignore_ascii_case("none")
        || trimmed.starts_with('#')
    {
        return parse_color_literal(trimmed);
    }

    if let Some(v) = file.defs.get(trimmed) {
        if stack.iter().any(|seen| seen == trimmed) {
            return None;
        }
        stack.push(trimmed.to_string());
        let resolved = resolve_color_value(file, v, mode, stack);
        stack.pop();
        return resolved;
    }

    if let Some(v) = file.theme.get(trimmed) {
        if stack.iter().any(|seen| seen == trimmed) {
            return None;
        }
        stack.push(trimmed.to_string());
        let resolved = resolve_color_value(file, v, mode, stack);
        stack.pop();
        return resolved;
    }

    parse_color_literal(trimmed)
}

fn parse_color_literal(value: &str) -> Option<Color> {
    let lowered = value.trim().to_ascii_lowercase();
    if lowered == "transparent" || lowered == "none" {
        return Some(Color::Reset);
    }

    let hex = lowered.strip_prefix('#')?;
    match hex.len() {
        3 => {
            let r = parse_hex_nibble(hex.as_bytes()[0])?;
            let g = parse_hex_nibble(hex.as_bytes()[1])?;
            let b = parse_hex_nibble(hex.as_bytes()[2])?;
            Some(Color::Rgb(r * 17, g * 17, b * 17))
        }
        4 => {
            let r = parse_hex_nibble(hex.as_bytes()[0])?;
            let g = parse_hex_nibble(hex.as_bytes()[1])?;
            let b = parse_hex_nibble(hex.as_bytes()[2])?;
            let a = parse_hex_nibble(hex.as_bytes()[3])?;
            if a == 0 {
                Some(Color::Reset)
            } else {
                Some(Color::Rgb(r * 17, g * 17, b * 17))
            }
        }
        6 => {
            let bytes = hex.as_bytes();
            let r = parse_hex_byte(bytes[0], bytes[1])?;
            let g = parse_hex_byte(bytes[2], bytes[3])?;
            let b = parse_hex_byte(bytes[4], bytes[5])?;
            Some(Color::Rgb(r, g, b))
        }
        8 => {
            let bytes = hex.as_bytes();
            let r = parse_hex_byte(bytes[0], bytes[1])?;
            let g = parse_hex_byte(bytes[2], bytes[3])?;
            let b = parse_hex_byte(bytes[4], bytes[5])?;
            let a = parse_hex_byte(bytes[6], bytes[7])?;
            if a == 0 {
                Some(Color::Reset)
            } else {
                Some(Color::Rgb(r, g, b))
            }
        }
        _ => None,
    }
}

fn parse_hex_nibble(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(10 + c - b'a'),
        _ => None,
    }
}

fn parse_hex_byte(a: u8, b: u8) -> Option<u8> {
    let high = parse_hex_nibble(a)?;
    let low = parse_hex_nibble(b)?;
    Some((high << 4) | low)
}

fn ansi_to_color(code: u64) -> Color {
    if code < 16 {
        return match code {
            0 => Color::Rgb(0, 0, 0),
            1 => Color::Rgb(128, 0, 0),
            2 => Color::Rgb(0, 128, 0),
            3 => Color::Rgb(128, 128, 0),
            4 => Color::Rgb(0, 0, 128),
            5 => Color::Rgb(128, 0, 128),
            6 => Color::Rgb(0, 128, 128),
            7 => Color::Rgb(192, 192, 192),
            8 => Color::Rgb(128, 128, 128),
            9 => Color::Rgb(255, 0, 0),
            10 => Color::Rgb(0, 255, 0),
            11 => Color::Rgb(255, 255, 0),
            12 => Color::Rgb(0, 0, 255),
            13 => Color::Rgb(255, 0, 255),
            14 => Color::Rgb(0, 255, 255),
            _ => Color::Rgb(255, 255, 255),
        };
    }

    if code < 232 {
        let index = code - 16;
        let b = index % 6;
        let g = (index / 6) % 6;
        let r = index / 36;
        let convert = |v: u64| if v == 0 { 0 } else { (v * 40 + 55) as u8 };
        return Color::Rgb(convert(r), convert(g), convert(b));
    }

    if code < 256 {
        let gray = ((code - 232) * 10 + 8) as u8;
        return Color::Rgb(gray, gray, gray);
    }

    Color::Reset
}

pub struct Styles;

impl Styles {
    pub fn title() -> Style {
        Style::default().add_modifier(Modifier::BOLD)
    }

    pub fn muted() -> Style {
        Style::default().fg(Color::Rgb(128, 128, 128))
    }

    pub fn success() -> Style {
        Style::default().fg(Color::Rgb(80, 200, 120))
    }

    pub fn error() -> Style {
        Style::default().fg(Color::Rgb(255, 80, 80))
    }

    pub fn warning() -> Style {
        Style::default().fg(Color::Rgb(255, 200, 80))
    }

    pub fn selected() -> Style {
        Style::default().add_modifier(Modifier::REVERSED)
    }
}
