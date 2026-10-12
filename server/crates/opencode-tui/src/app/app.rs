use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use chrono::{TimeZone, Utc};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::api::{ApiClient, ApiMessage, McpStatusInfo, SessionInfo, SessionRevertInfo, ToolRank};
use crate::app::state::AppState;
use crate::app::terminal;
use crate::command::CommandAction;
use crate::components::{
    Agent, AgentSelectDialog, AlertDialog, CommandPalette, ForkDialog, ForkEntry, HelpDialog,
    HomeView, McpDialog, McpItem, Model, ModelSelectDialog, PermissionAction, PermissionPrompt,
    Prompt, PromptStashDialog, ProviderDialog, QuestionPrompt,
    SessionDeleteState, SessionExportDialog, SessionItem, SessionListDialog, SessionRenameDialog,
    SessionView, SkillListDialog, SlashCommandPopup, StashItem, StatusDialog, StatusLine,
    SubagentDialog, TagDialog, TaskKind, ThemeListDialog, ThemeOption, TimelineDialog,
    TimelineEntry, Toast, ToastVariant,
};
use crate::context::keybind::LeaderKeyState;
use crate::context::{
    AppContext, McpConnectionStatus, McpServerStatus, Message, MessagePart as ContextMessagePart,
    MessageRole, RevertInfo, Session, SessionStatus, TokenUsage,
};
use crate::event::{CustomEvent, Event, StateChange};
use crate::router::Route;
use crate::ui::{Clipboard, Selection};

// TS parity: renderer targetFps is 60, ~16ms frame budget.
const TICK_RATE_MS: u64 = 16;
const MAX_EVENTS_PER_FRAME: usize = 256;

pub struct App {
    context: Arc<AppContext>,
    state: AppState,
    /// `None` when the app runs headless (see [`App::new_for_test`]).
    terminal: Option<terminal::Tui>,
    event_rx: Receiver<Event>,
    prompt: Prompt,
    selection: Selection,
    session_view: Option<SessionView>,
    active_session_id: Option<String>,
    command_palette: CommandPalette,
    slash_popup: SlashCommandPopup,
    leader_state: LeaderKeyState,
    model_select: ModelSelectDialog,
    agent_select: AgentSelectDialog,
    alert_dialog: AlertDialog,
    help_dialog: HelpDialog,
    session_list_dialog: SessionListDialog,
    session_rename_dialog: SessionRenameDialog,
    session_export_dialog: SessionExportDialog,
    prompt_stash_dialog: PromptStashDialog,
    skill_list_dialog: SkillListDialog,
    theme_list_dialog: ThemeListDialog,
    status_dialog: StatusDialog,
    mcp_dialog: McpDialog,
    timeline_dialog: TimelineDialog,
    fork_dialog: ForkDialog,
    provider_dialog: ProviderDialog,
    subagent_dialog: SubagentDialog,
    tag_dialog: TagDialog,
    permission_prompt: PermissionPrompt,
    question_prompt: QuestionPrompt,
    toast: Toast,
    /// Snapshot of rendered screen lines for text selection copy.
    screen_lines: Vec<String>,
    available_models: HashSet<String>,
    model_variants: HashMap<String, Vec<String>>,
    model_variant_selection: HashMap<String, Option<String>>,
    pending_initial_submit: bool,
    pending_session_sync: Option<String>,
    last_session_sync: Instant,
    last_aux_sync: Instant,
    event_caused_change: bool,
}

/// Background services an `App` needs: a real terminal starts crossterm input
/// and the server event listener, a headless one starts nothing.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Runtime {
    Live,
    Headless,
}

impl App {
    pub fn new() -> anyhow::Result<Self> {
        Self::build(Some(terminal::init()?), Runtime::Live)
    }

    /// Build an `App` that never touches the real terminal: no raw mode, no
    /// alternate screen, no input thread, no server event listener, and no API
    /// client (so the `refresh_*` calls below short-circuit instead of doing
    /// 30s-timeout HTTP requests). Meant for `cargo test`.
    #[doc(hidden)]
    pub fn new_for_test() -> anyhow::Result<Self> {
        Self::build(None, Runtime::Headless)
    }

    pub fn has_terminal(&self) -> bool {
        self.terminal.is_some()
    }

    /// Switch the rendered route. Lets a headless test drive the Session view,
    /// which is the only route that paints the sidebar.
    pub fn navigate(&mut self, route: Route) {
        self.context.navigate(route);
    }

    fn build(tui: Option<terminal::Tui>, runtime: Runtime) -> anyhow::Result<Self> {
        let (event_tx, event_rx) = mpsc::channel();
        let event_tx_input = event_tx.clone();
        let context = Arc::new(AppContext::new());
        let mut prompt = Prompt::new(context.clone())
            .with_placeholder("Ask anything... \"Fix a TODO in the codebase\"");
        let mut pending_initial_submit = false;
        let mut initial_session_id: Option<String> = None;

        if let Ok(dir) = std::env::current_dir() {
            *context.directory.write() = dir.display().to_string();
        }

        if runtime == Runtime::Live {
            let base_url = resolve_tui_base_url();
            let api_client = Arc::new(ApiClient::new(base_url.clone()));
            context.set_api_client(api_client);
            spawn_server_event_listener(event_tx.clone(), base_url, context.clone());
        }

        if let Ok(agent) = std::env::var("OPENCODE_TUI_AGENT") {
            let agent = agent.trim();
            if !agent.is_empty() {
                context.set_agent(agent.to_string());
            }
        }
        if let Ok(model) = std::env::var("OPENCODE_TUI_MODEL") {
            let model = model.trim();
            if !model.is_empty() {
                context.set_model_selection(model.to_string(), provider_from_model(model));
                context.set_model_variant(None);
            }
        }
        if let Ok(session_id) = std::env::var("OPENCODE_TUI_SESSION") {
            let session_id = session_id.trim();
            if !session_id.is_empty() {
                initial_session_id = Some(session_id.to_string());
                context.navigate(Route::Session {
                    session_id: session_id.to_string(),
                });
            }
        }
        if let Ok(initial_prompt) = std::env::var("OPENCODE_TUI_PROMPT") {
            let initial_prompt = initial_prompt.trim();
            if !initial_prompt.is_empty() {
                prompt.set_input(initial_prompt.to_string());
                pending_initial_submit = true;
            }
        }
        {
            let theme = context.theme.read().clone();
            let agent = context.current_agent.read().clone();
            prompt.set_spinner_color(agent_color_from_name(&theme, &agent));
        }

        let tick_rate = Duration::from_millis(TICK_RATE_MS);
        let attach_input = runtime == Runtime::Live;
        thread::spawn(move || {
            if !attach_input {
                return;
            }

            let mut last_tick = Instant::now();

            loop {
                let timeout = tick_rate
                    .checked_sub(last_tick.elapsed())
                    .unwrap_or(tick_rate);

                if crossterm::event::poll(timeout).unwrap_or(false) {
                    let event = match crossterm::event::read() {
                        Ok(crossterm::event::Event::Key(key)) => Some(Event::Key(key)),
                        Ok(crossterm::event::Event::Mouse(mouse))
                            if !matches!(mouse.kind, crossterm::event::MouseEventKind::Moved) =>
                        {
                            Some(Event::Mouse(mouse))
                        }
                        Ok(crossterm::event::Event::Resize(w, h)) => Some(Event::Resize(w, h)),
                        Ok(crossterm::event::Event::FocusGained) => Some(Event::FocusGained),
                        Ok(crossterm::event::Event::FocusLost) => Some(Event::FocusLost),
                        Ok(crossterm::event::Event::Paste(s)) => Some(Event::Paste(s)),
                        _ => None,
                    };

                    if let Some(e) = event {
                        if event_tx_input.send(e).is_err() {
                            break;
                        }
                    }
                }

                if last_tick.elapsed() >= tick_rate {
                    if event_tx_input.send(Event::Tick).is_err() {
                        break;
                    }
                    last_tick = Instant::now();
                }
            }
        });

        let mut app = Self {
            context,
            state: AppState::default(),
            terminal: tui,
            event_rx,
            prompt,
            selection: Selection::new(),
            session_view: None,
            active_session_id: None,
            command_palette: CommandPalette::new(),
            slash_popup: SlashCommandPopup::new(),
            leader_state: LeaderKeyState::new(),
            model_select: ModelSelectDialog::new(),
            agent_select: AgentSelectDialog::new(),
            alert_dialog: AlertDialog::info(""),
            help_dialog: HelpDialog::new(),
            session_list_dialog: SessionListDialog::new(),
            session_rename_dialog: SessionRenameDialog::new(),
            session_export_dialog: SessionExportDialog::new(),
            prompt_stash_dialog: PromptStashDialog::new(),
            skill_list_dialog: SkillListDialog::new(),
            theme_list_dialog: ThemeListDialog::new(),
            status_dialog: StatusDialog::new(),
            mcp_dialog: McpDialog::new(),
            timeline_dialog: TimelineDialog::new(),
            fork_dialog: ForkDialog::new(),
            provider_dialog: ProviderDialog::new(),
            subagent_dialog: SubagentDialog::new(),
            tag_dialog: TagDialog::new(),
            permission_prompt: PermissionPrompt::new(),
            question_prompt: QuestionPrompt::new(),
            toast: Toast::new(),
            screen_lines: Vec::new(),
            available_models: HashSet::new(),
            model_variants: HashMap::new(),
            model_variant_selection: HashMap::new(),
            pending_initial_submit,
            pending_session_sync: None,
            last_session_sync: Instant::now(),
            last_aux_sync: Instant::now(),
            event_caused_change: true,
        };

        app.refresh_model_dialog();
        app.refresh_agent_dialog();
        let _ = app.refresh_skill_list_dialog();
        app.refresh_session_list_dialog();
        app.refresh_theme_list_dialog();
        let _ = app.refresh_lsp_status();
        let _ = app.refresh_mcp_dialog();

        if let Some(session_id) = initial_session_id {
            let _ = app.sync_session_from_server(&session_id);
            app.ensure_session_view(&session_id);
        }
        app.sync_prompt_spinner_style();
        app.sync_prompt_spinner_state();

        Ok(app)
    }

    pub fn run(&mut self) -> anyhow::Result<()> {
        self.draw()?;

        while self.state != AppState::Exiting {
            let mut should_draw = false;

            let first_event = match self
                .event_rx
                .recv_timeout(Duration::from_millis(TICK_RATE_MS))
            {
                Ok(event) => Some(event),
                Err(mpsc::RecvTimeoutError::Timeout) => None,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            };

            if let Some(event) = first_event {
                self.handle_event(&event)?;
                should_draw |= self.event_caused_change;

                let mut deferred_mouse_move: Option<Event> = None;
                for _ in 0..MAX_EVENTS_PER_FRAME {
                    let next = match self.event_rx.try_recv() {
                        Ok(next) => next,
                        Err(mpsc::TryRecvError::Empty) => break,
                        Err(mpsc::TryRecvError::Disconnected) => {
                            self.state = AppState::Exiting;
                            break;
                        }
                    };

                    let is_mouse_move = matches!(
                        next,
                        Event::Mouse(crossterm::event::MouseEvent {
                            kind: crossterm::event::MouseEventKind::Moved,
                            ..
                        })
                    );

                    if is_mouse_move {
                        deferred_mouse_move = Some(next);
                        continue;
                    }

                    if let Some(moved) = deferred_mouse_move.take() {
                        self.handle_event(&moved)?;
                        should_draw |= self.event_caused_change;
                    }

                    self.handle_event(&next)?;
                    should_draw |= self.event_caused_change;
                }

                if let Some(moved) = deferred_mouse_move {
                    self.handle_event(&moved)?;
                    should_draw |= self.event_caused_change;
                }
            }

            if should_draw {
                self.draw()?;
            }
        }

        terminal::restore()?;
        Ok(())
    }

