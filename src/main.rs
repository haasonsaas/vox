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
use std::io::{self, Write as _};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use transcribe::StreamEvent;

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

    /// One-shot mode: record once, print transcription to stdout, exit
    #[arg(long)]
    oneshot: bool,

    /// Clear history before starting
    #[arg(long)]
    clear_history: bool,

    /// Don't load or save history
    #[arg(long)]
    no_history: bool,

    /// Auto-stop recording after N seconds of silence (0 = disabled)
    #[arg(long, default_value_t = constants::DEFAULT_SILENCE_TIMEOUT_SECS)]
    silence_timeout: f32,
}

struct ResolvedAuth {
    api_key: String,
    api_base: String,
    organization: Option<String>,
    project: Option<String>,
    /// ChatGPT OAuth account ID (sent as chatgpt-account-id header)
    account_id: Option<String>,
    source: String,
}

const HISTORY_FILE: &str = ".vox_history";
const MAX_HISTORY_ENTRIES: usize = 100;

fn history_path() -> Option<std::path::PathBuf> {
    std::env::var("HOME")
        .ok()
        .map(|h| std::path::PathBuf::from(h).join(HISTORY_FILE))
}

fn load_history() -> Vec<String> {
    history_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|contents| {
            contents
                .lines()
                .filter(|l| !l.is_empty())
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

fn save_history(history: &[String]) {
    if let Some(path) = history_path() {
        let entries: Vec<&String> = history.iter().rev().take(MAX_HISTORY_ENTRIES).collect();
        let text: String = entries
            .into_iter()
            .rev()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let _ = std::fs::write(path, text + "\n");
    }
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

    // One-shot mode: record, transcribe, print, exit — no TUI
    if cli.oneshot {
        return run_oneshot(&cli, &auth).await;
    }

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

async fn run_oneshot(
    cli: &Cli,
    auth: &ResolvedAuth,
) -> Result<(), Box<dyn std::error::Error>> {
    eprint!("Recording... press Ctrl+C to stop. ");

    let capture = audio::VoiceCapture::start(None)?;

    // Wait for Ctrl+C
    tokio::signal::ctrl_c().await?;
    eprintln!("done.");

    let recorded = capture.stop()?;
    eprint!("Transcribing... ");

    let text = transcribe::transcribe(
        recorded,
        &auth.api_key,
        &auth.api_base,
        auth.organization.as_deref(),
        auth.project.as_deref(),
        auth.account_id.as_deref(),
        cli.context.as_deref(),
    )
    .await?;

    eprintln!("done.");

    if cli.clipboard {
        if let Ok(mut cb) = arboard::Clipboard::new() {
            let _ = cb.set_text(&text);
        }
    }

    io::stdout().write_all(text.as_bytes())?;
    io::stdout().write_all(b"\n")?;
    Ok(())
}

async fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    cli: &Cli,
    auth: &ResolvedAuth,
) -> Result<AppState, Box<dyn std::error::Error>> {
    let mut state = AppState::new(auth.source.clone());
    state.no_history = cli.no_history;
    state.auto_copy = cli.clipboard;
    if cli.silence_timeout > 0.0 {
        state.silence_timeout_ticks =
            (cli.silence_timeout / (TICK_RATE_MS as f32 / 1000.0)) as u64;
    }
    if cli.clear_history {
        if let Some(path) = history_path() {
            let _ = std::fs::remove_file(path);
        }
    }
    if !cli.no_history && !cli.clear_history {
        state.history = load_history();
    }

    let mut capture: Option<audio::VoiceCapture> = None;
    let mut record_start: Option<Instant> = None;
    let mut env: f64 = 0.0;

    let tick_rate = Duration::from_millis(TICK_RATE_MS);
    let mut last_tick = Instant::now();

    // Channel for streaming transcription events
    let (tx, mut rx) = mpsc::channel::<StreamEvent>(64);

    loop {
        terminal.draw(|f| ui::draw(f, &state))?;

        // Check for streaming transcription events (non-blocking, drain all available)
        while let Ok(event) = rx.try_recv() {
            match event {
                StreamEvent::Delta(delta) => {
                    if let Mode::Transcribing {
                        ref mut partial_text,
                        ..
                    } = state.mode
                    {
                        partial_text.push_str(&delta);
                    }
                }
                StreamEvent::Done(text) => {
                    let mut copied = false;
                    if cli.clipboard {
                        if let Ok(mut cb) = arboard::Clipboard::new() {
                            copied = cb.set_text(&text).is_ok();
                        }
                    }
                    state.history.push(text.clone());
                    if !state.no_history {
                        save_history(&state.history);
                    }
                    state.set_mode(Mode::Result { text, copied });
                }
                StreamEvent::Error(message) => {
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
                    // Don't quit during recording — treat as cancel
                    if matches!(state.mode, Mode::Recording { .. }) {
                        drop(capture.take());
                        record_start = None;
                        state.set_mode(Mode::Idle);
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
                            start_recording(&mut state, &mut capture, &mut record_start, &mut env);
                        }
                        KeyCode::Char('d') => {
                            state.input_devices = audio::list_input_devices();
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
                                        if cb.set_text(text).is_ok() {
                                            state.flash("copied".to_string());
                                        }
                                    }
                                }
                            }
                        }
                        KeyCode::Char('x') => {
                            // Delete selected history entry
                            if let Some(i) = state.history_selected {
                                if i < state.history.len() {
                                    state.history.remove(i);
                                    if !state.no_history {
                                        save_history(&state.history);
                                    }
                                    if state.history.is_empty() {
                                        state.history_selected = None;
                                    } else if i >= state.history.len() {
                                        state.history_selected =
                                            Some(state.history.len() - 1);
                                    }
                                    state.flash("deleted".to_string());
                                }
                            }
                        }
                        KeyCode::Char('X') => {
                            // Clear all history
                            if !state.history.is_empty() {
                                state.history.clear();
                                state.history_selected = None;
                                if !state.no_history {
                                    save_history(&state.history);
                                }
                                state.flash("history cleared".to_string());
                            }
                        }
                        _ => {}
                    },
                    Mode::Recording { .. } => match key.code {
                        KeyCode::Char(' ') => {
                            stop_and_transcribe(
                                &mut state,
                                &mut capture,
                                &mut record_start,
                                &env,
                                &tx,
                                auth,
                                cli,
                            )?;
                        }
                        KeyCode::Esc => {
                            // Cancel recording without transcribing
                            drop(capture.take());
                            record_start = None;
                            state.set_mode(Mode::Idle);
                        }
                        _ => {}
                    },
                    Mode::Transcribing { .. } => {
                        // Waveform keeps animating, can't interrupt
                    }
                    Mode::Result { .. } => {
                        // Clone text out to avoid borrow issues
                        let text = if let Mode::Result { ref text, .. } = state.mode {
                            text.clone()
                        } else {
                            unreachable!()
                        };
                        match key.code {
                            KeyCode::Char(' ') => {
                                state.set_mode(Mode::Idle);
                            }
                            KeyCode::Char('r') => {
                                start_recording(
                                    &mut state,
                                    &mut capture,
                                    &mut record_start,
                                    &mut env,
                                );
                            }
                            KeyCode::Char('c') | KeyCode::Char('C') => {
                                if let Ok(mut cb) = arboard::Clipboard::new() {
                                    if cb.set_text(&text).is_ok() {
                                        state.flash("copied".to_string());
                                    }
                                }
                                state.set_mode(Mode::Result {
                                    text,
                                    copied: true,
                                });
                            }
                            KeyCode::Char('s') => {
                                let ts = chrono::Local::now().format("%Y%m%d_%H%M%S");
                                let filename = format!("vox_{ts}.txt");
                                match std::fs::write(&filename, &text) {
                                    Ok(()) => state.flash(format!("saved {filename}")),
                                    Err(e) => {
                                        state.set_mode(Mode::Error {
                                            message: format!("save failed: {e}"),
                                        });
                                    }
                                }
                            }
                            KeyCode::Char('w') => {
                                if let Some(ref recording) = state.last_recording {
                                    let ts = chrono::Local::now().format("%Y%m%d_%H%M%S");
                                    let filename = format!("vox_{ts}.wav");
                                    match audio::encode_wav_raw(recording) {
                                        Ok(wav_bytes) => {
                                            match std::fs::write(&filename, wav_bytes) {
                                                Ok(()) => {
                                                    state.flash(format!("saved {filename}"))
                                                }
                                                Err(e) => {
                                                    state.set_mode(Mode::Error {
                                                        message: format!("save failed: {e}"),
                                                    });
                                                }
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
                                state.result_scroll = state.result_scroll.saturating_add(1);
                            }
                            _ => {}
                        }
                    }
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

                    // VAD: track silence and auto-stop
                    if state.silence_timeout_ticks > 0 {
                        if env < VAD_ENERGY_THRESHOLD {
                            state.silence_ticks += 1;
                        } else {
                            state.silence_ticks = 0;
                        }
                        // Only auto-stop after at least MIN_DURATION_SECONDS of recording
                        if state.silence_ticks >= state.silence_timeout_ticks
                            && duration >= MIN_DURATION_SECONDS
                        {
                            stop_and_transcribe(
                                &mut state,
                                &mut capture,
                                &mut record_start,
                                &env,
                                &tx,
                                auth,
                                cli,
                            )?;
                        }
                    }
                }
            }

            last_tick = Instant::now();
        }
    }
}

fn start_recording(
    state: &mut AppState,
    capture: &mut Option<audio::VoiceCapture>,
    record_start: &mut Option<Instant>,
    env: &mut f64,
) {
    let device_name = state
        .input_devices
        .get(state.selected_device)
        .map(|(name, _)| name.as_str());
    match audio::VoiceCapture::start(device_name) {
        Ok(c) => {
            *capture = Some(c);
            *record_start = Some(Instant::now());
            *env = 0.0;
            state.silence_ticks = 0;
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

fn stop_and_transcribe(
    state: &mut AppState,
    capture: &mut Option<audio::VoiceCapture>,
    record_start: &mut Option<Instant>,
    env: &f64,
    tx: &mpsc::Sender<StreamEvent>,
    auth: &ResolvedAuth,
    cli: &Cli,
) -> Result<(), Box<dyn std::error::Error>> {
    let recorded = capture
        .take()
        .ok_or_else(|| VoxError::Audio("no capture in progress".to_string()))?
        .stop()?;
    let duration = audio::clip_duration_seconds(&recorded);

    // Capture energy for morph animation
    state.transition_energy = *env;

    state.set_mode(Mode::Transcribing {
        duration_secs: duration,
        partial_text: String::new(),
    });
    *record_start = None;

    state.last_recording = Some(audio::RecordedAudio {
        data: recorded.data.clone(),
        sample_rate: recorded.sample_rate,
        channels: recorded.channels,
    });

    // Spawn streaming transcription task
    let tx = tx.clone();
    let api_key = auth.api_key.clone();
    let api_base = auth.api_base.clone();
    let organization = auth.organization.clone();
    let project = auth.project.clone();
    let account_id = auth.account_id.clone();
    let context = cli.context.clone();
    tokio::spawn(async move {
        transcribe::transcribe_streaming(
            recorded,
            &api_key,
            &api_base,
            organization.as_deref(),
            project.as_deref(),
            account_id.as_deref(),
            context.as_deref(),
            tx,
        )
        .await;
    });
    Ok(())
}

// ─── Auth resolution ────────────────────────────────────────

fn resolve_auth(cli: &Cli) -> Result<ResolvedAuth, VoxError> {
    // Auth resolution order:
    //   1. --api-key flag
    //   2. CODEX_API_KEY env var
    //   3. OPENAI_API_KEY env var
    //   4. ~/.codex/auth.json → OPENAI_API_KEY field (auth_mode: "apikey")
    //   5. ~/.codex/auth.json → tokens.access_token (auth_mode: "chatgpt")
    let auth_json = read_codex_auth_json();

    let (api_key, account_id, source) = if let Some(ref key) = cli.api_key {
        (key.clone(), None, "--api-key".to_string())
    } else if let Ok(key) = std::env::var("CODEX_API_KEY") {
        if !key.is_empty() {
            (key, None, "CODEX_API_KEY".to_string())
        } else {
            try_resolve_key(&auth_json)?
        }
    } else {
        try_resolve_key(&auth_json)?
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
        account_id,
        source,
    })
}

/// Read and parse ~/.codex/auth.json if it exists.
fn read_codex_auth_json() -> Option<serde_json::Value> {
    let home = std::env::var("HOME").ok()?;
    let auth_file = std::path::PathBuf::from(home).join(".codex").join("auth.json");
    let contents = std::fs::read_to_string(auth_file).ok()?;
    serde_json::from_str(&contents).ok()
}

/// Try env vars and auth.json to find a usable API key/token.
/// Returns (key, optional_account_id, source_description).
fn try_resolve_key(
    auth_json: &Option<serde_json::Value>,
) -> Result<(String, Option<String>, String), VoxError> {
    // OPENAI_API_KEY env var
    if let Ok(key) = std::env::var("OPENAI_API_KEY") {
        if !key.is_empty() {
            return Ok((key, None, "OPENAI_API_KEY".to_string()));
        }
    }

    // ~/.codex/auth.json
    if let Some(v) = auth_json {
        let auth_mode = v.get("auth_mode").and_then(|m| m.as_str()).unwrap_or("");

        // API key mode: OPENAI_API_KEY field is set
        if auth_mode == "apikey" {
            if let Some(key) = v.get("OPENAI_API_KEY").and_then(|k| k.as_str()) {
                if !key.is_empty() {
                    return Ok((key.to_string(), None, "codex (api key)".to_string()));
                }
            }
        }

        // ChatGPT OAuth mode: use tokens.access_token
        if auth_mode == "chatgpt" {
            if let Some(tokens) = v.get("tokens") {
                if let Some(access_token) = tokens.get("access_token").and_then(|t| t.as_str()) {
                    if !access_token.is_empty() {
                        let account_id = tokens
                            .get("account_id")
                            .and_then(|a| a.as_str())
                            .map(String::from);
                        return Ok((
                            access_token.to_string(),
                            account_id,
                            "codex (chatgpt)".to_string(),
                        ));
                    }
                }
            }
        }

        // Fallback: try OPENAI_API_KEY regardless of auth_mode
        if let Some(key) = v.get("OPENAI_API_KEY").and_then(|k| k.as_str()) {
            if !key.is_empty() {
                return Ok((key.to_string(), None, "codex auth.json".to_string()));
            }
        }
    }

    Err(VoxError::NoApiKey)
}
