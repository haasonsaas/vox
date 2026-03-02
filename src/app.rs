use crate::audio::RecordedAudio;

#[derive(Clone)]
pub enum Mode {
    Idle,
    Recording {
        duration_secs: f32,
        energy: f64,
    },
    Transcribing {
        duration_secs: f32,
    },
    Result {
        text: String,
        copied: bool,
    },
    Error {
        message: String,
    },
}

#[allow(dead_code)]
pub struct AppState {
    pub mode: Mode,
    pub tick: u64,
    pub history: Vec<String>,
    pub auth_source: String,

    // History navigation
    pub history_selected: Option<usize>,

    // Result scrolling
    pub result_scroll: u16,

    // Transition tracking
    pub transition_tick: u64,

    // Device picker
    pub input_devices: Vec<(String, bool)>,
    pub selected_device: usize,
    pub show_device_picker: bool,

    // Last recording for save-to-file
    pub last_recording: Option<RecordedAudio>,
}

impl AppState {
    pub fn new(auth_source: String) -> Self {
        Self {
            mode: Mode::Idle,
            tick: 0,
            history: Vec::new(),
            auth_source,
            history_selected: None,
            result_scroll: 0,
            transition_tick: 0,
            input_devices: Vec::new(),
            selected_device: 0,
            show_device_picker: false,
            last_recording: None,
        }
    }

    /// Set mode and record the tick for transition animations.
    pub fn set_mode(&mut self, mode: Mode) {
        self.transition_tick = self.tick;
        self.result_scroll = 0;
        self.mode = mode;
    }

    /// How many ticks since the last mode transition.
    pub fn transition_age(&self) -> u64 {
        self.tick.saturating_sub(self.transition_tick)
    }
}