    fn handle_event(&mut self, event: &Event) -> anyhow::Result<()> {
        self.event_caused_change = true;

        match event {
            Event::Key(key) => {
                // Handle inline permission prompt before dialogs
                if self.permission_prompt.is_open {
                    match key.code {
                        KeyCode::Char('y') | KeyCode::Enter => {
                            if let Some(_request) = self.permission_prompt.approve() {
                                // TODO: Call API when available
                                // self.api_client.reply_permission(&request.id, "once");
                            }
                        }
                        KeyCode::Char('n') => {
                            let _ = self.permission_prompt.deny();
                        }
                        KeyCode::Char('a') => {
                            let _ = self.permission_prompt.approve_always();
                        }
                        KeyCode::Esc => {
                            self.permission_prompt.deny();
                        }
                        _ => {}
                    }
                    return Ok(());
                }

                // Handle inline question prompt before dialogs
                if self.question_prompt.is_open {
                    match key.code {
                        KeyCode::Up => self.question_prompt.move_up(),
                        KeyCode::Down => self.question_prompt.move_down(),
                        KeyCode::Char(' ') => self.question_prompt.toggle_selected(),
                        KeyCode::Enter => {
                            if let Some((_question, _answer)) = self.question_prompt.confirm() {
                                // TODO: Call API when available
                            }
                        }
                        KeyCode::Esc => {
                            self.question_prompt.close();
                        }
                        KeyCode::Char(c) => self.question_prompt.type_char(c),
                        KeyCode::Backspace => self.question_prompt.backspace(),
                        _ => {}
                    }
                    return Ok(());
                }

                if self.handle_dialog_key(*key)? {
                    return Ok(());
                }

                // Leader key handling
                if self.leader_state.active {
                    if self.leader_state.check_timeout() {
                        // Leader timed out, fall through to normal handling
                    } else {
                        let action = match key.code {
                            KeyCode::Char('n') => Some(CommandAction::NewSession),
                            KeyCode::Char('l') => Some(CommandAction::SwitchSession),
                            KeyCode::Char('m') => Some(CommandAction::SwitchModel),
                            KeyCode::Char('a') => Some(CommandAction::SwitchAgent),
                            KeyCode::Char('t') => Some(CommandAction::SwitchTheme),
                            KeyCode::Char('b') => Some(CommandAction::ToggleSidebar),
                            KeyCode::Char('s') => Some(CommandAction::ViewStatus),
                            KeyCode::Char('q') => Some(CommandAction::Exit),
                            KeyCode::Char('u') => Some(CommandAction::Undo),
                            KeyCode::Char('r') => Some(CommandAction::Redo),
                            _ => None,
                        };
                        self.leader_state.reset();
                        if let Some(action) = action {
                            self.execute_command_action(action)?;
                        }
                        return Ok(());
                    }
                }

                // Ctrl+X starts leader key sequence
                if key.code == KeyCode::Char('x') && key.modifiers == KeyModifiers::CONTROL {
                    self.leader_state.start(KeyCode::Char('x'));
                    return Ok(());
                }

                // Ctrl+Shift+C (crossterm reports uppercase 'C' with SHIFT modifier)
                if (key.code == KeyCode::Char('C') || key.code == KeyCode::Char('c'))
                    && key.modifiers.contains(KeyModifiers::CONTROL)
                    && key.modifiers.contains(KeyModifiers::SHIFT)
                {
                    self.copy_selection();
                    return Ok(());
                }

                if key.code == KeyCode::Char('c') && key.modifiers == KeyModifiers::CONTROL {
                    // If there's an active selection, copy it instead of exiting (TS parity)
                    if self.selection.is_active() {
                        self.copy_selection();
                        return Ok(());
                    }
                    self.state = AppState::Exiting;
                    return Ok(());
                }

                if key.code == KeyCode::Esc {
                    if self.selection.is_active() {
                        self.selection.clear();
                        return Ok(());
                    }
                }

                if key.code == KeyCode::Char('q') && key.modifiers.is_empty() {
                    self.state = AppState::Exiting;
                    return Ok(());
                }

                if self.matches_keybind("session_interrupt", *key) {
                    if self.prompt.is_shell_mode() {
                        self.prompt.exit_shell_mode();
                        self.prompt.clear_interrupt_confirmation();
                        return Ok(());
                    }
                    if let Route::Session { session_id } = self.context.current_route() {
                        let status = {
                            let session_ctx = self.context.session.read();
                            session_ctx.status(&session_id).clone()
                        };
                        if !matches!(status, SessionStatus::Idle) {
                            if !self.prompt.register_interrupt_keypress() {
                                return Ok(());
                            }
                            if let Some(client) = self.context.get_api_client() {
                                let _ = client.abort_session(&session_id);
                            }
                            self.prompt.clear_interrupt_confirmation();
                            self.set_session_status(&session_id, SessionStatus::Idle);
                            self.sync_prompt_spinner_state();
                            return Ok(());
                        }
                    }
                    self.prompt.clear_interrupt_confirmation();
                    return Ok(());
                }

                if self.matches_keybind("input_paste", *key) {
                    self.paste_clipboard_to_prompt();
                    return Ok(());
                }
                if self.matches_keybind("input_copy", *key) {
                    self.copy_prompt_to_clipboard();
                    return Ok(());
                }
                if self.matches_keybind("input_cut", *key) {
                    self.cut_prompt_to_clipboard();
                    return Ok(());
                }
                if self.matches_keybind("history_previous", *key) {
                    self.prompt.history_previous_entry();
                    return Ok(());
                }
                if self.matches_keybind("history_next", *key) {
                    self.prompt.history_next_entry();
                    return Ok(());
                }
                if self.matches_keybind("page_up", *key) {
                    if let Route::Session { .. } = self.context.current_route() {
                        if let Some(ref mut sv) = self.session_view {
                            sv.scroll_page_up();
                            return Ok(());
                        }
                    }
                }
                if self.matches_keybind("page_down", *key) {
                    if let Route::Session { .. } = self.context.current_route() {
                        if let Some(ref mut sv) = self.session_view {
                            sv.scroll_page_down();
                            return Ok(());
                        }
                    }
                }

                if self.matches_keybind("command_palette", *key) {
                    self.sync_command_palette_labels();
                    self.command_palette.open();
                    return Ok(());
                }
                if self.matches_keybind("model_cycle", *key) {
                    self.refresh_model_dialog();
                    self.model_select.open();
                    return Ok(());
                }
                if self.matches_keybind("agent_cycle", *key) {
                    self.cycle_agent(1);
                    return Ok(());
                }
                if self.matches_keybind("agent_cycle_reverse", *key) {
                    self.cycle_agent(-1);
                    return Ok(());
                }
                if self.matches_keybind("variant_cycle", *key) {
                    self.cycle_model_variant();
                    return Ok(());
                }
                if self.matches_keybind("sidebar_toggle", *key) {
                    self.context.toggle_sidebar();
                    return Ok(());
                }
                if self.matches_keybind("display_thinking", *key) {
                    self.context.toggle_thinking();
                    return Ok(());
                }
                if self.matches_keybind("tool_details", *key) {
                    self.context.toggle_tool_details();
                    return Ok(());
                }
                if self.matches_keybind("input_clear", *key) {
                    self.prompt.clear();
                    return Ok(());
                }
                if self.matches_keybind("input_newline", *key) {
                    let route = self.context.current_route();
                    if matches!(route, Route::Home | Route::Session { .. }) {
                        self.prompt.insert_text("\n");
                        return Ok(());
                    }
                }
                if self.matches_keybind("help_toggle", *key) {
                    self.help_dialog.open();
                    return Ok(());
                }

                // Slash command popup: open when '/' is typed at position 0
                if key.code == KeyCode::Char('/')
                    && key.modifiers.is_empty()
                    && self.prompt.cursor_position() == 0
                    && self.prompt.get_input().is_empty()
                {
                    self.slash_popup.open();
                    return Ok(());
                }

                let route = self.context.current_route();
                match route {
                    Route::Home | Route::Session { .. } => {
                        if key.code == KeyCode::Enter && key.modifiers.is_empty() {
                            self.submit_prompt()?;
                        } else if !key.modifiers.contains(KeyModifiers::CONTROL)
                            && !key.modifiers.contains(KeyModifiers::ALT)
                        {
                            self.prompt.handle_key(*key);
                        }
                    }
                    _ => {
                        if !key.modifiers.contains(KeyModifiers::CONTROL)
                            && !key.modifiers.contains(KeyModifiers::ALT)
                        {
                            self.prompt.handle_key(*key);
                        }
                    }
                }
            }
            Event::Resize(_, _) => {
                if let Some(tui) = self.terminal.as_mut() {
                    tui.autoresize()?;
                }
            }
            Event::Mouse(mouse_event) => {
                use crossterm::event::{MouseButton, MouseEventKind};
                match mouse_event.kind {
                    MouseEventKind::Down(button) => {
                        let col = mouse_event.column;
                        let row = mouse_event.row;

                        if button == MouseButton::Right {
                            // Right-click copies selection (if any) then clears it
                            if self.selection.is_active() {
                                self.copy_selection();
                            }
                            return Ok(());
                        }

                        if self.permission_prompt.is_open {
                            self.permission_prompt.handle_click(col, row);
                            if let Some(action) = self.permission_prompt.take_pending_action() {
                                match action {
                                    PermissionAction::Approve => {
                                        let _ = self.permission_prompt.approve();
                                    }
                                    PermissionAction::Deny => {
                                        let _ = self.permission_prompt.deny();
                                    }
                                    PermissionAction::ApproveAlways => {
                                        let _ = self.permission_prompt.approve_always();
                                    }
                                }
                            }
                            return Ok(());
                        }

                        // Question prompt click
                        if self.question_prompt.is_open {
                            self.question_prompt.handle_click(col, row);
                            return Ok(());
                        }

                        if self.handle_dialog_mouse(mouse_event)? {
                            return Ok(());
                        }

                        if button == MouseButton::Left {
                            if let Route::Session { .. } = self.context.current_route() {
                                if let Some(ref mut sv) = self.session_view {
                                    if sv.handle_sidebar_click(col, row) {
                                        return Ok(());
                                    }
                                    if sv.is_point_in_sidebar(col, row) {
                                        return Ok(());
                                    }
                                    if sv.handle_click(col, row) {
                                        return Ok(());
                                    }
                                }
                            }
                            // Clear previous selection and start a new one
                            self.selection.start(row, col);
                        }
                    }
                    MouseEventKind::ScrollUp => {
                        if self.handle_dialog_mouse(mouse_event)? {
                            return Ok(());
                        }
                        if let Some(ref mut sv) = self.session_view {
                            if !sv.scroll_sidebar_up_at(mouse_event.column, mouse_event.row) {
                                sv.scroll_up_mouse();
                            }
                        }
                    }
                    MouseEventKind::ScrollDown => {
                        if self.handle_dialog_mouse(mouse_event)? {
                            return Ok(());
                        }
                        if let Some(ref mut sv) = self.session_view {
                            if !sv.scroll_sidebar_down_at(mouse_event.column, mouse_event.row) {
                                sv.scroll_down_mouse();
                            }
                        }
                    }
                    MouseEventKind::Drag(_) => {
                        let col = mouse_event.column;
                        let row = mouse_event.row;
                        self.selection.update(row, col);
                    }
                    MouseEventKind::Moved => {
                        if self.handle_dialog_mouse(mouse_event)? {
                            return Ok(());
                        }
                        self.event_caused_change = false;
                    }
                    MouseEventKind::Up(_) => {
                        self.selection.finalize();
                    }
                    _ => {}
                }
            }
            Event::Paste(text) => {
                if !text.is_empty() {
                    self.prompt.insert_text(text);
                }
            }
            Event::Custom(event) => match event {
                CustomEvent::StateChanged(StateChange::SessionUpdated(session_id)) => {
                    if let Route::Session { session_id: active } = self.context.current_route() {
                        if active == *session_id {
                            if should_sync_session(
                                None,
                                session_id,
                                self.last_session_sync.elapsed(),
                            ) {
                                let succeeded = self.sync_session_from_server(session_id).is_ok();
                                self.pending_session_sync =
                                    apply_sync_outcome(self.pending_session_sync.take(), succeeded);
                            } else {
                                self.pending_session_sync = Some(session_id.to_string());
                            }
                        }
                    }
                    self.sync_prompt_spinner_state();
                }
                CustomEvent::StateChanged(StateChange::SessionStatusBusy(session_id)) => {
                    self.set_session_status(session_id, SessionStatus::Running);
                    self.sync_prompt_spinner_state();
                }
                CustomEvent::StateChanged(StateChange::SessionStatusIdle(session_id)) => {
                    self.set_session_status(session_id, SessionStatus::Idle);
                    self.sync_prompt_spinner_state();
                }
                CustomEvent::StateChanged(StateChange::SessionStatusRetrying {
                    session_id,
                    attempt,
                    message,
                    next,
                }) => {
                    self.set_session_status(
                        session_id,
                        SessionStatus::Retrying {
                            message: message.clone(),
                            attempt: *attempt,
                            next: *next,
                        },
                    );
                    self.sync_prompt_spinner_state();
                }
                _ => {}
            },
            Event::Tick => {
                let mut tick_changed = false;
                tick_changed |= self.toast.tick(TICK_RATE_MS);
                tick_changed |= self.prompt.tick_spinner(TICK_RATE_MS);
                tick_changed |= self.sync_prompt_spinner_state();

                if self.pending_initial_submit && !self.prompt.get_input().trim().is_empty() {
                    self.pending_initial_submit = false;
                    self.submit_prompt()?;
                    tick_changed = true;
                }

                let route = self.context.current_route();
                if let Route::Session { session_id } = route {
                    if self
                        .pending_session_sync
                        .as_deref()
                        .is_some_and(|pending| pending != session_id)
                    {
                        self.pending_session_sync = None;
                    }

                    if should_sync_session(
                        self.pending_session_sync.as_deref(),
                        &session_id,
                        self.last_session_sync.elapsed(),
                    ) {
                        let pending_due =
                            self.pending_session_sync.as_deref() == Some(&*session_id);
                        let succeeded = self.sync_session_from_server(&session_id).is_ok();
                        tick_changed |= succeeded;
                        if pending_due {
                            self.pending_session_sync =
                                apply_sync_outcome(self.pending_session_sync.take(), succeeded);
                        }
                    }
                }
                if self.last_aux_sync.elapsed() >= Duration::from_secs(5) {
                    self.refresh_session_list_dialog();
                    let _ = self.refresh_skill_list_dialog();
                    let _ = self.refresh_lsp_status();
                    let _ = self.refresh_mcp_dialog();
                    self.last_aux_sync = Instant::now();
                    tick_changed = true;
                }
                self.event_caused_change = tick_changed;
            }
            _ => {}
        }

        Ok(())
    }

    fn has_open_dialog_layer(&self) -> bool {
        self.alert_dialog.is_open()
            || self.help_dialog.is_open()
            || self.status_dialog.is_open()
            || self.session_rename_dialog.is_open()
            || self.session_export_dialog.is_open()
            || self.prompt_stash_dialog.is_open()
            || self.skill_list_dialog.is_open()
            || self.slash_popup.is_open()
            || self.command_palette.is_open()
            || self.model_select.is_open()
            || self.agent_select.is_open()
            || self.session_list_dialog.is_open()
            || self.theme_list_dialog.is_open()
            || self.mcp_dialog.is_open()
            || self.timeline_dialog.is_open()
            || self.fork_dialog.is_open()
            || self.provider_dialog.is_open()
            || self.subagent_dialog.is_open()
            || self.tag_dialog.is_open()
    }

    fn close_top_dialog(&mut self) -> bool {
        if self.alert_dialog.is_open() {
            self.alert_dialog.close();
            return true;
        }
        if self.help_dialog.is_open() {
            self.help_dialog.close();
            return true;
        }
        if self.status_dialog.is_open() {
            self.status_dialog.close();
            return true;
        }
        if self.session_rename_dialog.is_open() {
            self.session_rename_dialog.close();
            return true;
        }
        if self.session_export_dialog.is_open() {
            self.session_export_dialog.close();
            return true;
        }
        if self.prompt_stash_dialog.is_open() {
            self.prompt_stash_dialog.close();
            return true;
        }
        if self.skill_list_dialog.is_open() {
            self.skill_list_dialog.close();
            return true;
        }
        if self.slash_popup.is_open() {
            self.slash_popup.close();
            return true;
        }
        if self.command_palette.is_open() {
            self.command_palette.close();
            return true;
        }
        if self.model_select.is_open() {
            self.model_select.close();
            return true;
        }
        if self.agent_select.is_open() {
            self.agent_select.close();
            return true;
        }
        if self.session_list_dialog.is_open() {
            self.session_list_dialog.close();
            return true;
        }
        if self.theme_list_dialog.is_open() {
            let initial = self.theme_list_dialog.initial_theme_id().to_string();
            let _ = self.context.set_theme_by_name(&initial);
            self.theme_list_dialog.close();
            return true;
        }
        if self.mcp_dialog.is_open() {
            self.mcp_dialog.close();
            return true;
        }
        if self.timeline_dialog.is_open() {
            self.timeline_dialog.close();
            return true;
        }
        if self.fork_dialog.is_open() {
            self.fork_dialog.close();
            return true;
        }
        if self.provider_dialog.is_open() {
            self.provider_dialog.close();
            return true;
        }
        if self.subagent_dialog.is_open() {
            self.subagent_dialog.close();
            return true;
        }
        if self.tag_dialog.is_open() {
            self.tag_dialog.close();
            return true;
        }
        false
    }

    fn scroll_active_dialog(&mut self, up: bool) {
        if self.prompt_stash_dialog.is_open() {
            if up {
                self.prompt_stash_dialog.move_up();
            } else {
                self.prompt_stash_dialog.move_down();
            }
            return;
        }
        if self.skill_list_dialog.is_open() {
            if up {
                self.skill_list_dialog.move_up();
            } else {
                self.skill_list_dialog.move_down();
            }
            return;
        }
        if self.slash_popup.is_open() {
            if up {
                self.slash_popup.move_up();
            } else {
                self.slash_popup.move_down();
            }
            return;
        }
        if self.command_palette.is_open() {
            if up {
                self.command_palette.move_up();
            } else {
                self.command_palette.move_down();
            }
            return;
        }
        if self.model_select.is_open() {
            if up {
                self.model_select.move_up();
            } else {
                self.model_select.move_down();
            }
            return;
        }
        if self.agent_select.is_open() {
            if up {
                self.agent_select.move_up();
            } else {
                self.agent_select.move_down();
            }
            return;
        }
        if self.session_list_dialog.is_open() {
            if self.session_list_dialog.is_renaming() {
                return;
            }
            if up {
                self.session_list_dialog.move_up();
            } else {
                self.session_list_dialog.move_down();
            }
            return;
        }
        if self.theme_list_dialog.is_open() {
            if up {
                self.theme_list_dialog.move_up();
            } else {
                self.theme_list_dialog.move_down();
            }
            if let Some(theme_id) = self.theme_list_dialog.selected_theme_id() {
                let _ = self.context.set_theme_by_name(&theme_id);
            }
            return;
        }
        if self.mcp_dialog.is_open() {
            if up {
                self.mcp_dialog.move_up();
            } else {
                self.mcp_dialog.move_down();
            }
            return;
        }
        if self.timeline_dialog.is_open() {
            if up {
                self.timeline_dialog.move_up();
            } else {
                self.timeline_dialog.move_down();
            }
            return;
        }
        if self.fork_dialog.is_open() {
            if up {
                self.fork_dialog.move_up();
            } else {
                self.fork_dialog.move_down();
            }
            return;
        }
        if self.provider_dialog.is_open() {
            if up {
                self.provider_dialog.move_up();
            } else {
                self.provider_dialog.move_down();
            }
            return;
        }
        if self.subagent_dialog.is_open() {
            if up {
                self.subagent_dialog.scroll_up();
            } else {
                self.subagent_dialog.scroll_down(50);
            }
            return;
        }
        if self.tag_dialog.is_open() {
            if up {
                self.tag_dialog.move_up();
            } else {
                self.tag_dialog.move_down();
            }
        }
    }

    fn handle_dialog_mouse(
        &mut self,
        mouse_event: &crossterm::event::MouseEvent,
    ) -> anyhow::Result<bool> {
        use crossterm::event::{MouseButton, MouseEventKind};

        if !self.has_open_dialog_layer() {
            return Ok(false);
        }

        match mouse_event.kind {
            MouseEventKind::ScrollUp => {
                self.scroll_active_dialog(true);
                self.event_caused_change = true;
                Ok(true)
            }
            MouseEventKind::ScrollDown => {
                self.scroll_active_dialog(false);
                self.event_caused_change = true;
                Ok(true)
            }
            MouseEventKind::Down(MouseButton::Left) => {
                self.event_caused_change = self.close_top_dialog();
                Ok(true)
            }
            MouseEventKind::Moved
            | MouseEventKind::Down(_)
            | MouseEventKind::Drag(_)
            | MouseEventKind::Up(_) => {
                self.event_caused_change = false;
                Ok(true)
            }
            _ => {
                self.event_caused_change = false;
                Ok(true)
            }
        }
    }

