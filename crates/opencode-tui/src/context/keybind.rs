use crossterm::event::{KeyCode, KeyModifiers};
use std::collections::HashMap;
use std::time::{Duration, Instant};

const LEADER_TIMEOUT_MS: u64 = 2000;

pub struct LeaderKeyState {
    pub active: bool,
    pub start_time: Option<Instant>,
    pub leader_key: Option<KeyCode>,
}

impl LeaderKeyState {
    pub fn new() -> Self {
        Self {
            active: false,
            start_time: None,
            leader_key: None,
        }
    }

    pub fn start(&mut self, key: KeyCode) {
        self.active = true;
        self.start_time = Some(Instant::now());
        self.leader_key = Some(key);
    }

    pub fn check_timeout(&mut self) -> bool {
        if let Some(start) = self.start_time {
            if start.elapsed() > Duration::from_millis(LEADER_TIMEOUT_MS) {
                self.reset();
                return true;
            }
        }
        false
    }

    pub fn reset(&mut self) {
        self.active = false;
        self.start_time = None;
        self.leader_key = None;
    }
}

impl Default for LeaderKeyState {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Keybind {
    pub code: KeyCode,
    pub modifiers: KeyModifiers,
}

impl Keybind {
    pub fn new(code: KeyCode, modifiers: KeyModifiers) -> Self {
        Self { code, modifiers }
    }

    pub fn key(code: KeyCode) -> Self {
        Self {
            code,
            modifiers: KeyModifiers::empty(),
        }
    }

    pub fn ctrl(code: KeyCode) -> Self {
        Self {
            code,
            modifiers: KeyModifiers::CONTROL,
        }
    }

    pub fn alt(code: KeyCode) -> Self {
        Self {
            code,
            modifiers: KeyModifiers::ALT,
        }
    }

    pub fn shift(code: KeyCode) -> Self {
        Self {
            code,
            modifiers: KeyModifiers::SHIFT,
        }
    }

    pub fn ctrl_shift(code: KeyCode) -> Self {
        Self {
            code,
            modifiers: KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        }
    }
}

impl std::fmt::Display for Keybind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut parts = Vec::new();

        if self.modifiers.contains(KeyModifiers::CONTROL) {
            parts.push("ctrl");
        }
        if self.modifiers.contains(KeyModifiers::ALT) {
            parts.push("alt");
        }
        if self.modifiers.contains(KeyModifiers::SHIFT) {
            parts.push("shift");
        }

        let key_str = match self.code {
            KeyCode::Char(c) => c.to_string(),
            KeyCode::Enter => "enter".to_string(),
            KeyCode::Backspace => "backspace".to_string(),
            KeyCode::Esc => "esc".to_string(),
            KeyCode::Tab => "tab".to_string(),
            KeyCode::BackTab => "shift+tab".to_string(),
            KeyCode::Delete => "del".to_string(),
            KeyCode::Home => "home".to_string(),
            KeyCode::End => "end".to_string(),
            KeyCode::PageUp => "pgup".to_string(),
            KeyCode::PageDown => "pgdn".to_string(),
            KeyCode::Up => "up".to_string(),
            KeyCode::Down => "down".to_string(),
            KeyCode::Left => "left".to_string(),
            KeyCode::Right => "right".to_string(),
            KeyCode::F(n) => format!("f{}", n),
            _ => "?".to_string(),
        };
        parts.push(&key_str);

        write!(f, "{}", parts.join("+"))
    }
}

pub struct KeybindRegistry {
    bindings: HashMap<String, Keybind>,
}

impl Default for KeybindRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl KeybindRegistry {
    pub fn new() -> Self {
        let mut registry = Self {
            bindings: HashMap::new(),
        };
        registry.register_defaults();
        registry
    }

