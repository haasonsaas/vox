use ratatui::style::Color;

// ─── Envelope follower ─────────────────────────────────────
pub const ENVELOPE_ATTACK_WEIGHT: f64 = 0.8;
pub const ENVELOPE_ATTACK_CARRY: f64 = 0.2;
pub const ENVELOPE_DECAY_WEIGHT: f64 = 0.08;
pub const ENVELOPE_DECAY_CARRY: f64 = 0.92;

// ─── Waveform parameters ──────────────────────────────────
pub const WAVEFORM_TIME_SCALE: f64 = 0.08;
pub const WAVEFORM_TRANSCRIBING_TIME_SCALE: f64 = 0.04;
pub const WAVEFORM_IDLE_TIME_SCALE: f64 = 0.05;
pub const WAVEFORM_ENERGY_BOOST: f64 = 0.6;
pub const WAVEFORM_BREATH_MIN: f64 = 0.6;
pub const WAVEFORM_BREATH_RANGE: f64 = 0.4;
pub const WAVEFORM_IDLE_BREATH_MIN: f64 = 0.3;
pub const WAVEFORM_IDLE_BREATH_RANGE: f64 = 0.15;

// ─── Scan-line ────────────────────────────────────────────
pub const SCANLINE_PERIOD: u64 = 90;

// ─── Tick / framerate ─────────────────────────────────────
pub const TICK_RATE_MS: u64 = 33; // ~30fps

// ─── Transition timing ────────────────────────────────────
pub const TRANSITION_FADE_IN_TICKS: u64 = 10;
pub const TRANSITION_PULSE_TICKS: u64 = 3;

// ─── Audio model ──────────────────────────────────────────
pub const MIN_DURATION_SECONDS: f32 = 1.0;
pub const AUDIO_MODEL: &str = "gpt-4o-mini-transcribe";

// ─── Color palette ────────────────────────────────────────
pub mod colors {
    use ratatui::style::Color;

    pub const BLUE: Color = Color::Rgb(10, 132, 255);
    pub const PINK: Color = Color::Rgb(255, 55, 95);
    pub const GREEN: Color = Color::Rgb(48, 220, 155);
    pub const SURFACE: Color = Color::Rgb(18, 18, 22);
    pub const TEXT: Color = Color::Rgb(220, 220, 230);
    pub const TEXT_DIM: Color = Color::Rgb(130, 130, 145);
    pub const RED_PULSE: Color = Color::Rgb(255, 59, 48);
}

/// Linearly interpolate between two RGB colors. `t` is clamped to [0.0, 1.0].
pub fn lerp_color(a: Color, b: Color, t: f64) -> Color {
    let t = t.clamp(0.0, 1.0);
    match (a, b) {
        (Color::Rgb(r1, g1, b1), Color::Rgb(r2, g2, b2)) => Color::Rgb(
            (r1 as f64 + (r2 as f64 - r1 as f64) * t) as u8,
            (g1 as f64 + (g2 as f64 - g1 as f64) * t) as u8,
            (b1 as f64 + (b2 as f64 - b1 as f64) * t) as u8,
        ),
        _ => if t < 0.5 { a } else { b },
    }
}

/// Dim an RGB color by a factor (0.0 = black, 1.0 = unchanged).
pub fn dim_color(c: Color, factor: f64) -> Color {
    match c {
        Color::Rgb(r, g, b) => Color::Rgb(
            (r as f64 * factor) as u8,
            (g as f64 * factor) as u8,
            (b as f64 * factor) as u8,
        ),
        other => other,
    }
}
