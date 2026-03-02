use crate::ui::Mode;

pub struct AppState {
    pub mode: Mode,
    pub tick: u64,
    pub history: Vec<String>,
    pub auth_source: String,
}

impl AppState {
    pub fn new(auth_source: String) -> Self {
        Self {
            mode: Mode::Idle,
            tick: 0,
            history: Vec::new(),
            auth_source,
        }
    }
}
