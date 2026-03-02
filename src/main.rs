mod app;
mod audio;
mod transcribe;
mod ui;
mod waveform;

use app::AppState;
use clap::Parser;
use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{self, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::prelude::*;
use std::io;
use std::time::{Duration, Instant};
use ui::Mode;

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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    let auth = resolve_auth(&cli)?;

    // Setup terminal
    terminal::enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_app(&mut terminal, &cli, &auth);

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

fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    cli: &Cli,
    auth: &ResolvedAuth,
) -> Result<AppState, Box<dyn std::error::Error>> {
    let mut state = AppState::new(auth.source.clone());
    let mut capture: Option<audio::VoiceCapture> = None;
    let mut record_start: Option<Instant> = None;
    let mut env: f64 = 0.0; // envelope follower for waveform energy

    let tick_rate = Duration::from_millis(33); // ~30fps
    let mut last_tick = Instant::now();

    loop {
        terminal.draw(|f| ui::draw(f, &state))?;

        let timeout = tick_rate.saturating_sub(last_tick.elapsed());
        if event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                // Quit
                if key.code == KeyCode::Char('q')
                    || key.code == KeyCode::Char('Q')
                    || (key.modifiers.contains(KeyModifiers::CONTROL)
                        && key.code == KeyCode::Char('c'))
                {
                    // Drop capture cleanly
                    drop(capture.take());
                    return Ok(state);
                }

                match &state.mode {
                    Mode::Idle => {
                        if key.code == KeyCode::Char(' ') {
                            match audio::VoiceCapture::start() {
                                Ok(c) => {
                                    capture = Some(c);
                                    record_start = Some(Instant::now());
                                    env = 0.0;
                                    state.mode = Mode::Recording {
                                        duration_secs: 0.0,
                                        energy: 0.0,
                                    };
                                }
                                Err(e) => {
                                    state.mode = Mode::Error {
                                        message: e,
                                    };
                                }
                            }
                        }
                    }
                    Mode::Recording { .. } => {
                        if key.code == KeyCode::Char(' ') {
                            let recorded = capture
                                .take()
                                .ok_or("no capture")?
                                .stop()?;
                            let duration = audio::clip_duration_seconds(&recorded);
                            state.mode = Mode::Transcribing {
                                duration_secs: duration,
                            };
                            record_start = None;

                            // Run transcription (blocking for simplicity)
                            let rt = tokio::runtime::Runtime::new()?;
                            let result = rt.block_on(transcribe::transcribe(
                                recorded,
                                &auth.api_key,
                                &auth.api_base,
                                auth.organization.as_deref(),
                                auth.project.as_deref(),
                                cli.context.as_deref(),
                            ));

                            match result {
                                Ok(text) => {
                                    let mut copied = false;
                                    if cli.clipboard {
                                        if let Ok(mut cb) = arboard::Clipboard::new() {
                                            copied = cb.set_text(&text).is_ok();
                                        }
                                    }
                                    state.history.push(text.clone());
                                    state.mode = Mode::Result { text, copied };
                                }
                                Err(e) => {
                                    state.mode = Mode::Error { message: e };
                                }
                            }
                        }
                    }
                    Mode::Transcribing { .. } => {
                        // Can't interrupt transcription
                    }
                    Mode::Result { ref text, .. } => {
                        if key.code == KeyCode::Char(' ') {
                            state.mode = Mode::Idle;
                        } else if key.code == KeyCode::Char('c') || key.code == KeyCode::Char('C')
                        {
                            if let Ok(mut cb) = arboard::Clipboard::new() {
                                let _ = cb.set_text(text);
                            }
                            state.mode = Mode::Result {
                                text: text.clone(),
                                copied: true,
                            };
                        }
                    }
                    Mode::Error { .. } => {
                        if key.code == KeyCode::Char(' ') {
                            state.mode = Mode::Idle;
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

                    // Envelope follower (same as Lark's energy system)
                    if peak > env {
                        env = 0.8 * peak + 0.2 * env;
                    } else {
                        env = 0.08 * peak + 0.92 * env;
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

fn resolve_auth(cli: &Cli) -> Result<ResolvedAuth, Box<dyn std::error::Error>> {
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

fn try_openai_key_or_auth_json() -> Result<(String, String), Box<dyn std::error::Error>> {
    if let Ok(key) = std::env::var("OPENAI_API_KEY") {
        if !key.is_empty() {
            return Ok((key, "OPENAI_API_KEY".to_string()));
        }
    }

    if let Some(home) = std::env::var("HOME").ok().map(std::path::PathBuf::from) {
        let auth_file = home.join(".codex").join("auth.json");
        if auth_file.exists() {
            let contents = std::fs::read_to_string(&auth_file)?;
            let v: serde_json::Value = serde_json::from_str(&contents)?;
            if let Some(key) = v.get("OPENAI_API_KEY").and_then(|k| k.as_str()) {
                if !key.is_empty() {
                    return Ok((key.to_string(), format!("{}", auth_file.display())));
                }
            }
        }
    }

    Err("no API key found. Set OPENAI_API_KEY, CODEX_API_KEY, use --api-key, or run `codex login`"
        .into())
}