    fn register_defaults(&mut self) {
        self.register("app_exit", Keybind::ctrl(KeyCode::Char('c')));
        self.register("app_exit_alt", Keybind::key(KeyCode::Esc));

        self.register("input_submit", Keybind::key(KeyCode::Enter));
        self.register("input_newline", Keybind::alt(KeyCode::Enter));
        self.register("input_clear", Keybind::ctrl(KeyCode::Char('u')));
        self.register("input_paste", Keybind::ctrl(KeyCode::Char('v')));
        self.register("input_copy", Keybind::ctrl_shift(KeyCode::Char('c')));
        self.register("input_cut", Keybind::ctrl_shift(KeyCode::Char('x')));

        self.register("history_previous", Keybind::alt(KeyCode::Up));
        self.register("history_next", Keybind::alt(KeyCode::Down));

        self.register("command_list", Keybind::ctrl(KeyCode::Char('x')));
        self.register("command_palette", Keybind::ctrl(KeyCode::Char('p')));

        self.register("agent_cycle", Keybind::key(KeyCode::Tab));
        self.register("agent_cycle_reverse", Keybind::key(KeyCode::BackTab));
        self.register("model_cycle", Keybind::ctrl(KeyCode::Char('m')));
        self.register("variant_cycle", Keybind::ctrl(KeyCode::Char('v')));

        self.register("session_parent", Keybind::ctrl(KeyCode::Char('o')));
        self.register("session_child_cycle", Keybind::ctrl(KeyCode::Char('j')));
        self.register(
            "session_child_cycle_reverse",
            Keybind::ctrl(KeyCode::Char('k')),
        );
        self.register("session_rename", Keybind::ctrl(KeyCode::Char('r')));
        self.register("session_delete", Keybind::ctrl(KeyCode::Char('d')));
        self.register("session_interrupt", Keybind::key(KeyCode::Esc));

        self.register("sidebar_toggle", Keybind::ctrl(KeyCode::Char('s')));
        self.register("help_toggle", Keybind::ctrl(KeyCode::Char('h')));

        self.register("editor_open", Keybind::ctrl(KeyCode::Char('e')));

        self.register("cursor_up", Keybind::key(KeyCode::Up));
        self.register("cursor_down", Keybind::key(KeyCode::Down));
        self.register("cursor_left", Keybind::key(KeyCode::Left));
        self.register("cursor_right", Keybind::key(KeyCode::Right));

        self.register("page_up", Keybind::key(KeyCode::PageUp));
        self.register("page_down", Keybind::key(KeyCode::PageDown));
        self.register("home", Keybind::key(KeyCode::Home));
        self.register("end", Keybind::key(KeyCode::End));
    }

    pub fn register(&mut self, name: &str, keybind: Keybind) {
        self.bindings.insert(name.to_string(), keybind);
    }

    pub fn get(&self, name: &str) -> Option<&Keybind> {
        self.bindings.get(name)
    }

    pub fn match_key(&self, name: &str, code: KeyCode, modifiers: KeyModifiers) -> bool {
        let Some(kb) = self.bindings.get(name) else {
            return false;
        };

        if kb.code == code && kb.modifiers == modifiers {
            return true;
        }

        // Terminal implementations can report Shift+Tab as either:
        // - KeyCode::BackTab with no modifiers
        // - KeyCode::Tab with SHIFT modifier
        if kb.code == KeyCode::BackTab {
            match (code, modifiers) {
                (KeyCode::BackTab, mods) if mods.is_empty() || mods == KeyModifiers::SHIFT => {
                    return true;
                }
                (KeyCode::Tab, mods) if mods.contains(KeyModifiers::SHIFT) => {
                    return true;
                }
                _ => {}
            }
        }

        false
    }

    pub fn print(&self, name: &str) -> String {
        self.bindings
            .get(name)
            .map(|kb| kb.to_string())
            .unwrap_or_else(|| "?".to_string())
    }

    pub fn all(&self) -> &HashMap<String, Keybind> {
        &self.bindings
    }
}
