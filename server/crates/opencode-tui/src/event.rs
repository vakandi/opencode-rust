use crossterm::event::{KeyEvent, MouseEvent};

#[derive(Clone, Debug)]
pub enum Event {
    Key(KeyEvent),
    Mouse(MouseEvent),
    Resize(u16, u16),
    Tick,
    FocusGained,
    FocusLost,
    Paste(String),
    Custom(CustomEvent),
}

#[derive(Clone, Debug)]
pub enum CustomEvent {
    Message(String),
    StreamChunk(String),
    StreamComplete,
    StreamError(String),
    ToolCallStart { id: String, name: String },
    ToolCallComplete { id: String, result: String },
    StateChanged(StateChange),
}

#[derive(Clone, Debug)]
pub enum StateChange {
    SessionCreated(String),
    SessionUpdated(String),
    SessionStatusBusy(String),
    SessionStatusIdle(String),
    SessionStatusRetrying {
        session_id: String,
        attempt: u32,
        message: String,
        next: i64,
    },
    SessionDeleted(String),
    ModelChanged(String),
    AgentChanged(String),
    ProviderConnected(String),
    ProviderDisconnected(String),
    McpServerStatusChanged {
        name: String,
        status: String,
    },
    TodoUpdated,
    DiffUpdated,
}

pub struct EventBus {
    tx: std::sync::mpsc::Sender<Event>,
}

impl Clone for EventBus {
    fn clone(&self) -> Self {
        Self {
            tx: self.tx.clone(),
        }
    }
}

impl EventBus {
    pub fn new() -> Self {
        let (tx, _rx) = std::sync::mpsc::channel();
        Self { tx }
    }

    pub fn sender(&self) -> std::sync::mpsc::Sender<Event> {
        self.tx.clone()
    }

    pub fn send(&self, event: Event) {
        let _ = self.tx.send(event);
    }

    pub fn send_custom(&self, event: CustomEvent) {
        let _ = self.tx.send(Event::Custom(event));
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}
