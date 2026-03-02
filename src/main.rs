mod app;
mod audio;
mod constants;
mod error;
mod transcribe;
mod ui;
mod waveform;

use app::{AppState, Mode};
use clap::Parser;
use constants::*;
use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{self, EnterAlternateScreen, LeaveAlternateScreen},
};
use error::VoxError;
use ratatui::prelude::*;
use std::io;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

#[derive(Parser)]
#[command(name = "vox", about = "Beautiful voice-to-text transcription")]
struct Cli {
    /// OpenAI API key. Resolution order: this flag, CODEX_API_KEY, OPENAI_API_KEY,
    /// then ~/.codex/auth.json
    #[arg(long)]
    api_key: Option<String>,

    /// OpenAI API base URL (also reads OPENAI_BASE_URL env var)
    #[arg(long)]
    api_base: Option<String>,

    /// OpenAI organization ID (also reads OPENAI_ORGANIZATION env var)
    #[arg(long)]
    organization: Option<String>,

    /// OpenAI project ID (also reads OPENAI_PROJECT env var)
    #[arg(long)]
    project: Option<String>,

    /// Copy transcription to clipboard automatically
    #[arg(short, long)]
    clipboard: bool,

    /// Optional context prompt to improve transcription accuracy
    #[arg(long)]
    context: Option<String>,
}

struct ResolvedAuth {
    api_key: String,
    api_base: String,
    organization: Option<String>,
    project: Option<String>,
    source: String,
}

enum TranscriptionResult {
    Success { text: String },
    Error { message: String },
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Install panic hook to restore terminal on panic
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = terminal::disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        original_hook(info);
    }));

    let cli = Cli::parse();
    let auth = resolve_auth(&cli)?;

    // Setup terminal
    terminal::enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_app(&mut terminal, &cli, &auth).await;

    // Restore terminal
    terminal::disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    // Print final transcriptions to stdout for piping
    if let Ok(ref state) = result {
        for entry in &state.history {
            println!("{entry}");
        }
    }

    result.map(|_| ())
}

async fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    cli: &Cli,
    auth: &ResolvedAuth,
) -> Result<AppState, Box<dyn std::error::Error>> {
    let mut state = AppState::new(auth.source.clone());
    let mut capture: Option<audio::VoiceCapture> = None;
    let mut record_start: Option<Instant> = None;
    let mut env: f64 = 0.0;

    let tick_rate = Duration::from_millis(TICK_RATE_MS);
    let mut last_tick = Instant::now();

    // Channel for non-blocking transcription results
    let (tx, mut rx) = mpsc::channel::<TranscriptionResult>(4);

    loop {
        terminal.draw(|f| ui::draw(f, &state))?;

        // Check for completed transcriptions (non-blocking)
        if let Ok(result) = rx.try_recv() {
            match result {
                TranscriptionResult::Success { text } => {
                    let mut copied = false;
                    if cli.clipboard {
                        if let Ok(mut cb) = arboard::Clipboard::new() {
                            copied = cb.set_text(&text).is_ok();
                        }
                    }
                    state.history.push(text.clone());
                    state.set_mode(Mode::Result { text, copied });
                }
                TranscriptionResult::Error { message } => {
                    state.set_mode(Mode::Error { message });
                }
            }
        }

        let timeout = tick_rate.saturating_sub(last_tick.elapsed());
        if event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                // Quit
                if key.code == KeyCode::Char('q')
                    || key.code == KeyCode::Char('Q')
                    || (key.modifiers.contains(KeyModifiers::CONTROL)
                        && key.code == KeyCode::Char('c'))
                {
                    // Don't quit if device picker is open
                    if state.show_device_picker {
                        state.show_device_picker = false;
                        continue;
                    }
                    drop(capture.take());
                    return Ok(state);
                }

                // Device picker overlay takes priority
                if state.show_device_picker {
                    match key.code {
                        KeyCode::Up => {
                            if state.selected_device > 0 {
                                state.selected_device -= 1;
                            }
                        }
                        KeyCode::Down => {
                            if state.selected_device + 1 < state.input_devices.len() {
                                state.selected_device += 1;
                            }
                        }
                        KeyCode::Enter => {
                            state.show_device_picker = false;
                        }
                        KeyCode::Esc => {
                            state.show_device_picker = false;
                        }
                        _ => {}
                    }
                    continue;
                }

                match &state.mode {
                    Mode::Idle => match key.code {
                        KeyCode::Char(' ') => {
                            let device_name = state
                                .input_devices
                                .get(state.selected_device)
                                .map(|(name, _)| name.as_str());
                            match audio::VoiceCapture::start(device_name) {
                                Ok(c) => {
                                    capture = Some(c);
                                    record_start = Some(Instant::now());
                                    env = 0.0;
                                    state.set_mode(Mode::Recording {
                                        duration_secs: 0.0,
                                        energy: 0.0,
                                    });
                                }
                                Err(e) => {
                                    state.set_mode(Mode::Error {
                                        message: e.to_string(),
                                    });
                                }
                            }
                        }
                        KeyCode::Char('d') => {
                            state.input_devices = audio::list_input_devices();
                            // Find current default
                            if let Some(pos) = state
                                .input_devices
                                .iter()
                                .position(|(_, is_default)| *is_default)
                            {
                                state.selected_device = pos;
                            }
                            state.show_device_picker = true;
                        }
                        KeyCode::Up => {
                            if !state.history.is_empty() {
                                match state.history_selected {
                                    None => {
                                        state.history_selected =
                                            Some(state.history.len() - 1);
                                    }
                                    Some(i) if i > 0 => {
                                        state.history_selected = Some(i - 1);
                                    }
                                    _ => {}
                                }
                            }
                        }
                        KeyCode::Down => {
                            if let Some(i) = state.history_selected {
                                if i + 1 < state.history.len() {
                                    state.history_selected = Some(i + 1);
                                } else {
                                    state.history_selected = None;
                                }
                            }
                        }
                        KeyCode::Esc => {
                            state.history_selected = None;
                        }
                        KeyCode::Char('c') | KeyCode::Char('C') | KeyCode::Enter => {
                            if let Some(i) = state.history_selected {
                                if let Some(text) = state.history.get(i) {
                                    if let Ok(mut cb) = arboard::Clipboard::new() {
                                        let _ = cb.set_text(text);
                                    }
                                }
                            }
                        }
                        _ => {}
                    },
                    Mode::Recording { .. } => {
                        if key.code == KeyCode::Char(' ') {
                            let recorded = capture
                                .take()
                                .ok_or_else(|| {
                                    VoxError::Audio("no capture in progress".to_string())
                                })?
                                .stop()?;
                            let duration = audio::clip_duration_seconds(&recorded);
                            state.set_mode(Mode::Transcribing {
                                duration_secs: duration,
                            });
                            record_start = None;

                            // Store recording for potential save
                            state.last_recording = Some(audio::RecordedAudio {
                                data: recorded.data.clone(),
                                sample_rate: recorded.sample_rate,
                                channels: recorded.channels,
                            });

                            // Spawn non-blocking transcription task
                            let tx = tx.clone();
                            let api_key = auth.api_key.clone();
                            let api_base = auth.api_base.clone();
                            let organization = auth.organization.clone();
                            let project = auth.project.clone();
                            let context = cli.context.clone();
                            tokio::spawn(async move {
                                let result = transcribe::transcribe(
                                    recorded,
                                    &api_key,
                                    &api_base,
                                    organization.as_deref(),
                                    project.as_deref(),
                                    context.as_deref(),
                                )
                                .await;
                                let msg = match result {
                                    Ok(text) => TranscriptionResult::Success { text },
                                    Err(e) => TranscriptionResult::Error {
                                        message: e.to_string(),
                                    },
                                };
                                let _ = tx.send(msg).await;
                            });
                        }
                    }
                    Mode::Transcribing { .. } => {
                        // Can't interrupt — waveform keeps animating
                    }
                    Mode::Result { ref text, .. } => match key.code {
                        KeyCode::Char(' ') => {
                            state.set_mode(Mode::Idle);
                        }
                        KeyCode::Char('c') | KeyCode::Char('C') => {
                            if let Ok(mut cb) = arboard::Clipboard::new() {
                                let _ = cb.set_text(text);
                            }
                            state.set_mode(Mode::Result {
                                text: text.clone(),
                                copied: true,
                            });
                        }
                        KeyCode::Char('s') => {
                            // Save transcript to file
                            let ts = chrono::Local::now().format("%Y%m%d_%H%M%S");
                            let filename = format!("vox_{ts}.txt");
                            if let Err(e) = std::fs::write(&filename, text) {
                                state.set_mode(Mode::Error {
                                    message: format!("save failed: {e}"),
                                });
                            }
                        }
                        KeyCode::Char('w') => {
                            // Save WAV to disk
                            if let Some(ref recording) = state.last_recording {
                                let ts = chrono::Local::now().format("%Y%m%d_%H%M%S");
                                let filename = format!("vox_{ts}.wav");
                                match audio::encode_wav_raw(recording) {
                                    Ok(wav_bytes) => {
                                        if let Err(e) = std::fs::write(&filename, wav_bytes) {
                                            state.set_mode(Mode::Error {
                                                message: format!("save failed: {e}"),
                                            });
                                        }
                                    }
                                    Err(e) => {
                                        state.set_mode(Mode::Error {
                                            message: format!("wav encode failed: {e}"),
                                        });
                                    }
                                }
                            }
                        }
                        KeyCode::Up => {
                            if state.result_scroll > 0 {
                                state.result_scroll -= 1;
                            }
                        }
                        KeyCode::Down => {
                            state.result_scroll += 1;
                        }
                        _ => {}
                    },
                    Mode::Error { .. } => {
                        if key.code == KeyCode::Char(' ') {
                            state.set_mode(Mode::Idle);
                        }
                    }
                }
            }
        }

        if last_tick.elapsed() >= tick_rate {
            state.tick += 1;

            // Update recording state
            if let Some(ref cap) = capture {
                if let Some(start) = record_start {
                    let duration = start.elapsed().as_secs_f32();
                    let peak = cap.last_peak() as f64 / i16::MAX as f64;

                    if peak > env {
                        env = ENVELOPE_ATTACK_WEIGHT * peak + ENVELOPE_ATTACK_CARRY * env;
                    } else {
                        env = ENVELOPE_DECAY_WEIGHT * peak + ENVELOPE_DECAY_CARRY * env;
                    }

                    state.mode = Mode::Recording {
                        duration_secs: duration,
                        energy: env,
                    };
                }
            }

            last_tick = Instant::now();
        }
    }
}

