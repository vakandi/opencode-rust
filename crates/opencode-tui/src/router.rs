use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Route {
    Home,
    Session { session_id: String },
    Settings,
    Help,
}

impl Default for Route {
    fn default() -> Self {
        Self::Home
    }
}

impl std::fmt::Display for Route {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Route::Home => write!(f, "Home"),
            Route::Session { session_id } => write!(f, "Session: {}", session_id),
            Route::Settings => write!(f, "Settings"),
            Route::Help => write!(f, "Help"),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Router {
    history: Vec<Route>,
    current: Route,
}

impl Router {
    pub fn new() -> Self {
        Self {
            history: vec![Route::Home],
            current: Route::Home,
        }
    }

    pub fn current(&self) -> &Route {
        &self.current
    }

    pub fn navigate(&mut self, route: Route) {
        if self.current != route {
            self.history.push(self.current.clone());
            self.current = route;
        }
    }

    pub fn go_back(&mut self) -> bool {
        if self.history.len() > 1 {
            self.history.pop();
            if let Some(prev) = self.history.last() {
                self.current = prev.clone();
                return true;
            }
        }
        false
    }

    pub fn is_home(&self) -> bool {
        matches!(self.current, Route::Home)
    }

    pub fn is_session(&self) -> bool {
        matches!(self.current, Route::Session { .. })
    }

    pub fn session_id(&self) -> Option<&str> {
        match &self.current {
            Route::Session { session_id } => Some(session_id),
            _ => None,
        }
    }
}
