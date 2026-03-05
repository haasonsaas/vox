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
        partial_text: String,
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

    // Transient flash message (e.g. "saved vox_20260302.txt")
    pub flash_message: Option<String>,
    pub flash_tick: u64,

    // Don't persist history to disk
    pub no_history: bool,

    // Auto-copy to clipboard
    pub auto_copy: bool,

    // Waveform morph: energy at the moment recording ended
    pub transition_energy: f64,

    // Voice activity detection: consecutive low-energy ticks
    pub silence_ticks: u64,
    pub silence_timeout_ticks: u64, // 0 = disabled
}

const FLASH_DURATION_TICKS: u64 = 60; // ~2 seconds at 30fps

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
            flash_message: None,
            flash_tick: 0,
            no_history: false,
            auto_copy: false,
            transition_energy: 0.0,
            silence_ticks: 0,
            silence_timeout_ticks: 0,
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

    /// Show a transient status message that auto-expires.
    pub fn flash(&mut self, msg: String) {
        self.flash_message = Some(msg);
        self.flash_tick = self.tick;
    }

    /// Get the current flash message if still active.
    pub fn active_flash(&self) -> Option<&str> {
        if let Some(ref msg) = self.flash_message {
            if self.tick.saturating_sub(self.flash_tick) < FLASH_DURATION_TICKS {
                return Some(msg);
            }
        }
        None
    }
}