// ─── Auth resolution ────────────────────────────────────────

fn resolve_auth(cli: &Cli) -> Result<ResolvedAuth, VoxError> {
    let (api_key, source) = if let Some(ref key) = cli.api_key {
        (key.clone(), "--api-key".to_string())
    } else if let Ok(key) = std::env::var("CODEX_API_KEY") {
        if !key.is_empty() {
            (key, "CODEX_API_KEY".to_string())
        } else {
            try_openai_key_or_auth_json()?
        }
    } else {
        try_openai_key_or_auth_json()?
    };

    let api_base = cli
        .api_base
        .clone()
        .or_else(|| std::env::var("OPENAI_BASE_URL").ok().filter(|s| !s.is_empty()))
        .unwrap_or_else(|| "https://api.openai.com/v1".to_string());

    let organization = cli
        .organization
        .clone()
        .or_else(|| std::env::var("OPENAI_ORGANIZATION").ok().filter(|s| !s.is_empty()));

    let project = cli
        .project
        .clone()
        .or_else(|| std::env::var("OPENAI_PROJECT").ok().filter(|s| !s.is_empty()));

    Ok(ResolvedAuth {
        api_key,
        api_base,
        organization,
        project,
        source,
    })
}

fn try_openai_key_or_auth_json() -> Result<(String, String), VoxError> {
    if let Ok(key) = std::env::var("OPENAI_API_KEY") {
        if !key.is_empty() {
            return Ok((key, "OPENAI_API_KEY".to_string()));
        }
    }

    if let Some(home) = std::env::var("HOME").ok().map(std::path::PathBuf::from) {
        let auth_file = home.join(".codex").join("auth.json");
        if auth_file.exists() {
            let contents = std::fs::read_to_string(&auth_file)
                .map_err(|e| VoxError::Terminal(e.to_string()))?;
            let v: serde_json::Value = serde_json::from_str(&contents)
                .map_err(|e| VoxError::Terminal(e.to_string()))?;
            if let Some(key) = v.get("OPENAI_API_KEY").and_then(|k| k.as_str()) {
                if !key.is_empty() {
                    return Ok((key.to_string(), format!("{}", auth_file.display())));
                }
            }
        }
    }

    Err(VoxError::NoApiKey)
}