    fn handle_dialog_key(&mut self, key: KeyEvent) -> anyhow::Result<bool> {
        if self.alert_dialog.is_open() {
            match key.code {
                KeyCode::Esc | KeyCode::Enter => {
                    self.alert_dialog.close();
                }
                KeyCode::Char('c')
                    if key.modifiers.contains(KeyModifiers::CONTROL)
                        && !self.alert_dialog.message().is_empty() =>
                {
                    let _ = Clipboard::write_text(self.alert_dialog.message());
                }
                _ => {}
            }
            return Ok(true);
        }
        if self.help_dialog.is_open() {
            if matches!(key.code, KeyCode::Esc | KeyCode::Enter) {
                self.help_dialog.close();
            }
            return Ok(true);
        }
        if self.status_dialog.is_open() {
            if matches!(key.code, KeyCode::Esc | KeyCode::Enter) {
                self.status_dialog.close();
            }
            return Ok(true);
        }
        if self.session_rename_dialog.is_open() {
            match key.code {
                KeyCode::Esc => self.session_rename_dialog.close(),
                KeyCode::Backspace => self.session_rename_dialog.handle_backspace(),
                KeyCode::Enter => {
                    if let Some((session_id, title)) = self.session_rename_dialog.confirm() {
                        if let Some(client) = self.context.get_api_client() {
                            if let Err(err) = client.update_session_title(&session_id, &title) {
                                self.alert_dialog.set_message(&format!(
                                    "Failed to rename session `{}`:\n{}",
                                    session_id, err
                                ));
                                self.alert_dialog.open();
                            } else {
                                self.refresh_session_list_dialog();
                                let _ = self.sync_session_from_server(&session_id);
                            }
                        }
                    }
                }
                KeyCode::Char(c)
                    if !key.modifiers.contains(KeyModifiers::CONTROL)
                        && !key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    self.session_rename_dialog.handle_input(c);
                }
                _ => {}
            }
            return Ok(true);
        }
        if self.session_export_dialog.is_open() {
            match key.code {
                KeyCode::Esc => self.session_export_dialog.close(),
                KeyCode::Backspace => self.session_export_dialog.handle_backspace(),
                KeyCode::Enter => {
                    if let Some(session_id) = self.session_export_dialog.session_id() {
                        let filename = self.session_export_dialog.filename().trim();
                        if filename.is_empty() {
                            self.alert_dialog
                                .set_message("Filename cannot be empty for export.");
                            self.alert_dialog.open();
                        } else {
                            match self.export_session_to_file(session_id, filename) {
                                Ok(path) => {
                                    self.alert_dialog.set_message(&format!(
                                        "Session exported to `{}`.",
                                        path.display()
                                    ));
                                    self.alert_dialog.open();
                                    self.session_export_dialog.close();
                                }
                                Err(err) => {
                                    self.alert_dialog.set_message(&format!(
                                        "Failed to export session:\n{}",
                                        err
                                    ));
                                    self.alert_dialog.open();
                                }
                            }
                        }
                    }
                }
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    if let Some(session_id) = self.session_export_dialog.session_id() {
                        match self.build_session_transcript(session_id) {
                            Some(text) => {
                                if let Err(err) = Clipboard::write_text(&text) {
                                    self.alert_dialog.set_message(&format!(
                                        "Failed to copy transcript to clipboard:\n{}",
                                        err
                                    ));
                                    self.alert_dialog.open();
                                } else {
                                    self.alert_dialog
                                        .set_message("Session transcript copied to clipboard.");
                                    self.alert_dialog.open();
                                }
                            }
                            None => {
                                self.alert_dialog
                                    .set_message("No transcript available for current session.");
                                self.alert_dialog.open();
                            }
                        }
                    }
                }
                KeyCode::Char(c)
                    if !key.modifiers.contains(KeyModifiers::CONTROL)
                        && !key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    self.session_export_dialog.handle_input(c);
                }
                _ => {}
            }
            return Ok(true);
        }
        if self.prompt_stash_dialog.is_open() {
            match key.code {
                KeyCode::Esc => self.prompt_stash_dialog.close(),
                KeyCode::Up => self.prompt_stash_dialog.move_up(),
                KeyCode::Down => self.prompt_stash_dialog.move_down(),
                KeyCode::Backspace => self.prompt_stash_dialog.handle_backspace(),
                KeyCode::Enter => {
                    if let Some(index) = self.prompt_stash_dialog.selected_index() {
                        if self.prompt.load_stash(index) {
                            self.prompt_stash_dialog.close();
                        }
                    }
                }
                KeyCode::Char('d') => {
                    if let Some(index) = self.prompt_stash_dialog.selected_index() {
                        if self.prompt.remove_stash(index) {
                            let entries = self
                                .prompt
                                .stash_entries()
                                .iter()
                                .cloned()
                                .map(|entry| StashItem {
                                    input: entry.input,
                                    created_at: entry.created_at,
                                })
                                .collect::<Vec<_>>();
                            self.prompt_stash_dialog.set_entries(entries);
                        }
                    }
                }
                KeyCode::Char(c)
                    if !key.modifiers.contains(KeyModifiers::CONTROL)
                        && !key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    self.prompt_stash_dialog.handle_input(c);
                }
                _ => {}
            }
            return Ok(true);
        }
        if self.skill_list_dialog.is_open() {
            match key.code {
                KeyCode::Esc => self.skill_list_dialog.close(),
                KeyCode::Up => self.skill_list_dialog.move_up(),
                KeyCode::Down => self.skill_list_dialog.move_down(),
                KeyCode::Backspace => self.skill_list_dialog.handle_backspace(),
                KeyCode::Enter => {
                    if let Some(skill) = self.skill_list_dialog.selected_skill() {
                        self.prompt.set_input(format!("/{} ", skill));
                        self.skill_list_dialog.close();
                    }
                }
                KeyCode::Char(c)
                    if !key.modifiers.contains(KeyModifiers::CONTROL)
                        && !key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    self.skill_list_dialog.handle_input(c);
                }
                _ => {}
            }
            return Ok(true);
        }

        if self.slash_popup.is_open() {
            match key.code {
                KeyCode::Esc => self.slash_popup.close(),
                KeyCode::Up => self.slash_popup.move_up(),
                KeyCode::Down => self.slash_popup.move_down(),
                KeyCode::Backspace => {
                    if !self.slash_popup.handle_backspace() {
                        self.slash_popup.close();
                    }
                }
                KeyCode::Enter => {
                    self.slash_popup.select_current();
                    if let Some(action) = self.slash_popup.take_action() {
                        self.execute_command_action(action)?;
                    }
                }
                KeyCode::Char(c)
                    if !key.modifiers.contains(KeyModifiers::CONTROL)
                        && !key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    self.slash_popup.handle_input(c);
                }
                _ => {}
            }
            return Ok(true);
        }

        if self.command_palette.is_open() {
            match key.code {
                KeyCode::Esc => self.command_palette.close(),
                KeyCode::Up => self.command_palette.move_up(),
                KeyCode::Down => self.command_palette.move_down(),
                KeyCode::Backspace => self.command_palette.handle_backspace(),
                KeyCode::Enter => {
                    let action = self.command_palette.selected_action();
                    self.command_palette.close();
                    if let Some(action) = action {
                        self.execute_command_action(action)?;
                    }
                }
                KeyCode::Char(c)
                    if !key.modifiers.contains(KeyModifiers::CONTROL)
                        && !key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    self.command_palette.handle_input(c);
                }
                _ => {}
            }
            return Ok(true);
        }

        if self.model_select.is_open() {
            match key.code {
                KeyCode::Esc => self.model_select.close(),
                KeyCode::Up => self.model_select.move_up(),
                KeyCode::Down => self.model_select.move_down(),
                KeyCode::Backspace => self.model_select.handle_backspace(),
                KeyCode::Enter => {
                    if let Some(model) = self.model_select.selected_model() {
                        let model_ref = format!("{}/{}", model.provider, model.id);
                        self.set_active_model_selection(model_ref, Some(model.provider.clone()));
                    }
                    self.model_select.close();
                }
                KeyCode::Char(c)
                    if !key.modifiers.contains(KeyModifiers::CONTROL)
                        && !key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    self.model_select.handle_input(c);
                }
                _ => {}
            }
            return Ok(true);
        }

        if self.agent_select.is_open() {
            match key.code {
                KeyCode::Esc => self.agent_select.close(),
                KeyCode::Up => self.agent_select.move_up(),
                KeyCode::Down => self.agent_select.move_down(),
                KeyCode::Enter => {
                    if let Some(agent) = self.agent_select.selected_agent() {
                        self.context.set_agent(agent.name.clone());
                    }
                    self.agent_select.close();
                }
                _ => {}
            }
            return Ok(true);
        }

        if self.session_list_dialog.is_open() {
            if self.session_list_dialog.is_renaming() {
                match key.code {
                    KeyCode::Esc => self.session_list_dialog.cancel_rename(),
                    KeyCode::Backspace => self.session_list_dialog.handle_rename_backspace(),
                    KeyCode::Enter => {
                        if let Some((session_id, title)) = self.session_list_dialog.confirm_rename()
                        {
                            if let Some(client) = self.context.get_api_client() {
                                if let Err(err) = client.update_session_title(&session_id, &title) {
                                    self.alert_dialog.set_message(&format!(
                                        "Failed to rename session `{}`:\n{}",
                                        session_id, err
                                    ));
                                    self.alert_dialog.open();
                                } else {
                                    self.refresh_session_list_dialog();
                                    if self.active_session_id.as_deref()
                                        == Some(session_id.as_str())
                                    {
                                        let _ = self.sync_session_from_server(&session_id);
                                    }
                                }
                            }
                        }
                    }
                    KeyCode::Char(c)
                        if !key.modifiers.contains(KeyModifiers::CONTROL)
                            && !key.modifiers.contains(KeyModifiers::ALT) =>
                    {
                        self.session_list_dialog.handle_rename_input(c);
                    }
                    _ => {}
                }
                return Ok(true);
            }

            match key.code {
                KeyCode::Esc => self.session_list_dialog.close(),
                KeyCode::Up => self.session_list_dialog.move_up(),
                KeyCode::Down => self.session_list_dialog.move_down(),
                KeyCode::Backspace => {
                    self.session_list_dialog.handle_backspace();
                    self.refresh_session_list_dialog();
                }
                KeyCode::Enter => {
                    let target = self.session_list_dialog.selected_session_id();
                    self.session_list_dialog.close();
                    if let Some(session_id) = target {
                        self.context.navigate(Route::Session {
                            session_id: session_id.clone(),
                        });
                        self.ensure_session_view(&session_id);
                        let _ = self.sync_session_from_server(&session_id);
                    }
                }
                KeyCode::Char('r') if self.matches_keybind("session_rename", key) => {
                    let _ = self.session_list_dialog.start_rename_selected();
                }
                KeyCode::Char('d') if self.matches_keybind("session_delete", key) => {
                    if let Some(state) = self.session_list_dialog.trigger_delete_selected() {
                        match state {
                            SessionDeleteState::Armed(_) => {}
                            SessionDeleteState::Confirmed(session_id) => {
                                if let Some(client) = self.context.get_api_client() {
                                    if let Err(err) = client.delete_session(&session_id) {
                                        self.alert_dialog.set_message(&format!(
                                            "Failed to delete session `{}`:\n{}",
                                            session_id, err
                                        ));
                                        self.alert_dialog.open();
                                    } else {
                                        if self.active_session_id.as_deref()
                                            == Some(session_id.as_str())
                                        {
                                            self.context.navigate(Route::Home);
                                            self.active_session_id = None;
                                            self.session_view = None;
                                        }
                                        self.refresh_session_list_dialog();
                                    }
                                }
                            }
                        }
                    }
                }
                KeyCode::Char(c)
                    if !key.modifiers.contains(KeyModifiers::CONTROL)
                        && !key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    self.session_list_dialog.handle_input(c);
                    self.refresh_session_list_dialog();
                }
                _ => {}
            }
            return Ok(true);
        }

        if self.theme_list_dialog.is_open() {
            match key.code {
                KeyCode::Esc => {
                    let initial = self.theme_list_dialog.initial_theme_id().to_string();
                    let _ = self.context.set_theme_by_name(&initial);
                    self.theme_list_dialog.close();
                }
                KeyCode::Up => {
                    self.theme_list_dialog.move_up();
                    if let Some(theme_id) = self.theme_list_dialog.selected_theme_id() {
                        let _ = self.context.set_theme_by_name(&theme_id);
                    }
                }
                KeyCode::Down => {
                    self.theme_list_dialog.move_down();
                    if let Some(theme_id) = self.theme_list_dialog.selected_theme_id() {
                        let _ = self.context.set_theme_by_name(&theme_id);
                    }
                }
                KeyCode::Backspace => {
                    self.theme_list_dialog.handle_backspace();
                    if let Some(theme_id) = self.theme_list_dialog.selected_theme_id() {
                        let _ = self.context.set_theme_by_name(&theme_id);
                    }
                }
                KeyCode::Enter => {
                    if let Some(theme_id) = self.theme_list_dialog.selected_theme_id() {
                        let _ = self.context.set_theme_by_name(&theme_id);
                    }
                    self.theme_list_dialog.close();
                }
                KeyCode::Char(c)
                    if !key.modifiers.contains(KeyModifiers::CONTROL)
                        && !key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    self.theme_list_dialog.handle_input(c);
                    if let Some(theme_id) = self.theme_list_dialog.selected_theme_id() {
                        let _ = self.context.set_theme_by_name(&theme_id);
                    }
                }
                _ => {}
            }
            return Ok(true);
        }

        if self.mcp_dialog.is_open() {
            match key.code {
                KeyCode::Esc => self.mcp_dialog.close(),
                KeyCode::Up => self.mcp_dialog.move_up(),
                KeyCode::Down => self.mcp_dialog.move_down(),
                KeyCode::Char('r') => {
                    let _ = self.refresh_mcp_dialog();
                }
                KeyCode::Char('a') => {
                    if let Some(item) = self.mcp_dialog.selected_item() {
                        if let Some(client) = self.context.get_api_client() {
                            match client.start_mcp_auth(&item.name) {
                                Ok(auth) => {
                                    self.alert_dialog.set_message(&format!(
                                        "MCP `{}` auth started:\n{}\n\nComplete OAuth, then reconnect.",
                                        item.name, auth.authorization_url
                                    ));
                                    self.alert_dialog.open();
                                }
                                Err(err) => {
                                    self.alert_dialog.set_message(&format!(
                                        "Failed to start MCP auth `{}`:\n{}",
                                        item.name, err
                                    ));
                                    self.alert_dialog.open();
                                }
                            }
                            let _ = client.authenticate_mcp(&item.name);
                            let _ = self.refresh_mcp_dialog();
                        }
                    }
                }
                KeyCode::Char('x') => {
                    if let Some(item) = self.mcp_dialog.selected_item() {
                        if let Some(client) = self.context.get_api_client() {
                            if let Err(err) = client.remove_mcp_auth(&item.name) {
                                self.alert_dialog.set_message(&format!(
                                    "Failed to clear MCP auth `{}`:\n{}",
                                    item.name, err
                                ));
                                self.alert_dialog.open();
                            }
                            let _ = self.refresh_mcp_dialog();
                        }
                    }
                }
                KeyCode::Enter => {
                    if let Some(item) = self.mcp_dialog.selected_item() {
                        if let Some(client) = self.context.get_api_client() {
                            let result = if item.status == "connected" {
                                client.disconnect_mcp(&item.name)
                            } else {
                                client.connect_mcp(&item.name)
                            };
                            if let Err(err) = result {
                                self.alert_dialog.set_message(&format!(
                                    "Failed to toggle MCP `{}`:\n{}",
                                    item.name, err
                                ));
                                self.alert_dialog.open();
                            }
                            let _ = self.refresh_mcp_dialog();
                        }
                    }
                }
                _ => {}
            }
            return Ok(true);
        }

        if self.timeline_dialog.is_open() {
            match key.code {
                KeyCode::Esc => self.timeline_dialog.close(),
                KeyCode::Up => self.timeline_dialog.move_up(),
                KeyCode::Down => self.timeline_dialog.move_down(),
                KeyCode::Enter => {
                    if let Some(msg_id) = self.timeline_dialog.selected_message_id() {
                        let msg_id = msg_id.to_string();
                        self.timeline_dialog.close();
                        if let Some(ref mut sv) = self.session_view {
                            sv.scroll_to_message(&msg_id);
                        }
                    }
                }
                _ => {}
            }
            return Ok(true);
        }

        if self.fork_dialog.is_open() {
            match key.code {
                KeyCode::Esc => self.fork_dialog.close(),
                KeyCode::Up => self.fork_dialog.move_up(),
                KeyCode::Down => self.fork_dialog.move_down(),
                KeyCode::Enter => {
                    let session_id = self.fork_dialog.session_id().map(|s| s.to_string());
                    let msg_id = self
                        .fork_dialog
                        .selected_message_id()
                        .map(|s| s.to_string());
                    self.fork_dialog.close();
                    if let Some(sid) = session_id {
                        if let Some(client) = self.context.get_api_client() {
                            match client.fork_session(&sid, msg_id.as_deref()) {
                                Ok(new_session) => {
                                    self.cache_session_from_api(&new_session);
                                    self.context.navigate(Route::Session {
                                        session_id: new_session.id.clone(),
                                    });
                                    self.ensure_session_view(&new_session.id);
                                    let _ = self.sync_session_from_server(&new_session.id);
                                    self.alert_dialog.set_message(&format!(
                                        "Forked session created: {}",
                                        new_session.title
                                    ));
                                    self.alert_dialog.open();
                                }
                                Err(err) => {
                                    self.alert_dialog
                                        .set_message(&format!("Failed to fork session:\n{}", err));
                                    self.alert_dialog.open();
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
            return Ok(true);
        }

        if self.provider_dialog.is_open() {
            match key.code {
                KeyCode::Esc => self.provider_dialog.close(),
                KeyCode::Up => self.provider_dialog.move_up(),
                KeyCode::Down => self.provider_dialog.move_down(),
                KeyCode::Enter => {
                    if let Some(provider) = self.provider_dialog.selected_provider() {
                        self.provider_dialog.selected_provider = Some(provider.clone());
                        self.provider_dialog.input_mode = true;
                    }
                }
                _ => {}
            }
            return Ok(true);
        }

        if self.subagent_dialog.is_open() {
            match key.code {
                KeyCode::Esc => self.subagent_dialog.close(),
                KeyCode::Up => self.subagent_dialog.scroll_up(),
                KeyCode::Down => self.subagent_dialog.scroll_down(50),
                _ => {}
            }
            return Ok(true);
        }

        if self.tag_dialog.is_open() {
            match key.code {
                KeyCode::Esc => self.tag_dialog.close(),
                KeyCode::Up => self.tag_dialog.move_up(),
                KeyCode::Down => self.tag_dialog.move_down(),
                KeyCode::Char(' ') => self.tag_dialog.toggle_selection(),
                KeyCode::Enter => self.tag_dialog.close(),
                _ => {}
            }
            return Ok(true);
        }

        Ok(false)
    }

    fn execute_command_action(&mut self, action: CommandAction) -> anyhow::Result<()> {
        match action {
            CommandAction::SubmitPrompt => self.submit_prompt()?,
            CommandAction::ClearPrompt => self.prompt.clear(),
            CommandAction::PasteClipboard => self.paste_clipboard_to_prompt(),
            CommandAction::CopyPrompt => self.copy_prompt_to_clipboard(),
            CommandAction::CutPrompt => self.cut_prompt_to_clipboard(),
            CommandAction::HistoryPrevious => self.prompt.history_previous_entry(),
            CommandAction::HistoryNext => self.prompt.history_next_entry(),
            CommandAction::ToggleSidebar => self.context.toggle_sidebar(),
            CommandAction::ToggleHeader => self.context.toggle_header(),
            CommandAction::ToggleScrollbar => self.context.toggle_scrollbar(),
            CommandAction::SwitchSession => {
                self.refresh_session_list_dialog();
                self.session_list_dialog
                    .open(self.active_session_id.as_deref());
            }
            CommandAction::RenameSession => {
                self.open_session_rename_dialog();
            }
            CommandAction::ExportSession => {
                self.open_session_export_dialog();
            }
            CommandAction::PromptStashPush => {
                if self.prompt.stash_current() {
                    self.alert_dialog.set_message("Prompt stashed.");
                    self.alert_dialog.open();
                } else {
                    self.alert_dialog
                        .set_message("Prompt is empty, nothing to stash.");
                    self.alert_dialog.open();
                }
            }
            CommandAction::PromptStashList => {
                self.open_prompt_stash_dialog();
            }
            CommandAction::PromptSkillList => {
                self.open_skill_list_dialog();
            }
            CommandAction::SwitchTheme => {
                self.refresh_theme_list_dialog();
                let current_theme = self.context.current_theme_name();
                self.theme_list_dialog.open(&current_theme);
            }
            CommandAction::CycleVariant => {
                self.cycle_model_variant();
            }
            CommandAction::ToggleAppearance => {
                let _ = self.context.toggle_theme_mode();
            }
            CommandAction::ViewStatus => {
                self.refresh_status_dialog();
                self.status_dialog.open();
            }
            CommandAction::ToggleMcp => {
                let _ = self.refresh_mcp_dialog();
                self.mcp_dialog.open();
            }
            CommandAction::ToggleTips => {
                self.context.toggle_tips_hidden();
            }
            CommandAction::SwitchModel => {
                self.refresh_model_dialog();
                self.model_select.open();
            }
            CommandAction::SwitchAgent => {
                self.refresh_agent_dialog();
                self.agent_select.open();
            }
            CommandAction::NewSession => {
                self.context.navigate(Route::Home);
                self.active_session_id = None;
                self.session_view = None;
            }
            CommandAction::ShowHelp => {
                self.help_dialog.open();
            }
            CommandAction::ToggleCommandPalette => {
                self.sync_command_palette_labels();
                self.command_palette.open();
            }
            CommandAction::ToggleTimestamps => {
                self.context.toggle_timestamps();
            }
            CommandAction::ToggleThinking => {
                self.context.toggle_thinking();
            }
            CommandAction::ToggleToolDetails => {
                self.context.toggle_tool_details();
            }
            CommandAction::ToggleDensity => {
                self.context.toggle_message_density();
            }
            CommandAction::ToggleSemanticHighlight => {
                self.context.toggle_semantic_highlight();
            }
            CommandAction::ExternalEditor => {}
            CommandAction::ConnectProvider => {
                self.provider_dialog.open();
            }
            CommandAction::ShareSession => {
                self.handle_share_session();
            }
            CommandAction::UnshareSession => {
                self.handle_unshare_session();
            }
            CommandAction::ForkSession => {
                self.handle_fork_session();
            }
            CommandAction::CompactSession => {
                self.handle_compact_session();
            }
            CommandAction::Timeline => {
                self.handle_open_timeline();
            }
            CommandAction::Undo => {
                self.handle_undo();
            }
            CommandAction::Redo => {
                self.handle_redo();
            }
            CommandAction::ListSessions | CommandAction::OpenSessionList => {
                self.refresh_session_list_dialog();
                self.session_list_dialog
                    .open(self.active_session_id.as_deref());
            }
            CommandAction::CopySession => {
                self.handle_copy_session();
            }
            CommandAction::OpenStash => {
                self.open_prompt_stash_dialog();
            }
            CommandAction::OpenSkills => {
                self.open_skill_list_dialog();
            }
            CommandAction::OpenThemeList => {
                self.refresh_theme_list_dialog();
                let current_theme = self.context.current_theme_name();
                self.theme_list_dialog.open(&current_theme);
            }
            CommandAction::ShowStatus => {
                self.refresh_status_dialog();
                self.status_dialog.open();
            }
            CommandAction::ManageMcp | CommandAction::OpenMcpList => {
                let _ = self.refresh_mcp_dialog();
                self.mcp_dialog.open();
            }
            CommandAction::OpenModelList => {
                self.refresh_model_dialog();
                self.model_select.open();
            }
            CommandAction::OpenAgentList => {
                self.refresh_agent_dialog();
                self.agent_select.open();
            }
            CommandAction::Exit => self.state = AppState::Exiting,
        }

        Ok(())
    }

    fn paste_clipboard_to_prompt(&mut self) {
        match Clipboard::read_text() {
            Ok(text) => {
                if !text.is_empty() {
                    self.prompt.insert_text(&text);
                }
            }
            Err(err) => {
                self.alert_dialog
                    .set_message(&format!("Failed to read clipboard:\n{}", err));
                self.alert_dialog.open();
            }
        }
    }

    fn copy_prompt_to_clipboard(&mut self) {
        let content = self.prompt.get_input().trim();
        if content.is_empty() {
            return;
        }
        if let Err(err) = Clipboard::write_text(content) {
            self.alert_dialog
                .set_message(&format!("Failed to write clipboard:\n{}", err));
            self.alert_dialog.open();
        }
    }

    fn cut_prompt_to_clipboard(&mut self) {
        let content = self.prompt.get_input().trim();
        if content.is_empty() {
            return;
        }
        if let Err(err) = Clipboard::write_text(content) {
            self.alert_dialog
                .set_message(&format!("Failed to write clipboard:\n{}", err));
            self.alert_dialog.open();
            return;
        }
        self.prompt.clear();
    }

    /// Copy the current screen selection to clipboard and show a toast.
    fn copy_selection(&mut self) {
        if !self.selection.is_active() {
            return;
        }
        let lines = self.screen_lines.clone();
        let text = self
            .selection
            .get_selected_text(|row| lines.get(row as usize).cloned());
        if !text.is_empty() {
            match Clipboard::write_text(&text) {
                Ok(()) => {
                    self.toast
                        .show(ToastVariant::Info, "Copied to clipboard", 2000);
                }
                Err(err) => {
                    self.toast
                        .show(ToastVariant::Error, &format!("Copy failed: {}", err), 3000);
                }
            }
        }
        self.selection.clear();
    }

    fn current_session_id(&self) -> Option<String> {
        match self.context.current_route() {
            Route::Session { session_id } => Some(session_id),
            _ => self.active_session_id.clone(),
        }
    }

    fn open_session_rename_dialog(&mut self) {
        let Some(session_id) = self.current_session_id() else {
            self.alert_dialog
                .set_message("No active session to rename.");
            self.alert_dialog.open();
            return;
        };

        let title = self
            .context
            .session
            .read()
            .sessions
            .get(&session_id)
            .map(|s| s.title.clone())
            .unwrap_or_else(|| "New Session".to_string());
        self.session_rename_dialog.open(session_id, title);
    }

    fn open_session_export_dialog(&mut self) {
        let Some(session_id) = self.current_session_id() else {
            self.alert_dialog
                .set_message("No active session to export.");
            self.alert_dialog.open();
            return;
        };

        let title = self
            .context
            .session
            .read()
            .sessions
            .get(&session_id)
            .map(|s| s.title.clone())
            .unwrap_or_else(|| "New Session".to_string());
        let default_filename = default_export_filename(&title, &session_id);
        self.session_export_dialog
            .open(session_id, default_filename);
    }

    fn open_prompt_stash_dialog(&mut self) {
        let entries = self
            .prompt
            .stash_entries()
            .iter()
            .cloned()
            .map(|entry| StashItem {
                input: entry.input,
                created_at: entry.created_at,
            })
            .collect::<Vec<_>>();
        self.prompt_stash_dialog.set_entries(entries);
        self.prompt_stash_dialog.open();
    }

    fn open_skill_list_dialog(&mut self) {
        if let Err(err) = self.refresh_skill_list_dialog() {
            self.alert_dialog
                .set_message(&format!("Failed to refresh skills:\n{}", err));
            self.alert_dialog.open();
        }
        self.skill_list_dialog.open();
    }

    fn handle_share_session(&mut self) {
        let Some(session_id) = self.current_session_id() else {
            self.alert_dialog.set_message("No active session to share.");
            self.alert_dialog.open();
            return;
        };
        let Some(client) = self.context.get_api_client() else {
            return;
        };
        match client.share_session(&session_id) {
            Ok(response) => {
                let _ = Clipboard::write_text(&response.url);
                self.alert_dialog.set_message(&format!(
                    "Session shared. Link copied to clipboard:\n{}",
                    response.url
                ));
                self.alert_dialog.open();
            }
            Err(err) => {
                self.alert_dialog
                    .set_message(&format!("Failed to share session:\n{}", err));
                self.alert_dialog.open();
            }
        }
    }

    fn handle_unshare_session(&mut self) {
        let Some(session_id) = self.current_session_id() else {
            self.alert_dialog
                .set_message("No active session to unshare.");
            self.alert_dialog.open();
            return;
        };
        let Some(client) = self.context.get_api_client() else {
            return;
        };
        match client.unshare_session(&session_id) {
            Ok(_) => {
                self.alert_dialog
                    .set_message("Session sharing link revoked.");
                self.alert_dialog.open();
            }
            Err(err) => {
                self.alert_dialog
                    .set_message(&format!("Failed to unshare session:\n{}", err));
                self.alert_dialog.open();
            }
        }
    }

    fn handle_compact_session(&mut self) {
        let Some(session_id) = self.current_session_id() else {
            self.alert_dialog
                .set_message("No active session to compact.");
            self.alert_dialog.open();
            return;
        };
        let Some(client) = self.context.get_api_client() else {
            return;
        };
        match client.compact_session(&session_id) {
            Ok(_) => {
                let _ = self.sync_session_from_server(&session_id);
                self.alert_dialog
                    .set_message("Session compacted successfully.");
                self.alert_dialog.open();
            }
            Err(err) => {
                self.alert_dialog
                    .set_message(&format!("Failed to compact session:\n{}", err));
                self.alert_dialog.open();
            }
        }
    }

    fn handle_undo(&mut self) {
        let Some(session_id) = self.current_session_id() else {
            self.alert_dialog.set_message("No active session for undo.");
            self.alert_dialog.open();
            return;
        };
        let session_ctx = self.context.session.read();
        let messages = session_ctx.messages.get(&session_id);
        let last_user_msg = messages
            .and_then(|msgs| msgs.iter().rev().find(|m| m.role == MessageRole::User))
            .map(|m| (m.id.clone(), m.content.clone()));
        drop(session_ctx);

        let Some((msg_id, msg_content)) = last_user_msg else {
            self.alert_dialog.set_message("No user message to revert.");
            self.alert_dialog.open();
            return;
        };
        let Some(client) = self.context.get_api_client() else {
            return;
        };
        match client.revert_session(&session_id, &msg_id) {
            Ok(_) => {
                self.prompt.set_input(msg_content);
                let _ = self.sync_session_from_server(&session_id);
                self.alert_dialog
                    .set_message("Message reverted. Prompt restored.");
                self.alert_dialog.open();
            }
            Err(err) => {
                self.alert_dialog
                    .set_message(&format!("Failed to revert message:\n{}", err));
                self.alert_dialog.open();
            }
        }
    }

    fn handle_redo(&mut self) {
        let Some(_session_id) = self.current_session_id() else {
            self.alert_dialog.set_message("No active session for redo.");
            self.alert_dialog.open();
            return;
        };
        // Redo re-submits the current prompt content (which was restored by undo)
        let input = self.prompt.get_input().trim().to_string();
        if input.is_empty() {
            self.alert_dialog
                .set_message("Nothing to redo. Prompt is empty.");
            self.alert_dialog.open();
            return;
        }
        // Re-submit the prompt to effectively redo
        if let Err(err) = self.submit_prompt() {
            self.alert_dialog
                .set_message(&format!("Failed to redo:\n{}", err));
            self.alert_dialog.open();
        }
    }

    fn handle_copy_session(&mut self) {
        let Some(session_id) = self.current_session_id() else {
            self.alert_dialog.set_message("No active session to copy.");
            self.alert_dialog.open();
            return;
        };
        match self.build_session_transcript(&session_id) {
            Some(text) => {
                if let Err(err) = Clipboard::write_text(&text) {
                    self.alert_dialog
                        .set_message(&format!("Failed to copy transcript to clipboard:\n{}", err));
                    self.alert_dialog.open();
                } else {
                    self.alert_dialog
                        .set_message("Session transcript copied to clipboard.");
                    self.alert_dialog.open();
                }
            }
            None => {
                self.alert_dialog
                    .set_message("No transcript available for current session.");
                self.alert_dialog.open();
            }
        }
    }

    fn handle_open_timeline(&mut self) {
        let Some(session_id) = self.current_session_id() else {
            self.alert_dialog
                .set_message("No active session for timeline.");
            self.alert_dialog.open();
            return;
        };
        let session_ctx = self.context.session.read();
        let entries = session_ctx
            .messages
            .get(&session_id)
            .map(|msgs| {
                msgs.iter()
                    .map(|m| {
                        let role = match m.role {
                            MessageRole::User => "user",
                            MessageRole::Assistant => "assistant",
                            MessageRole::System => "system",
                        };
                        let preview = m
                            .content
                            .chars()
                            .take(60)
                            .collect::<String>()
                            .replace('\n', " ");
                        TimelineEntry {
                            message_id: m.id.clone(),
                            role: role.to_string(),
                            preview,
                            timestamp: m.created_at.format("%H:%M:%S").to_string(),
                        }
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        drop(session_ctx);
        self.timeline_dialog.open(entries);
    }

    fn handle_fork_session(&mut self) {
        let Some(session_id) = self.current_session_id() else {
            self.alert_dialog.set_message("No active session to fork.");
            self.alert_dialog.open();
            return;
        };
        let session_ctx = self.context.session.read();
        let entries = session_ctx
            .messages
            .get(&session_id)
            .map(|msgs| {
                msgs.iter()
                    .map(|m| {
                        let role = match m.role {
                            MessageRole::User => "user",
                            MessageRole::Assistant => "assistant",
                            MessageRole::System => "system",
                        };
                        let preview = m
                            .content
                            .chars()
                            .take(60)
                            .collect::<String>()
                            .replace('\n', " ");
                        ForkEntry {
                            message_id: m.id.clone(),
                            role: role.to_string(),
                            preview,
                            timestamp: m.created_at.format("%H:%M:%S").to_string(),
                        }
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        drop(session_ctx);
        self.fork_dialog.open(session_id, entries);
    }

    fn build_session_transcript(&self, session_id: &str) -> Option<String> {
        let session_ctx = self.context.session.read();
        let session = session_ctx.sessions.get(session_id)?;
        let messages = session_ctx.messages.get(session_id)?;

        let mut output = String::new();
        output.push_str(&format!("# {}\n\n", session.title));
        output.push_str(&format!("Session ID: `{}`\n", session.id));
        output.push_str(&format!("Created: {}\n", session.created_at.to_rfc3339()));
        output.push_str(&format!("Updated: {}\n\n", session.updated_at.to_rfc3339()));

        if messages.is_empty() {
            output.push_str("_No messages_\n");
            return Some(output);
        }

        for message in messages {
            let role = match message.role {
                MessageRole::User => "User",
                MessageRole::Assistant => "Assistant",
                MessageRole::System => "System",
            };
            output.push_str(&format!("## {}\n\n", role));
            if message.content.trim().is_empty() {
                output.push_str("_Empty message_\n\n");
            } else {
                output.push_str(&message.content);
                output.push_str("\n\n");
            }
        }

        Some(output)
    }

    fn export_session_to_file(&self, session_id: &str, filename: &str) -> anyhow::Result<PathBuf> {
        let transcript = self.build_session_transcript(session_id).ok_or_else(|| {
            anyhow::anyhow!("No transcript available for session `{}`", session_id)
        })?;

        let mut path = PathBuf::from(filename.trim());
        if path.as_os_str().is_empty() {
            anyhow::bail!("filename cannot be empty");
        }
        if path.is_relative() {
            path = std::env::current_dir()?.join(path);
        }
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        std::fs::write(&path, transcript)?;
        Ok(path)
    }

    fn submit_prompt(&mut self) -> anyhow::Result<()> {
        let shell_mode = self.prompt.is_shell_mode();
        let input = self.prompt.take_input();
        if input.trim().is_empty() {
            return Ok(());
        }

        if shell_mode {
            return self.submit_shell_command(input);
        }

        let Some(client) = self.context.get_api_client() else {
            eprintln!("API client not initialized");
            return Ok(());
        };

        let agent = selected_agent(&self.context);
        let model = self.selected_model_for_prompt();
        let variant = self.context.current_model_variant();

        match self.context.current_route() {
            Route::Home => {
                let optimistic_session_id = self.create_optimistic_session();
                let opt_id = self.append_optimistic_user_message(
                    &optimistic_session_id,
                    &input,
                    agent.clone(),
                    model.clone(),
                    variant.clone(),
                );
                self.context.navigate(Route::Session {
                    session_id: optimistic_session_id.clone(),
                });
                self.ensure_session_view(&optimistic_session_id);
                self.set_session_status(&optimistic_session_id, SessionStatus::Running);
                self.prompt.set_spinner_task_kind(TaskKind::LlmRequest);
                self.prompt.set_spinner_active(true);
                // Render immediately so the user sees their message before network I/O.
                let _ = self.draw();

                let session = match client.create_session(None) {
                    Ok(session) => session,
                    Err(err) => {
                        self.remove_optimistic_session(&optimistic_session_id);
                        self.context.navigate(Route::Home);
                        self.active_session_id = None;
                        self.session_view = None;
                        self.prompt.set_spinner_active(false);
                        self.alert_dialog
                            .set_message(&format!("Failed to create session:\n{}", err));
                        self.alert_dialog.open();
                        return Ok(());
                    }
                };
                self.promote_optimistic_session(&optimistic_session_id, &session);
                self.context.navigate(Route::Session {
                    session_id: session.id.clone(),
                });
                self.ensure_session_view(&session.id);
                if let Err(err) = client.send_prompt(
                    &session.id,
                    input.clone(),
                    agent.clone(),
                    model.clone(),
                    variant.clone(),
                ) {
                    self.remove_optimistic_message(&session.id, &opt_id);
                    self.set_session_status(&session.id, SessionStatus::Idle);
                    self.sync_prompt_spinner_state();
                    self.alert_dialog
                        .set_message(&format!("Failed to send prompt:\n{}", err));
                    self.alert_dialog.open();
                    return Ok(());
                }
                self.set_session_status(&session.id, SessionStatus::Running);
                self.prompt.set_spinner_task_kind(TaskKind::LlmRequest);
                self.prompt.set_spinner_active(true);
            }
            Route::Session { session_id } => {
                // Optimistic: show user message immediately before network call
                let opt_id = self.append_optimistic_user_message(
                    &session_id,
                    &input,
                    agent.clone(),
                    model.clone(),
                    variant.clone(),
                );
                self.set_session_status(&session_id, SessionStatus::Running);
                self.prompt.set_spinner_task_kind(TaskKind::LlmRequest);
                self.prompt.set_spinner_active(true);
                self.ensure_session_view(&session_id);
                // Render immediately so the user sees their message before network I/O.
                let _ = self.draw();
                if let Err(err) = client.send_prompt(
                    &session_id,
                    input.clone(),
                    agent.clone(),
                    model.clone(),
                    variant.clone(),
                ) {
                    self.remove_optimistic_message(&session_id, &opt_id);
                    self.set_session_status(&session_id, SessionStatus::Idle);
                    self.sync_prompt_spinner_state();
                    self.alert_dialog
                        .set_message(&format!("Failed to send prompt:\n{}", err));
                    self.alert_dialog.open();
                    return Ok(());
                }
            }
            _ => {}
        }

        Ok(())
    }

    fn submit_shell_command(&mut self, command: String) -> anyhow::Result<()> {
        let command = command.trim().to_string();
        if command.is_empty() {
            return Ok(());
        }

        let Some(client) = self.context.get_api_client() else {
            eprintln!("API client not initialized");
            return Ok(());
        };

        let user_line = format!("$ {}", command);
        let agent = selected_agent(&self.context);
        let model = self.selected_model_for_prompt();
        let variant = self.context.current_model_variant();

        match self.context.current_route() {
            Route::Home => {
                let optimistic_session_id = self.create_optimistic_session();
                let _opt_id = self.append_optimistic_user_message(
                    &optimistic_session_id,
                    &user_line,
                    agent.clone(),
                    model.clone(),
                    variant.clone(),
                );
                self.context.navigate(Route::Session {
                    session_id: optimistic_session_id.clone(),
                });
                self.ensure_session_view(&optimistic_session_id);
                let _ = self.draw();

                let session = match client.create_session(None) {
                    Ok(session) => session,
                    Err(err) => {
                        self.remove_optimistic_session(&optimistic_session_id);
                        self.context.navigate(Route::Home);
                        self.active_session_id = None;
                        self.session_view = None;
                        self.alert_dialog
                            .set_message(&format!("Failed to create session:\n{}", err));
                        self.alert_dialog.open();
                        return Ok(());
                    }
                };
                self.promote_optimistic_session(&optimistic_session_id, &session);
                self.context.navigate(Route::Session {
                    session_id: session.id.clone(),
                });
                self.ensure_session_view(&session.id);

                if let Err(err) = client.execute_shell(&session.id, command.clone(), None) {
                    self.alert_dialog
                        .set_message(&format!("Failed to execute shell command:\n{}", err));
                    self.alert_dialog.open();
                    return Ok(());
                }

                self.set_session_status(&session.id, SessionStatus::Idle);
                let _ = self.sync_session_from_server(&session.id);
            }
            Route::Session { session_id } => {
                let opt_id = self.append_optimistic_user_message(
                    &session_id,
                    &user_line,
                    agent.clone(),
                    model.clone(),
                    variant.clone(),
                );
                self.ensure_session_view(&session_id);
                let _ = self.draw();
                if let Err(err) = client.execute_shell(&session_id, command.clone(), None) {
                    self.remove_optimistic_message(&session_id, &opt_id);
                    self.alert_dialog
                        .set_message(&format!("Failed to execute shell command:\n{}", err));
                    self.alert_dialog.open();
                    return Ok(());
                }
                self.set_session_status(&session_id, SessionStatus::Idle);
                let _ = self.sync_session_from_server(&session_id);
            }
            _ => {}
        }

        self.sync_prompt_spinner_state();
        Ok(())
    }

    fn refresh_model_dialog(&mut self) {
        let Some(client) = self.context.get_api_client() else {
            self.context.set_has_connected_provider(false);
            return;
        };

        let Ok(providers) = client.get_config_providers() else {
            self.context.set_has_connected_provider(false);
            return;
        };
        let has_connected_provider = providers.providers.iter().any(|p| !p.models.is_empty());
        self.context
            .set_has_connected_provider(has_connected_provider);

        let mut available_models = HashSet::new();
        let mut variant_map: HashMap<String, Vec<String>> = HashMap::new();
        let mut models = Vec::new();
        let mut context_providers = Vec::new();
        for provider in providers.providers {
            let provider_id = provider.id.clone();
            let provider_name = provider.name.clone();
            let mut provider_models = Vec::new();
            for model in provider.models {
                let model_id = model.id;
                let model_name = model.name;
                let model_context_window = model.context_window.unwrap_or(0);
                let model_ref = format!("{}/{}", provider_id, model_id);
                available_models.insert(model_ref.clone());
                let entry = variant_map.entry(model_ref).or_default();
                for variant in model.variants {
                    if !entry.iter().any(|value| value == &variant) {
                        entry.push(variant);
                    }
                }
                models.push(Model {
                    id: model_id.clone(),
                    name: model_name.clone(),
                    provider: provider_id.clone(),
                    context_window: model_context_window,
                });
                provider_models.push(crate::context::ModelInfo {
                    id: format!("{}/{}", provider_id, model_id),
                    name: model_name,
                    context_window: model_context_window,
                    max_output_tokens: 0,
                    supports_vision: false,
                    supports_tools: true,
                });
            }
            context_providers.push(crate::context::ProviderInfo {
                id: provider_id,
                name: provider_name,
                models: provider_models,
            });
        }
        *self.context.providers.write() = context_providers;
        models.sort_by(|a, b| {
            a.provider
                .cmp(&b.provider)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        for variants in variant_map.values_mut() {
            variants.sort();
        }
        self.model_select.set_models(models);
        self.available_models = available_models;
        self.model_variants = variant_map;
        self.model_variant_selection.retain(|model_key, variant| {
            let Some(available) = self.model_variants.get(model_key) else {
                return false;
            };
            match variant {
                Some(value) => available.iter().any(|item| item == value),
                None => true,
            }
        });
        self.sync_current_model_variant();

        let model_missing = self.context.current_model.read().is_none();
        if model_missing {
            if let Some((provider, model_id)) = providers.default_model.iter().next() {
                self.set_active_model_selection(
                    format!("{}/{}", provider, model_id),
                    Some(provider.clone()),
                );
            }
        }
    }

    fn refresh_agent_dialog(&mut self) {
        let Some(client) = self.context.get_api_client() else {
            return;
        };

        let Ok(agents) = client.list_agents() else {
            return;
        };

        if agents.is_empty() {
            return;
        }

        let theme = self.context.theme.read().clone();
        let mut agent_names = Vec::new();
        let mapped = agents
            .into_iter()
            .enumerate()
            .map(|(idx, agent)| {
                agent_names.push(agent.id.clone());
                Agent {
                    name: agent.id,
                    description: agent
                        .description
                        .unwrap_or_else(|| "No description".to_string()),
                    color: theme.agent_color(idx),
                }
            })
            .collect::<Vec<_>>();
        self.agent_select.set_agents(mapped);
        let current = self.context.current_agent.read().clone();
        if !agent_names.iter().any(|name| name == &current) {
            if let Some(first) = agent_names.first() {
                self.context.set_agent(first.clone());
                self.sync_prompt_spinner_style();
            }
        }
        self.prompt.set_agent_suggestions(agent_names);
    }

    fn cycle_agent(&mut self, direction: i8) {
        self.refresh_agent_dialog();

        let agents = self.agent_select.agents();
        if agents.is_empty() {
            return;
        }

        let current = self.context.current_agent.read().clone();
        let current_index = agents
            .iter()
            .position(|agent| agent.name == current)
            .unwrap_or(0);

        let len = agents.len();
        let next_index = if direction >= 0 {
            (current_index + 1) % len
        } else if current_index == 0 {
            len - 1
        } else {
            current_index - 1
        };
        let next_agent = agents[next_index].name.clone();

        self.context.set_agent(next_agent);
        self.sync_prompt_spinner_style();
    }

    fn refresh_session_list_dialog(&mut self) {
        let Some(client) = self.context.get_api_client() else {
            return;
        };

        let query = self.session_list_dialog.query().trim().to_string();
        let sessions_result = if query.is_empty() {
            client.list_sessions()
        } else {
            client.list_sessions_filtered(Some(&query), Some(30))
        };
        let Ok(sessions) = sessions_result else {
            return;
        };
        let status_map = client.get_session_status().unwrap_or_default();
        {
            let mut session_ctx = self.context.session.write();
            for session in &sessions {
                if let Some(status) = status_map.get(&session.id) {
                    session_ctx.set_status(&session.id, map_api_run_status(status));
                }
            }
        }

        let items = sessions
            .into_iter()
            .map(|session| SessionItem {
                is_busy: status_map.get(&session.id).map(|s| s.busy).unwrap_or(false),
                id: session.id,
                title: session.title,
                directory: session.directory,
                parent_id: session.parent_id,
                updated_at: session.time.updated,
            })
            .collect::<Vec<_>>();
        self.session_list_dialog.set_sessions(items);
    }

    fn refresh_theme_list_dialog(&mut self) {
        let options = self
            .context
            .available_theme_names()
            .into_iter()
            .map(|name| ThemeOption {
                id: name.clone(),
                name: format_theme_option_label(&name),
            })
            .collect::<Vec<_>>();
        self.theme_list_dialog.set_options(options);
    }

    fn refresh_skill_list_dialog(&mut self) -> anyhow::Result<()> {
        let Some(client) = self.context.get_api_client() else {
            return Ok(());
        };
        let skills = client.list_skills()?;
        self.skill_list_dialog.set_skills(skills.clone());
        self.prompt.set_skill_suggestions(skills);
        Ok(())
    }

    fn refresh_lsp_status(&mut self) -> anyhow::Result<()> {
        let Some(client) = self.context.get_api_client() else {
            return Ok(());
        };
        let servers = client.get_lsp_servers()?;
        let statuses = servers
            .into_iter()
            .map(|id| crate::context::LspStatus {
                id,
                root: "-".to_string(),
                status: crate::context::LspConnectionStatus::Connected,
            })
            .collect::<Vec<_>>();
        *self.context.lsp_status.write() = statuses;
        Ok(())
    }

    fn refresh_status_dialog(&mut self) {
        let formatters = self
            .context
            .get_api_client()
            .and_then(|client| client.get_formatters().ok())
            .unwrap_or_default();
        let route_label = match self.context.current_route() {
            Route::Home => "home".to_string(),
            Route::Session { session_id } => format!("session ({})", session_id),
            Route::Settings => "settings".to_string(),
            Route::Help => "help".to_string(),
        };
        let session_ctx = self.context.session.read();
        let mcp_servers = self.context.mcp_servers.read();
        let lsp_status = self.context.lsp_status.read();
        let connected_mcp = mcp_servers
            .iter()
            .filter(|s| matches!(s.status, McpConnectionStatus::Connected))
            .count();
        let mut lines = vec![
            StatusLine::title("Runtime"),
            StatusLine::normal(format!("Route: {}", route_label)),
            StatusLine::normal(format!(
                "Directory: {}",
                self.context.directory.read().as_str()
            )),
            StatusLine::normal(format!(
                "Agent: {}",
                self.context.current_agent.read().as_str()
            )),
            StatusLine::normal(format!("Model: {}", self.current_model_label())),
            StatusLine::normal(format!(
                "Theme: {}",
                format_theme_option_label(&self.context.current_theme_name())
            )),
            StatusLine::normal(format!("Loaded sessions: {}", session_ctx.sessions.len())),
            StatusLine::muted(""),
            StatusLine::title(format!(
                "MCP Servers ({}, connected: {})",
                mcp_servers.len(),
                connected_mcp
            )),
        ];
        if mcp_servers.is_empty() {
            lines.push(StatusLine::muted("- No MCP servers"));
        } else {
            for server in mcp_servers.iter() {
                let status_text = match server.status {
                    McpConnectionStatus::Connected => "connected",
                    McpConnectionStatus::Disconnected => "disconnected",
                    McpConnectionStatus::Failed => "failed",
                    McpConnectionStatus::NeedsAuth => "needs authentication",
                    McpConnectionStatus::NeedsClientRegistration => "needs client ID",
                    McpConnectionStatus::Disabled => "disabled",
                };
                let base = format!("- {}: {}", server.name, status_text);
                match server.status {
                    McpConnectionStatus::Connected => lines.push(StatusLine::success(base)),
                    McpConnectionStatus::NeedsAuth
                    | McpConnectionStatus::NeedsClientRegistration => {
                        lines.push(StatusLine::warning(base))
                    }
                    McpConnectionStatus::Failed => {
                        let text = if let Some(error) = &server.error {
                            format!("{} ({})", base, error)
                        } else {
                            base
                        };
                        lines.push(StatusLine::error(text));
                    }
                    _ => lines.push(StatusLine::muted(base)),
                }
            }
        }

        lines.push(StatusLine::muted(""));
        lines.push(StatusLine::title(format!(
            "LSP Servers ({})",
            lsp_status.len()
        )));
        if lsp_status.is_empty() {
            lines.push(StatusLine::muted("- No LSP servers"));
        } else {
            for server in lsp_status.iter() {
                lines.push(StatusLine::success(format!("- {}", server.id)));
            }
        }

        lines.push(StatusLine::muted(""));
        lines.push(StatusLine::title(format!(
            "Formatters ({})",
            formatters.len()
        )));
        if formatters.is_empty() {
            lines.push(StatusLine::muted("- No formatters"));
        } else {
            for formatter in formatters {
                lines.push(StatusLine::success(format!("- {}", formatter)));
            }
        }
        self.status_dialog.set_status_lines(lines);
    }

    fn refresh_mcp_dialog(&mut self) -> anyhow::Result<()> {
        let Some(client) = self.context.get_api_client() else {
            return Ok(());
        };
        let servers = client.get_mcp_status()?;

        let mcp_items = servers
            .iter()
            .map(|server| McpItem {
                name: server.name.clone(),
                status: server.status.clone(),
                tools: server.tools,
                resources: server.resources,
                error: server.error.clone(),
            })
            .collect::<Vec<_>>();
        self.mcp_dialog.set_items(mcp_items);

        let statuses = servers
            .into_iter()
            .map(|server| {
                let status = map_mcp_status(&server);
                McpServerStatus {
                    name: server.name,
                    status,
                    error: server.error,
                }
            })
            .collect::<Vec<_>>();
        *self.context.mcp_servers.write() = statuses;
        Ok(())
    }

    fn set_active_model_selection(&mut self, model_ref: String, provider: Option<String>) {
        let (model_key, explicit_variant) =
            parse_model_ref_selection(&model_ref, &self.available_models, &self.model_variants);
        let resolved_provider = provider.or_else(|| provider_from_model(&model_key));
        self.context
            .set_model_selection(model_key.clone(), resolved_provider);
        if let Some(variant) = explicit_variant {
            self.model_variant_selection
                .insert(model_key.clone(), Some(variant.clone()));
            self.context.set_model_variant(Some(variant));
            return;
        }
        let variant = self
            .model_variant_selection
            .get(&model_key)
            .cloned()
            .flatten();
        self.context.set_model_variant(variant);
    }

    fn sync_current_model_variant(&mut self) {
        let Some(model_ref) = self.context.current_model.read().clone() else {
            self.context.set_model_variant(None);
            return;
        };
        let (model_key, explicit_variant) =
            parse_model_ref_selection(&model_ref, &self.available_models, &self.model_variants);
        if let Some(explicit) = explicit_variant {
            self.model_variant_selection
                .insert(model_key.clone(), Some(explicit.clone()));
            self.context.set_model_variant(Some(explicit));
            return;
        }
        let selected = self
            .model_variant_selection
            .get(&model_key)
            .cloned()
            .flatten();
        let available = self.model_variants.get(&model_key);
        let valid = selected.filter(|value| {
            available
                .map(|items| items.iter().any(|item| item == value))
                .unwrap_or(false)
        });
        if valid.is_none() {
            self.model_variant_selection.insert(model_key, None);
        }
        self.context.set_model_variant(valid);
    }

    fn cycle_model_variant(&mut self) {
        let Some(model_ref) = self.context.current_model.read().clone() else {
            return;
        };
        let (model_key, explicit_variant) =
            parse_model_ref_selection(&model_ref, &self.available_models, &self.model_variants);
        let Some(variants) = self.model_variants.get(&model_key).cloned() else {
            self.model_variant_selection.remove(&model_key);
            self.context.set_model_variant(None);
            return;
        };
        if variants.is_empty() {
            self.model_variant_selection.insert(model_key, None);
            self.context.set_model_variant(None);
            return;
        }

        let current = self
            .model_variant_selection
            .get(&model_key)
            .cloned()
            .flatten()
            .or(explicit_variant);
        let next = match current {
            None => Some(variants[0].clone()),
            Some(current_value) => {
                let index = variants.iter().position(|item| item == &current_value);
                match index {
                    Some(idx) if idx + 1 < variants.len() => Some(variants[idx + 1].clone()),
                    _ => None,
                }
            }
        };
        self.model_variant_selection.insert(model_key, next.clone());
        self.context.set_model_variant(next);
    }

    fn current_model_label(&self) -> String {
        let Some(model) = self.context.current_model.read().clone() else {
            return "(not selected)".to_string();
        };
        let (base_model, _) =
            parse_model_ref_selection(&model, &self.available_models, &self.model_variants);
        if let Some(variant) = self.context.current_model_variant() {
            return format!("{base_model} ({variant})");
        }
        base_model
    }

    fn selected_model_for_prompt(&self) -> Option<String> {
        let model = self.context.current_model.read().clone()?;
        let (base, inline_variant) =
            parse_model_ref_selection(&model, &self.available_models, &self.model_variants);
        let variant = self.context.current_model_variant();

        let resolved = if let Some(variant) = variant {
            let candidate = format!("{base}/{variant}");
            if self.available_models.contains(&candidate) {
                candidate
            } else {
                model.clone()
            }
        } else if inline_variant.is_some() && self.available_models.contains(&base) {
            base
        } else {
            model.clone()
        };

        Some(resolved)
    }

    fn ensure_session_view(&mut self, session_id: &str) {
        if self.active_session_id.as_deref() == Some(session_id) {
            return;
        }

        self.active_session_id = Some(session_id.to_string());
        self.session_view = Some(SessionView::new(
            self.context.clone(),
            session_id.to_string(),
        ));
    }

    fn cache_session_from_api(&self, session: &SessionInfo) {
        let mut session_ctx = self.context.session.write();
        session_ctx.upsert_session(map_api_session(session));
    }

    fn create_optimistic_session(&mut self) -> String {
        let now = Utc::now();
        let session_id = format!("local_session_{}", now.timestamp_millis());
        let session = Session {
            id: session_id.clone(),
            title: "New Session".to_string(),
            created_at: now,
            updated_at: now,
            parent_id: None,
            share: None,
        };

        let mut session_ctx = self.context.session.write();
        session_ctx.upsert_session(session);
        session_ctx.messages.entry(session_id.clone()).or_default();
        session_ctx.set_status(&session_id, SessionStatus::Idle);
        session_id
    }

    fn remove_optimistic_session(&mut self, session_id: &str) {
        let mut session_ctx = self.context.session.write();
        session_ctx.sessions.remove(session_id);
        session_ctx.messages.remove(session_id);
        session_ctx.session_status.remove(session_id);
        session_ctx.session_diff.remove(session_id);
        session_ctx.todos.remove(session_id);
        session_ctx.revert.remove(session_id);
        if session_ctx.current_session_id.as_deref() == Some(session_id) {
            session_ctx.current_session_id = None;
        }
    }

    fn promote_optimistic_session(&mut self, optimistic_session_id: &str, session: &SessionInfo) {
        let mut session_ctx = self.context.session.write();
        let optimistic_messages = session_ctx
            .messages
            .remove(optimistic_session_id)
            .unwrap_or_default();
        let optimistic_status = session_ctx.session_status.remove(optimistic_session_id);
        let optimistic_diff = session_ctx.session_diff.remove(optimistic_session_id);
        let optimistic_todos = session_ctx.todos.remove(optimistic_session_id);
        let optimistic_revert = session_ctx.revert.remove(optimistic_session_id);
        session_ctx.sessions.remove(optimistic_session_id);

        let real_session_id = session.id.clone();
        session_ctx.upsert_session(map_api_session(session));
        if !optimistic_messages.is_empty() {
            session_ctx
                .messages
                .insert(real_session_id.clone(), optimistic_messages);
        }
        if let Some(status) = optimistic_status {
            session_ctx
                .session_status
                .insert(real_session_id.clone(), status);
        }
        if let Some(diff) = optimistic_diff {
            session_ctx
                .session_diff
                .insert(real_session_id.clone(), diff);
        }
        if let Some(todos) = optimistic_todos {
            session_ctx.todos.insert(real_session_id.clone(), todos);
        }
        if let Some(revert) = optimistic_revert {
            session_ctx.revert.insert(real_session_id, revert);
        }
    }

    fn append_optimistic_user_message(
        &mut self,
        session_id: &str,
        content: &str,
        agent: Option<String>,
        model: Option<String>,
        variant: Option<String>,
    ) -> String {
        let now = Utc::now();
        let id = format!("local_user_{}", now.timestamp_millis());
        let message = Message {
            id: id.clone(),
            role: MessageRole::User,
            content: content.to_string(),
            created_at: now,
            agent,
            model,
            mode: variant,
            finish: None,
            error: None,
            completed_at: None,
            cost: 0.0,
            tokens: TokenUsage::default(),
            parts: vec![ContextMessagePart::Text {
                text: content.to_string(),
            }],
        };

        let mut session_ctx = self.context.session.write();
        session_ctx
            .messages
            .entry(session_id.to_string())
            .or_default();
        session_ctx.add_message(session_id, message);
        if let Some(session) = session_ctx.sessions.get_mut(session_id) {
            session.updated_at = now;
        }
        id
    }

    fn remove_optimistic_message(&mut self, session_id: &str, msg_id: &str) {
        let mut session_ctx = self.context.session.write();
        if let Some(msgs) = session_ctx.messages.get_mut(session_id) {
            msgs.retain(|m| m.id != msg_id);
        }
    }

    fn sync_session_from_server(&mut self, session_id: &str) -> anyhow::Result<()> {
        self.last_session_sync = Instant::now();

        let Some(client) = self.context.get_api_client() else {
            return Ok(());
        };

        let session = client.get_session(session_id)?;
        let messages = client.get_messages(session_id)?;
        let revert = session.revert.as_ref().map(map_api_revert);

        let mut session_ctx = self.context.session.write();
        session_ctx.upsert_session(map_api_session(&session));
        session_ctx.set_messages(
            session_id,
            messages
                .iter()
                .map(map_api_message)
                .collect::<Vec<Message>>(),
        );
        if let Some(revert_info) = revert {
            session_ctx
                .revert
                .insert(session_id.to_string(), revert_info);
        } else {
            session_ctx.revert.remove(session_id);
        }
        drop(session_ctx);

        Ok(())
    }

    fn set_session_status(&mut self, session_id: &str, status: SessionStatus) {
        let mut session_ctx = self.context.session.write();
        session_ctx.set_status(session_id, status);
    }

    fn sync_prompt_spinner_style(&mut self) {
        let theme = self.context.theme.read().clone();
        let agent = self.context.current_agent.read().clone();
        self.prompt
            .set_spinner_color(agent_color_from_name(&theme, &agent));
    }

    fn sync_prompt_spinner_state(&mut self) -> bool {
        let before_active = self.prompt.spinner_active();
        let before_kind = self.prompt.spinner_task_kind();

        let Route::Session { session_id } = self.context.current_route() else {
            self.prompt.set_spinner_active(false);
            self.prompt.clear_interrupt_confirmation();
            return before_active != self.prompt.spinner_active()
                || before_kind != self.prompt.spinner_task_kind();
        };

        let status = {
            let session_ctx = self.context.session.read();
            session_ctx.status(&session_id).clone()
        };
        let is_active = !matches!(status, SessionStatus::Idle);
        self.prompt.set_spinner_active(is_active);
        if !is_active {
            self.prompt.clear_interrupt_confirmation();
            return before_active != self.prompt.spinner_active()
                || before_kind != self.prompt.spinner_task_kind();
        }

        let task_kind = self.infer_spinner_task_kind(&session_id, &status);
        if self.prompt.spinner_task_kind() != task_kind {
            self.prompt.set_spinner_task_kind(task_kind);
        }

        before_active != self.prompt.spinner_active()
            || before_kind != self.prompt.spinner_task_kind()
    }

    fn infer_spinner_task_kind(&self, session_id: &str, status: &SessionStatus) -> TaskKind {
        if matches!(status, SessionStatus::Retrying { .. }) {
            return TaskKind::LlmResponse;
        }

        let session_ctx = self.context.session.read();
        let Some(messages) = session_ctx.messages.get(session_id) else {
            return TaskKind::LlmRequest;
        };
        let Some(last_message) = messages.last() else {
            return TaskKind::LlmRequest;
        };

        match last_message.role {
            MessageRole::User => TaskKind::LlmRequest,
            MessageRole::Assistant => infer_task_kind_from_message(last_message),
            MessageRole::System => TaskKind::LlmResponse,
        }
    }

    fn matches_keybind(&self, keybind_name: &str, key: KeyEvent) -> bool {
        self.context
            .keybind
            .read()
            .match_key(keybind_name, key.code, key.modifiers)
    }

    fn sync_command_palette_labels(&mut self) {
        let show_thinking = *self.context.show_thinking.read();
        let show_tool_details = *self.context.show_tool_details.read();
        let density = *self.context.message_density.read();
        let semantic_hl = *self.context.semantic_highlight.read();
        let show_header = *self.context.show_header.read();
        let show_scrollbar = *self.context.show_scrollbar.read();
        let tips_hidden = *self.context.tips_hidden.read();
        self.command_palette.sync_visibility_labels(
            show_thinking,
            show_tool_details,
            density,
            semantic_hl,
            show_header,
            show_scrollbar,
            tips_hidden,
        );
    }

    fn draw(&mut self) -> anyhow::Result<()> {
        self.sync_render_prelude();

        // Moved out so `paint` can borrow the rest of `self`; put straight back.
        let Some(mut tui) = self.terminal.take() else {
            return Ok(());
        };

        let result = tui.draw(|frame| self.paint(frame)).map(|_| ());
        self.terminal = Some(tui);
        Ok(result?)
    }

    /// Paint the current app state into any frame. Requires no terminal, so tests
    /// can drive it through `ratatui::backend::TestBackend`.
    pub fn render(&mut self, frame: &mut ratatui::Frame<'_>) {
        self.sync_render_prelude();
        self.paint(frame);
    }

    pub fn screen_lines(&self) -> &[String] {
        &self.screen_lines
    }

    /// State the next frame depends on. Kept out of the frame closure so drawing
    /// never mutates shared context while a frame is being built.
    fn sync_render_prelude(&mut self) {
        self.context
            .set_pending_permissions(self.permission_prompt.pending_count());

        let route = self.context.current_route();
        if let Route::Session { session_id } = &route {
            self.ensure_session_view(session_id);
        } else {
            self.active_session_id = None;
            self.session_view = None;
        }
    }

    fn paint(&mut self, frame: &mut ratatui::Frame<'_>) {
        let context = self.context.clone();
        let prompt = &self.prompt;
        let route_for_draw = self.context.current_route();
        let show_modal_overlay = self.has_open_dialog_layer()
            || self.permission_prompt.is_open
            || self.question_prompt.is_open;
        let session_view = self.session_view.as_mut();
        let theme = self.context.theme.read().clone();
        let command_palette = &self.command_palette;
        let model_select = &self.model_select;
        let agent_select = &self.agent_select;
        let session_list_dialog = &self.session_list_dialog;
        let theme_list_dialog = &self.theme_list_dialog;
        let status_dialog = &self.status_dialog;
        let mcp_dialog = &self.mcp_dialog;
        let help_dialog = &self.help_dialog;
        let alert_dialog = &self.alert_dialog;
        let session_rename_dialog = &self.session_rename_dialog;
        let session_export_dialog = &self.session_export_dialog;
        let prompt_stash_dialog = &self.prompt_stash_dialog;
        let skill_list_dialog = &self.skill_list_dialog;
        let timeline_dialog = &self.timeline_dialog;
        let fork_dialog = &self.fork_dialog;
        let provider_dialog = &self.provider_dialog;
        let subagent_dialog = &self.subagent_dialog;
        let tag_dialog = &self.tag_dialog;
        let permission_prompt = &self.permission_prompt;
        let question_prompt = &self.question_prompt;
        let slash_popup = &self.slash_popup;
        let toast = &self.toast;
        let selection = &self.selection;

        let mut captured_lines: Vec<String> = Vec::new();

        let area = frame.size();
        if area.width < 10 || area.height < 10 {
            return;
        }

        match route_for_draw {
            Route::Home => {
                let home = HomeView::new(context.clone());
                home.render_with_prompt(frame, area, prompt);
            }
            Route::Session { .. } => {
                if let Some(view) = session_view {
                    view.render(frame, area, prompt);
                } else {
                    let home = HomeView::new(context.clone());
                    home.render_with_prompt(frame, area, prompt);
                }
            }
            _ => {
                let home = HomeView::new(context.clone());
                home.render_with_prompt(frame, area, prompt);
            }
        }

        if show_modal_overlay {
            let modal_backdrop = ratatui::widgets::Block::default()
                .style(ratatui::style::Style::default().bg(theme.background_menu));
            frame.render_widget(modal_backdrop, area);
        }

        slash_popup.render(frame, area, &theme);
        command_palette.render(frame, area, &theme);
        model_select.render(frame, area, &theme);
        agent_select.render(frame, area, &theme);
        session_list_dialog.render(frame, area, &theme);
        theme_list_dialog.render(frame, area, &theme);
        status_dialog.render(frame, area, &theme);
        mcp_dialog.render(frame, area, &theme);
        help_dialog.render(frame, area, &theme);
        alert_dialog.render(frame, area, &theme);
        session_rename_dialog.render(frame, area, &theme);
        session_export_dialog.render(frame, area, &theme);
        prompt_stash_dialog.render(frame, area, &theme);
        skill_list_dialog.render(frame, area, &theme);
        timeline_dialog.render(frame, area, &theme);
        fork_dialog.render(frame, area, &theme);
        provider_dialog.render(frame, area, &theme);
        subagent_dialog.render(frame, area, &theme);
        tag_dialog.render(frame, area, &theme);
        permission_prompt.render(frame, area, &theme);
        question_prompt.render(frame, area, &theme);

        // Render toast notification (top-right corner)
        if toast.is_visible() {
            let toast_width = 60u16.min(area.width.saturating_sub(4));
            let toast_height = toast.desired_height(toast_width);
            let base_x = area.x + area.width.saturating_sub(toast_width.saturating_add(2));
            let max_x = area.x + area.width.saturating_sub(toast_width);
            let toast_x = base_x.saturating_add(toast.slide_offset()).min(max_x);
            let toast_area = ratatui::layout::Rect {
                x: toast_x,
                y: 2.min(area.height.saturating_sub(1)),
                width: toast_width,
                height: toast_height.min(area.height.saturating_sub(2)),
            };
            toast.render(frame, toast_area, &theme);
        }

        // Snapshot the rendered buffer for text selection (before highlight overlay)
        let buf = frame.buffer_mut();
        captured_lines.clear();
        for y in area.y..area.y + area.height {
            let mut line = String::with_capacity(area.width as usize);
            for x in area.x..area.x + area.width {
                let cell = buf.get(x, y);
                line.push_str(cell.symbol());
            }
            let trimmed = line.trim_end().to_string();
            captured_lines.push(trimmed);
        }

        // Render selection highlight — invert colors on non-empty cells,
        // matching standard terminal selection behavior (like opentui).
        if selection.is_active() {
            use ratatui::style::Color;
            for y in area.y..area.y + area.height {
                for x in area.x..area.x + area.width {
                    if !selection.is_selected(y, x) {
                        continue;
                    }
                    let cell = buf.get(x, y);
                    let sym = cell.symbol();
                    // Only highlight cells with visible text content
                    if sym.is_empty() || sym.chars().all(|c| c == ' ') {
                        continue;
                    }
                    let cell = buf.get_mut(x, y);
                    // Resolve Reset to concrete terminal defaults before swapping.
                    // Reset fg = terminal default foreground (typically white/light).
                    // Reset bg = terminal default background (typically black/dark).
                    let fg = if cell.fg == Color::Reset {
                        Color::White
                    } else {
                        cell.fg
                    };
                    let bg = if cell.bg == Color::Reset {
                        Color::Black
                    } else {
                        cell.bg
                    };
                    cell.fg = bg;
                    cell.bg = fg;
                }
            }
        }

        self.screen_lines = captured_lines;
    }
}

impl Drop for App {
    fn drop(&mut self) {
        if self.terminal.is_some() {
            let _ = terminal::restore();
        }
    }
}

fn map_api_session(session: &SessionInfo) -> Session {
    Session {
        id: session.id.clone(),
        title: session.title.clone(),
        created_at: Utc
            .timestamp_millis_opt(session.time.created)
            .single()
            .unwrap_or_else(Utc::now),
        updated_at: Utc
            .timestamp_millis_opt(session.time.updated)
            .single()
            .unwrap_or_else(Utc::now),
        parent_id: session.parent_id.clone(),
        share: None,
    }
}

fn map_api_message(envelope: &ApiMessage) -> Message {
    let info = &envelope.info;
    let parts = fold_api_parts(&envelope.parts);

    Message {
        id: info.id.clone(),
        role: match info.role.as_str() {
            "assistant" => MessageRole::Assistant,
            "system" => MessageRole::System,
            _ => MessageRole::User,
        },
        content: parts
            .iter()
            .map(message_part_text)
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
        created_at: Utc
            .timestamp_millis_opt(info.created_at)
            .single()
            .unwrap_or_else(Utc::now),
        completed_at: info
            .completed_at
            .and_then(|ts| Utc.timestamp_millis_opt(ts).single()),
        agent: info.agent.clone(),
        model: info.model.clone(),
        mode: info.mode.clone(),
        finish: metadata_str(&info.metadata, "finish_reason")
            .map(str::to_string)
            .or_else(|| info.finish.clone()),
        error: metadata_str(&info.metadata, "error")
            .map(str::to_string)
            .or_else(|| info.error.clone()),
        cost: info.cost,
        tokens: TokenUsage {
            input: info.tokens.input,
            output: info.tokens.output,
            reasoning: info.tokens.reasoning,
            cache_read: info.tokens.cache_read,
            cache_write: info.tokens.cache_write,
        },
        parts,
    }
}

/// Everything gathered from the sibling parts that share one `callID`.
struct ToolFold {
    name: Option<String>,
    arguments: Option<String>,
    output: Option<String>,
    is_error: bool,
    rank: Option<ToolRank>,
}

impl ToolFold {
    fn new() -> Self {
        Self {
            name: None,
            arguments: None,
            output: None,
            is_error: false,
            rank: None,
        }
    }

    fn absorb(&mut self, part: &crate::api::MessagePart) {
        let rank = part.state.as_ref().and_then(|state| state.status_rank());

        if self.name.is_none() {
            self.name.clone_from(&part.tool);
        }
        if self.name.is_none() {
            self.name = part.tool_call.as_ref().map(|call| call.name.clone());
        }
        if self.arguments.is_none() {
            self.arguments = part
                .tool_call
                .as_ref()
                .map(|call| match &call.input {
                    serde_json::Value::String(text) => text.clone(),
                    other => other.to_string(),
                })
                .or_else(|| {
                    part.state
                        .as_ref()
                        .and_then(|state| state.input.as_ref())
                        .map(arguments_text)
                });
        }
        if let Some(result) = &part.tool_result {
            self.output.get_or_insert_with(|| result.content.clone());
            self.is_error |= result.is_error;
        }
        if let Some(state) = &part.state {
            if let Some(output) = &state.output {
                self.output.get_or_insert_with(|| output.clone());
            }
            self.is_error |= state.is_error();
        }

        if rank > self.rank {
            self.rank = rank;
        }
    }

    /// `None` until some sibling supplies an output, which only a terminal one ever does.
    fn result(&self) -> Option<(&str, bool)> {
        self.output.as_deref().map(|output| (output, self.is_error))
    }
}

fn metadata_str<'a>(
    metadata: &'a std::collections::HashMap<String, serde_json::Value>,
    key: &str,
) -> Option<&'a str> {
    metadata.get(key).and_then(|value| value.as_str())
}

fn arguments_text(input: &serde_json::Value) -> String {
    match input {
        serde_json::Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// The `callID` a part belongs to, if it is part of a tool invocation.
///
/// Covers both wire shapes: the server's `type: "tool"` parts keyed by `callID`, and the legacy
/// flat `toolCall` / `toolResult` siblings keyed by their own id fields.
fn part_tool_key(part: &crate::api::MessagePart) -> Option<String> {
    if let Some(call_id) = &part.call_id {
        return Some(call_id.clone());
    }
    if let Some(call) = &part.tool_call {
        return Some(call.id.clone());
    }
    part.tool_result
        .as_ref()
        .map(|result| result.tool_call_id.clone())
}

/// Collapses the parts of one message, grouping every tool invocation's siblings by `callID`.
///
/// The server sends a tool call and its result as two sibling parts that agree only on `callID`
/// and differ only in `state.status`, so a stateless per-part mapping cannot pair them up. The
/// winner of each group is the sibling with the highest lifecycle rank
/// (`pending < running < completed | error`), which makes the fold independent of arrival order.
fn fold_api_parts(raw_parts: &[crate::api::MessagePart]) -> Vec<ContextMessagePart> {
    enum Slot {
        Part(ContextMessagePart),
        Tool(usize),
    }

    let mut slots: Vec<Slot> = Vec::with_capacity(raw_parts.len());
    let mut tools: Vec<(String, ToolFold)> = Vec::new();

    for part in raw_parts {
        let Some(key) = part_tool_key(part) else {
            if let Some(mapped) = map_api_message_part(part) {
                slots.push(Slot::Part(mapped));
            }
            continue;
        };

        let index = match tools.iter().position(|(id, _)| *id == key) {
            Some(index) => index,
            None => {
                tools.push((key, ToolFold::new()));
                slots.push(Slot::Tool(tools.len() - 1));
                tools.len() - 1
            }
        };
        tools[index].1.absorb(part);
    }

    let mut folded: Vec<ContextMessagePart> = Vec::with_capacity(slots.len());
    for slot in slots {
        match slot {
            Slot::Part(part) => folded.push(part),
            Slot::Tool(index) => {
                let (key, group) = &tools[index];
                folded.push(ContextMessagePart::ToolCall {
                    id: key.clone(),
                    name: group.name.clone().unwrap_or_default(),
                    arguments: group.arguments.clone().unwrap_or_default(),
                });
                if let Some((output, is_error)) = group.result() {
                    folded.push(ContextMessagePart::ToolResult {
                        id: key.clone(),
                        result: output.to_string(),
                        is_error,
                    });
                }
            }
        }
    }
    folded
}

fn map_api_revert(revert: &SessionRevertInfo) -> RevertInfo {
    RevertInfo {
        message_id: revert.message_id.clone(),
        part_id: revert.part_id.clone(),
        snapshot: revert.snapshot.clone(),
        diff: revert.diff.clone(),
    }
}

/// Maps one part that is not part of a tool invocation. Tool parts never reach here: they are
/// resolved in groups by [`fold_api_parts`], because a call and its result are siblings.
fn map_api_message_part(part: &crate::api::MessagePart) -> Option<ContextMessagePart> {
    if let Some(text) = &part.text {
        if part.ignored == Some(true) {
            return None;
        }
        if part.part_type == "reasoning" {
            return Some(ContextMessagePart::Reasoning { text: text.clone() });
        }
        // Skip synthetic text parts (auto-continue prompts, etc.)
        if part.synthetic == Some(true) {
            return None;
        }
        return Some(ContextMessagePart::Text { text: text.clone() });
    }

    if let Some(file) = part.file_info() {
        return Some(ContextMessagePart::File {
            path: file.filename,
            mime: file.mime,
        });
    }

    if let Some(tool_call) = &part.tool_call {
        let arguments = if let Some(value) = tool_call.input.as_str() {
            value.to_string()
        } else {
            tool_call.input.to_string()
        };
        return Some(ContextMessagePart::ToolCall {
            id: tool_call.id.clone(),
            name: tool_call.name.clone(),
            arguments,
        });
    }

    if let Some(tool_result) = &part.tool_result {
        return Some(ContextMessagePart::ToolResult {
            id: tool_result.tool_call_id.clone(),
            result: tool_result.content.clone(),
            is_error: tool_result.is_error,
        });
    }

    None
}

fn message_part_text(part: &ContextMessagePart) -> String {
    match part {
        ContextMessagePart::Text { text } => text.clone(),
        ContextMessagePart::Reasoning { text } => format!("[reasoning] {}", text),
        ContextMessagePart::ToolCall {
            name, arguments, ..
        } => format!("[tool:{}] {}", name, arguments),
        ContextMessagePart::ToolResult {
            result, is_error, ..
        } => {
            if *is_error {
                return format!("[tool-error] {}", result);
            }
            format!("[tool-result] {}", result)
        }
        ContextMessagePart::File { path, .. } => format!("[file] {}", path),
        ContextMessagePart::Image { url } => format!("[image] {}", url),
    }
}

fn infer_task_kind_from_message(message: &Message) -> TaskKind {
    let Some(last_part) = message.parts.last() else {
        return TaskKind::LlmResponse;
    };

    match last_part {
        ContextMessagePart::Text { .. } | ContextMessagePart::Reasoning { .. } => {
            TaskKind::LlmResponse
        }
        ContextMessagePart::ToolCall { name, .. } => task_kind_from_tool_name(name),
        ContextMessagePart::ToolResult { id, .. } => message
            .parts
            .iter()
            .rev()
            .find_map(|part| match part {
                ContextMessagePart::ToolCall {
                    id: call_id, name, ..
                } if call_id == id => Some(task_kind_from_tool_name(name)),
                _ => None,
            })
            .unwrap_or(TaskKind::ToolCall),
        ContextMessagePart::File { .. } => TaskKind::FileRead,
        ContextMessagePart::Image { .. } => TaskKind::LlmResponse,
    }
}

fn task_kind_from_tool_name(name: &str) -> TaskKind {
    let normalized = name.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return TaskKind::ToolCall;
    }

    if normalized.contains("read")
        || normalized.contains("grep")
        || normalized.contains("glob")
        || normalized.contains("list")
    {
        return TaskKind::FileRead;
    }
    if normalized.contains("write")
        || normalized.contains("edit")
        || normalized.contains("patch")
        || normalized.contains("todo")
    {
        return TaskKind::FileWrite;
    }
    if normalized.contains("bash")
        || normalized.contains("shell")
        || normalized.contains("exec")
        || normalized.contains("command")
    {
        return TaskKind::CommandExec;
    }

    TaskKind::ToolCall
}

fn map_mcp_status(server: &McpStatusInfo) -> McpConnectionStatus {
    match server.status.as_str() {
        "connected" => McpConnectionStatus::Connected,
        "failed" => McpConnectionStatus::Failed,
        "needs_auth" => McpConnectionStatus::NeedsAuth,
        "needs_client_registration" => McpConnectionStatus::NeedsClientRegistration,
        "disabled" => McpConnectionStatus::Disabled,
        _ => McpConnectionStatus::Disconnected,
    }
}

fn map_api_run_status(status: &crate::api::SessionStatusInfo) -> SessionStatus {
    if status.busy {
        if status.status.eq_ignore_ascii_case("retry") {
            return SessionStatus::Retrying {
                message: status.message.clone().unwrap_or_default(),
                attempt: status.attempt.unwrap_or(0),
                next: status.next.unwrap_or_default(),
            };
        }
        return SessionStatus::Running;
    }
    SessionStatus::Idle
}

fn agent_color_from_name(theme: &crate::theme::Theme, agent_name: &str) -> ratatui::style::Color {
    if theme.agent_colors.is_empty() {
        return theme.primary;
    }
    let mut hasher = DefaultHasher::new();
    agent_name.hash(&mut hasher);
    let idx = (hasher.finish() as usize) % theme.agent_colors.len();
    theme.agent_colors[idx]
}

fn provider_from_model(model: &str) -> Option<String> {
    let model = model.trim();
    let (provider, _) = model
        .split_once('/')
        .or_else(|| model.split_once(':'))
        .unwrap_or((model, ""));
    if provider.is_empty() || provider == model {
        return None;
    }
    Some(provider.to_string())
}

fn resolve_tui_base_url() -> String {
    if let Ok(value) = std::env::var("OPENCODE_TUI_BASE_URL") {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }

    // Prefer a live backend endpoint over a hardcoded default. This avoids
    // accidental 404s when localhost:3000 is occupied by a non-opencode service.
    let candidates = [
        "http://127.0.0.1:3000",
        "http://localhost:3000",
        "http://127.0.0.1:4096",
        "http://localhost:4096",
    ];
    let client = match reqwest::blocking::Client::builder()
        .timeout(Duration::from_millis(300))
        .build()
    {
        Ok(client) => client,
        Err(_) => return "http://localhost:3000".to_string(),
    };

    for base in candidates {
        let health_url = format!("{}/health", base);
        if let Ok(response) = client.get(&health_url).send() {
            if response.status().is_success() {
                return base.to_string();
            }
        }
    }

    "http://localhost:3000".to_string()
}

fn spawn_server_event_listener(
    event_tx: Sender<Event>,
    base_url: String,
    context: Arc<AppContext>,
) {
    thread::spawn(move || {
        let client = match reqwest::blocking::Client::builder()
            .connect_timeout(Duration::from_secs(2))
            .build()
        {
            Ok(client) => client,
            Err(err) => {
                tracing::warn!(%err, "failed to initialize server event stream client");
                return;
            }
        };

        let event_url = format!("{}/event", base_url.trim_end_matches('/'));
        loop {
            match client
                .get(&event_url)
                .header(reqwest::header::ACCEPT, "text/event-stream")
                .send()
            {
                Ok(response) if response.status().is_success() => {
                    consume_server_event_stream(response, &event_tx, &context);
                }
                Ok(response) => {
                    tracing::debug!(
                        url = %event_url,
                        status = %response.status(),
                        "server event stream subscription rejected"
                    );
                    thread::sleep(Duration::from_millis(400));
                }
                Err(err) => {
                    tracing::debug!(
                        url = %event_url,
                        %err,
                        "server event stream disconnected"
                    );
                    thread::sleep(Duration::from_millis(400));
                }
            }
        }
    });
}

fn consume_server_event_stream(
    response: reqwest::blocking::Response,
    event_tx: &Sender<Event>,
    context: &Arc<AppContext>,
) {
    let mut reader = BufReader::new(response);
    let mut line = String::new();
    let mut data_lines: Vec<String> = Vec::new();

    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => {
                forward_server_event(&data_lines, event_tx, context);
                break;
            }
            Ok(_) => {
                let trimmed = line.trim_end_matches(['\r', '\n']);
                if trimmed.is_empty() {
                    forward_server_event(&data_lines, event_tx, context);
                    data_lines.clear();
                    continue;
                }
                if trimmed.starts_with(':') {
                    continue;
                }
                if let Some(payload) = trimmed.strip_prefix("data:") {
                    data_lines.push(payload.trim_start().to_string());
                }
            }
            Err(err) => {
                tracing::debug!(%err, "error while reading server event stream");
                break;
            }
        }
    }
}

fn forward_server_event(
    data_lines: &[String],
    event_tx: &Sender<Event>,
    context: &Arc<AppContext>,
) {
    if data_lines.is_empty() {
        return;
    }
    let payload = data_lines.join("\n");
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&payload) else {
        return;
    };
    let event_type = value.get("type").and_then(|item| item.as_str());
    let session_id = value
        .get("sessionID")
        .and_then(|item| item.as_str())
        .or_else(|| value.get("sessionId").and_then(|item| item.as_str()));
    let properties = value.get("properties").unwrap_or(&value);
    let properties_session_id = || {
        properties
            .get("sessionID")
            .and_then(|item| item.as_str())
            .or_else(|| {
                properties
                    .get("info")
                    .and_then(|info| info.get("sessionID"))
                    .and_then(|item| item.as_str())
            })
    };
    match event_type {
        Some("session.updated") => {
            let Some(session_id) = session_id else {
                return;
            };
            let _ = event_tx.send(Event::Custom(CustomEvent::StateChanged(
                StateChange::SessionUpdated(session_id.to_string()),
            )));
        }
        Some("session.status") => {
            let Some(session_id) = session_id else {
                return;
            };
            let status_type = value
                .get("status")
                .and_then(|status| status.get("type"))
                .and_then(|item| item.as_str())
                .or_else(|| value.get("status").and_then(|item| item.as_str()));
            match status_type {
                Some("busy") => {
                    let _ = event_tx.send(Event::Custom(CustomEvent::StateChanged(
                        StateChange::SessionStatusBusy(session_id.to_string()),
                    )));
                }
                Some("idle") => {
                    let _ = event_tx.send(Event::Custom(CustomEvent::StateChanged(
                        StateChange::SessionStatusIdle(session_id.to_string()),
                    )));
                }
                Some("retry") => {
                    let attempt = value
                        .get("status")
                        .and_then(|status| status.get("attempt"))
                        .and_then(|item| item.as_u64())
                        .and_then(|v| u32::try_from(v).ok())
                        .unwrap_or(0);
                    let message = value
                        .get("status")
                        .and_then(|status| status.get("message"))
                        .and_then(|item| item.as_str())
                        .unwrap_or_default()
.to_string();
        let context = Arc::new(AppContext::new());
        {
            let mut session_ctx = context.session.write();
            session_ctx.set_messages(
                "ses_a",
                vec![Message {
                    id: "msg_1".to_string(),
                    role: MessageRole::Assistant,
                    content: String::new(),
                    created_at: chrono::Utc::now(),
                    agent: None,
                    model: None,
                    mode: None,
                    finish: None,
                    error: None,
                    completed_at: None,
                    cost: 0.0,
                    tokens: TokenUsage::default(),
                    parts: vec![],
                }],
            );
        }

                    let next = value
                        .get("status")
                        .and_then(|status| status.get("next"))
                        .and_then(|item| item.as_i64())
                        .unwrap_or_default();
                    let _ = event_tx.send(Event::Custom(CustomEvent::StateChanged(
                        StateChange::SessionStatusRetrying {
                            session_id: session_id.to_string(),
                            attempt,
                            message,
                            next,
                        },
                    )));
                }
                _ => {}
            }
        }
        Some("message.part.delta") => {
            let Some(session_id) = properties_session_id() else {
                return;
            };
            let Some(message_id) = properties
                .get("messageID")
                .and_then(|item| item.as_str())
            else {
                return;
            };
            let Some(delta) = properties.get("delta").and_then(|item| item.as_str()) else {
                return;
            };

            if apply_part_delta_to_context(context, session_id, message_id, delta) {
                let _ = event_tx.send(Event::Custom(CustomEvent::StateChanged(
                    StateChange::SessionUpdated(session_id.to_string()),
                )));
            }
        }
        Some("message.part.updated") | Some("message.updated") => {
            let Some(session_id) = properties_session_id() else {
                return;
            };
            let _ = event_tx.send(Event::Custom(CustomEvent::StateChanged(
                StateChange::SessionUpdated(session_id.to_string()),
            )));
        }
        _ => {}
    }
}

fn apply_part_delta_to_context(
    context: &Arc<AppContext>,
    session_id: &str,
    message_id: &str,
    delta: &str,
) -> bool {
    let mut session_ctx = context.session.write();
    let Some(messages) = session_ctx.messages.get_mut(session_id) else {
        return false;
    };
    apply_part_delta(messages, message_id, delta)
}

fn apply_part_delta(messages: &mut Vec<Message>, message_id: &str, delta: &str) -> bool {
    if delta.is_empty() {
        return false;
    }
    let Some(message) = messages.iter_mut().find(|msg| msg.id == message_id) else {
        return false;
    };

    let extends_last = matches!(
        message.parts.last(),
        Some(ContextMessagePart::Text { .. }) | Some(ContextMessagePart::Reasoning { .. })
    );
    if extends_last {
        if let Some(ContextMessagePart::Text { text })
        | Some(ContextMessagePart::Reasoning { text }) = message.parts.last_mut()
        {
            text.push_str(delta);
        }
    } else {
        message.parts.push(ContextMessagePart::Text {
            text: delta.to_string(),
        });
    }
    message.content.push_str(delta);
    true
}

fn format_theme_option_label(theme_id: &str) -> String {
    if let Some((base, variant)) = split_theme_variant(theme_id) {
        return format!("{base} ({variant})");
    }
    theme_id.to_string()
}

fn split_theme_variant(theme_id: &str) -> Option<(&str, &str)> {
    let (base, variant) = theme_id
        .rsplit_once('@')
        .or_else(|| theme_id.rsplit_once(':'))?;
    if base.is_empty() || !matches!(variant, "dark" | "light") {
        return None;
    }
    Some((base, variant))
}

fn parse_model_ref_selection(
    model_ref: &str,
    available_models: &HashSet<String>,
    model_variants: &HashMap<String, Vec<String>>,
) -> (String, Option<String>) {
    let trimmed = model_ref.trim();
    if trimmed.is_empty() {
        return (String::new(), None);
    }
    if available_models.contains(trimmed) {
        return (trimmed.to_string(), None);
    }

    let Some((candidate_base, candidate_variant)) = trimmed.rsplit_once('/') else {
        return (trimmed.to_string(), None);
    };
    if candidate_variant.is_empty() || !available_models.contains(candidate_base) {
        return (trimmed.to_string(), None);
    }
    let Some(known_variants) = model_variants.get(candidate_base) else {
        return (trimmed.to_string(), None);
    };
    if !known_variants
        .iter()
        .any(|value| value == candidate_variant)
    {
        return (trimmed.to_string(), None);
    }
    (
        candidate_base.to_string(),
        Some(candidate_variant.to_string()),
    )
}

fn selected_agent(context: &Arc<AppContext>) -> Option<String> {
    let agent = context.current_agent.read().clone();
    if agent.trim().is_empty() {
        return None;
    }
    Some(agent)
}

fn default_export_filename(title: &str, session_id: &str) -> String {
    let mut slug = String::new();
    let mut prev_dash = false;
    for ch in title.trim().chars() {
        let normalized = if ch.is_ascii_alphanumeric() {
            ch.to_ascii_lowercase()
        } else if ch.is_ascii_whitespace() || ch == '-' || ch == '_' {
            '-'
        } else {
            continue;
        };
        if normalized == '-' {
            if prev_dash || slug.is_empty() {
                continue;
            }
            prev_dash = true;
            slug.push('-');
        } else {
            prev_dash = false;
            slug.push(normalized);
        }
    }
    if slug.ends_with('-') {
        slug.pop();
    }
    if slug.is_empty() {
        let short_id = session_id.chars().take(8).collect::<String>();
        slug = format!("session-{}", short_id);
    }
    format!("{slug}.md")
}

const SESSION_SYNC_EVENT_FLOOR: Duration = Duration::from_millis(50);
const SESSION_SYNC_PERIODIC_FLOOR: Duration = Duration::from_secs(2);

fn should_sync_session(pending: Option<&str>, active: &str, elapsed_since_sync: Duration) -> bool {
    if matches!(pending, Some(pending) if pending == active) {
        return elapsed_since_sync >= SESSION_SYNC_EVENT_FLOOR;
    }
    pending.is_none() && elapsed_since_sync >= SESSION_SYNC_PERIODIC_FLOOR
}

fn apply_sync_outcome(pending: Option<String>, succeeded: bool) -> Option<String> {
    if succeeded {
        None
    } else {
        pending
    }
}

#[cfg(test)]
mod throttling_tests {
    use super::*;
    use std::time::Duration;

    /// The server nests the payload of every part event under `properties`
    /// (events.rs `envelope`). Reading `sessionID` off the top level, which is
    /// what the flat `session.updated` shape needs, drops every delta silently.
    #[test]
    fn part_delta_is_read_out_of_properties_not_the_top_level() {
        let (tx, rx) = std::sync::mpsc::channel();
        let payload = serde_json::json!({
            "id": "evt_1",
            "type": "message.part.delta",
            "properties": {
                "sessionID": "ses_a",
                "messageID": "msg_1",
                "partID": "prt_1",
                "field": "text",
                "delta": "hello",
            }
        })
        .to_string();

        let context = Arc::new(AppContext::new());
        context.session.write().set_messages(
            "ses_a",
            vec![Message {
                id: "msg_1".to_string(),
                role: MessageRole::Assistant,
                content: String::new(),
                created_at: chrono::Utc::now(),
                agent: None,
                model: None,
                mode: None,
                finish: None,
                error: None,
                completed_at: None,
                cost: 0.0,
                tokens: TokenUsage::default(),
                parts: vec![],
            }],
        );

        forward_server_event(&[payload], &tx, &context);

        let mut saw_status = false;
        while let Ok(event) = rx.try_recv() {
            if let Event::Custom(CustomEvent::StateChanged(StateChange::SessionUpdated(s))) = event
            {
                assert_eq!(s, "ses_a", "sessionID must be lifted out of properties");
                saw_status = true;
            }
        }
        assert!(saw_status, "no SessionUpdated emitted for the delta");

        let content = context.session.read().messages["ses_a"][0].content.clone();
        assert_eq!(
            content, "hello",
            "the delta must reach the transcript with no HTTP call"
        );
    }

    /// A delta must reach the transcript with no HTTP call at all.
    #[test]
    fn part_delta_grows_the_transcript_without_a_server_round_trip() {
        let mut messages = vec![Message {
            id: "msg_1".to_string(),
            role: MessageRole::Assistant,
            content: String::new(),
            created_at: chrono::Utc::now(),
            agent: None,
            model: None,
            mode: None,
            finish: None,
            error: None,
            completed_at: None,
            cost: 0.0,
            tokens: TokenUsage::default(),
            parts: vec![],
        }];

        assert!(apply_part_delta(&mut messages, "msg_1", "Hello"));
        assert!(apply_part_delta(&mut messages, "msg_1", " world"));
        assert_eq!(messages[0].content, "Hello world");

        assert!(
            !apply_part_delta(&mut messages, "msg_missing", "orphan"),
            "a delta for an unknown message must be rejected, not silently dropped"
        );
    }

    #[test]
    fn sync_gate_is_closed_immediately_after_a_sync() {
        assert!(
            !should_sync_session(None, "ses_a", Duration::from_millis(10)),
            "a sync just performed must keep the gate shut, otherwise the 16ms \
             Tick re-enters the two blocking HTTP calls immediately"
        );
    }

    #[test]
    fn sync_gate_opens_after_the_2s_window() {
        assert!(should_sync_session(None, "ses_a", Duration::from_secs(3)));
    }

    #[test]
    fn pending_for_a_different_session_does_not_open_the_gate() {
        assert!(
            !should_sync_session(Some("other"), "ses_a", Duration::from_secs(3)),
            "a pending flag naming a different session must not sync the active one"
        );
    }

    /// A pending flag for the ACTIVE session is what opens the fast 50ms path,
    /// without waiting for the 2s window.
    #[test]
    fn pending_for_the_active_session_opens_the_fast_path() {
        assert!(should_sync_session(
            Some("ses_a"),
            "ses_a",
            Duration::from_millis(60)
        ));
        assert!(!should_sync_session(
            Some("ses_a"),
            "ses_a",
            Duration::from_millis(10)
        ));
    }

    /// REGRESSION: `last_session_sync` was stamped only after the two blocking
    /// HTTP calls succeeded, so a failing sync re-opened the 2s gate on every
    /// 16ms `Event::Tick` (~62 attempts/sec on the UI thread).
    #[test]
    fn failed_attempts_cost_one_sync_per_window() {
        let mut allowed = 0usize;
        let mut last_attempt = Instant::now() - Duration::from_secs(5);

        for _ in 0..125 {
            if should_sync_session(None, "ses_a", last_attempt.elapsed()) {
                allowed += 1;
                last_attempt = Instant::now();
            }
        }

        assert_eq!(
            allowed, 1,
            "a dead server must cost one sync attempt per 2s window, not one per tick"
        );
    }

    #[test]
    fn event_burst_costs_one_sync_per_50ms() {
        let mut allowed = 0usize;
        let mut last_attempt = Instant::now() - Duration::from_millis(500);

        for _ in 0..200 {
            if should_sync_session(
                Some("ses_a"),
                "ses_a",
                last_attempt.elapsed(),
            ) {
                allowed += 1;
                last_attempt = Instant::now();
            }
        }

        assert_eq!(
            allowed, 1,
            "a session.updated burst inside one 50ms window must cost one sync, got {allowed}"
        );
    }

    #[test]
    fn pending_survives_failure_and_clears_on_success() {
        let pending = Some("ses_abc".to_string());

        assert_eq!(
            apply_sync_outcome(pending.clone(), false),
            pending,
            "a failed attempt must leave the session pending so the Tick arm retries"
        );
        assert_eq!(apply_sync_outcome(pending, true), None);
    }

    /// One sync per tick: whichever branch fires, the attempt stamp it writes
    /// must close BOTH floors for the next tick.
    #[test]
    fn one_attempt_satisfies_both_floors() {
        let after_attempt = Instant::now();
        assert!(!should_sync_session(
            Some("ses_a"),
            "ses_a",
            after_attempt.elapsed()
        ));
        assert!(!should_sync_session(
            None,
            "ses_a",
            after_attempt.elapsed()
        ));
    }
}

#[cfg(test)]
mod wire_fold_tests {
    use super::*;
    use serde_json::json;

    fn envelope(parts: serde_json::Value) -> crate::api::ApiMessage {
        serde_json::from_value(json!({
            "info": {
                "id": "msg_01",
                "sessionID": "ses_01",
                "role": "assistant",
                "createdAt": 1_700_000_000_000_i64,
                "agent": null,
                "model": null,
                "metadata": {},
            },
            "parts": parts,
        }))
        .expect("envelope must parse")
    }

    fn text_part(id: &str, text: &str) -> serde_json::Value {
        json!({ "id": id, "sessionID": "ses_01", "messageID": "msg_01",
                "type": "text", "text": text, "synthetic": false, "ignored": false })
    }

    fn tool_part(
        id: &str,
        call_id: &str,
        name: Option<&str>,
        state: serde_json::Value,
    ) -> serde_json::Value {
        let mut part = json!({
            "id": id,
            "sessionID": "ses_01",
            "messageID": "msg_01",
            "type": "tool",
            "callID": call_id,
            "state": state,
        });
        if let Some(name) = name {
            part["tool"] = json!(name);
        }
        part
    }

    fn tool_calls(message: &Message) -> Vec<(&str, &str)> {
        message
            .parts
            .iter()
            .filter_map(|p| match p {
                ContextMessagePart::ToolCall { id, name, .. } => Some((id.as_str(), name.as_str())),
                _ => None,
            })
            .collect()
    }

    fn tool_results(message: &Message) -> Vec<(&str, &str, bool)> {
        message
            .parts
            .iter()
            .filter_map(|p| match p {
                ContextMessagePart::ToolResult {
                    id,
                    result,
                    is_error,
                } => Some((id.as_str(), result.as_str(), *is_error)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn sibling_call_and_result_fold_into_one_call_plus_one_result() {
        let message = map_api_message(&envelope(json!([
            tool_part(
                "p1",
                "call_01",
                Some("read"),
                json!({
                    "status": "pending",
                    "input": { "path": "src/main.rs" },
                    "raw": "",
                })
            ),
            tool_part(
                "p2",
                "call_01",
                None,
                json!({
                    "status": "completed",
                    "output": "fn main() {}",
                })
            ),
        ])));

        assert_eq!(
            tool_calls(&message),
            vec![("call_01", "read")],
            "the two siblings must collapse into a single tool call"
        );
        assert_eq!(
            tool_results(&message),
            vec![("call_01", "fn main() {}", false)],
            "the result must attach to the call it belongs to"
        );
        assert!(
            message.content.contains("[tool:read]"),
            "the folded call must reach the transcript text, got {:?}",
            message.content
        );
    }

    #[test]
    fn terminal_sibling_wins_even_when_it_arrives_before_the_call() {
        let message = map_api_message(&envelope(json!([
            tool_part(
                "p1",
                "call_01",
                None,
                json!({ "status": "completed", "output": "done" })
            ),
            tool_part(
                "p2",
                "call_01",
                Some("bash"),
                json!({ "status": "pending", "input": {} })
            ),
        ])));

        assert_eq!(
            tool_results(&message),
            vec![("call_01", "done", false)],
            "precedence is by lifecycle rank, not by arrival order"
        );
        assert_eq!(tool_calls(&message), vec![("call_01", "bash")]);
    }

    #[test]
    fn running_outranks_pending_and_yields_no_result() {
        let message = map_api_message(&envelope(json!([
            tool_part(
                "p1",
                "call_01",
                Some("edit"),
                json!({ "status": "running", "input": { "f": "a" } })
            ),
            tool_part(
                "p2",
                "call_01",
                None,
                json!({ "status": "pending", "input": {} })
            ),
        ])));

        assert_eq!(tool_calls(&message), vec![("call_01", "edit")]);
        assert!(
            tool_results(&message).is_empty(),
            "a non-terminal fold must not invent a result"
        );
    }

    #[test]
    fn error_status_marks_the_result_as_failed() {
        let message = map_api_message(&envelope(json!([
            tool_part(
                "p1",
                "call_01",
                Some("grep"),
                json!({ "status": "pending", "input": {} })
            ),
            tool_part(
                "p2",
                "call_01",
                None,
                json!({ "status": "error", "output": "no match" })
            ),
        ])));

        assert_eq!(tool_results(&message), vec![("call_01", "no match", true)]);
        assert!(
            message.content.contains("[tool-error]"),
            "an errored fold must say so in the text, got {:?}",
            message.content
        );
    }

    #[test]
    fn distinct_call_ids_stay_separate_invocations() {
        let message = map_api_message(&envelope(json!([
            tool_part(
                "p1",
                "call_01",
                Some("read"),
                json!({ "status": "completed", "output": "a" })
            ),
            tool_part(
                "p2",
                "call_02",
                Some("bash"),
                json!({ "status": "completed", "output": "b" })
            ),
        ])));

        assert_eq!(
            tool_calls(&message),
            vec![("call_01", "read"), ("call_02", "bash")]
        );
        assert_eq!(
            tool_results(&message),
            vec![("call_01", "a", false), ("call_02", "b", false)]
        );
    }

    #[test]
    fn folded_group_keeps_its_place_between_text_parts() {
        let message = map_api_message(&envelope(json!([
            text_part("t1", "before"),
            tool_part(
                "p1",
                "call_01",
                Some("read"),
                json!({ "status": "completed", "output": "a" })
            ),
            text_part("t2", "after"),
        ])));

        let kinds: Vec<&str> = message
            .parts
            .iter()
            .map(|p| match p {
                ContextMessagePart::Text { .. } => "text",
                ContextMessagePart::ToolCall { .. } => "call",
                ContextMessagePart::ToolResult { .. } => "result",
                _ => "other",
            })
            .collect();
        assert_eq!(kinds, vec!["text", "call", "result", "text"]);
    }

    #[test]
    fn identity_and_timestamp_come_from_the_envelope_info() {
        let message = map_api_message(&envelope(json!([text_part("t1", "hi")])));

        assert_eq!(message.id, "msg_01");
        assert_eq!(message.role, MessageRole::Assistant);
        assert_eq!(message.created_at.timestamp_millis(), 1_700_000_000_000_i64);
        assert_eq!(message.content, "hi");
    }

    #[test]
    fn an_envelope_with_no_parts_maps_to_an_empty_transcript_entry_not_a_crash() {
        let message = map_api_message(&envelope(json!([])));
        assert!(message.parts.is_empty());
        assert_eq!(message.content, "");
    }

    #[test]
    fn legacy_flat_tool_parts_still_fold_by_their_tool_call_id() {
        let message = map_api_message(&envelope(json!([
            {
                "id": "p1", "type": "toolCall",
                "toolCall": { "id": "call_09", "name": "read", "input": { "path": "a.rs" } },
            },
            {
                "id": "p2", "type": "toolResult",
                "toolResult": { "toolCallId": "call_09", "content": "body", "isError": false },
            },
        ])));

        assert_eq!(tool_calls(&message), vec![("call_09", "read")]);
        assert_eq!(tool_results(&message), vec![("call_09", "body", false)]);
    }

    #[test]
    fn file_parts_reach_the_transcript_with_their_real_server_shape() {
        let message = map_api_message(&envelope(json!([
            {
                "id": "prt_file", "type": "file",
                "url": "file:///tmp/trace.log", "filename": "trace.log", "mime": "text/plain",
            },
        ])));

        assert!(
            matches!(&message.parts[0], ContextMessagePart::File { path, mime }
            if path == "trace.log" && mime == "text/plain"),
            "the server spells file fields flat on the part; they must not be dropped, got {:?}",
            message.parts
        );
    }

    #[test]
    fn error_and_finish_come_from_nested_metadata_not_flat_info_fields() {
        let message = map_api_message(
            &serde_json::from_value(json!({
                "info": {
                    "id": "msg_01",
                    "sessionID": "ses_01",
                    "role": "assistant",
                    "createdAt": 1_700_000_000_000_i64,
                    "metadata": {
                        "error": "API error: 403 Forbidden",
                        "finish_reason": "error",
                        "model_provider": "opencode",
                        "model_id": "ling-3.1-flash-free",
                    },
                },
                "parts": [],
            }))
            .expect("envelope with nested metadata must parse"),
        );

        assert_eq!(
            message.error.as_deref(),
            Some("API error: 403 Forbidden"),
            "the server nests the error under info.metadata; a flat info.error is never sent"
        );
        assert_eq!(
            message.finish.as_deref(),
            Some("error"),
            "the server nests finish_reason under info.metadata"
        );
    }

    #[test]
    fn a_message_with_no_metadata_has_no_error_and_no_finish() {
        let message = map_api_message(&envelope(json!([])));
        assert_eq!(message.error, None);
        assert_eq!(message.finish, None);
    }

    #[test]
    fn ignored_and_synthetic_text_parts_stay_filtered_out() {
        let message = map_api_message(&envelope(json!([
            {
                "id": "t1", "type": "text", "text": "hidden",
                "synthetic": true, "ignored": false,
            },
            text_part("t2", "shown"),
        ])));

        assert_eq!(message.content, "shown");
    }
}
