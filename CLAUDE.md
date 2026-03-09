# Vox - Voice-to-Text TUI

## Quick Reference
```bash
cargo build               # Build
cargo run                 # Run the TUI
cargo test                # Run tests
```

## Architecture
- **Language**: Rust
- **TUI framework**: ratatui + crossterm
- **Audio capture**: cpal
- **Transcription**: OpenAI Whisper API (via reqwest)
- **Clipboard**: arboard

## Key Files
- `src/main.rs` — Entry point and app setup
- `src/` — TUI rendering, audio capture, API integration

## Design Goals
- Should look "magical" — not a boring terminal app
- Visual feedback during recording (waveforms, indicators)
- Instant copy-to-clipboard after transcription
- Minimal keystrokes to record → transcribe → use

## Environment
- Requires `OPENAI_API_KEY` for Whisper transcription
- Audio input device must be available (microphone permissions on macOS)
