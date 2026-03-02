use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::widgets::Widget;

/// Lark-inspired multi-layered sine wave visualization.
/// Renders 3 colored sine waves (blue, pink, green) with composite harmonics,
/// phase offsets, and amplitude modulation driven by audio energy.
pub struct Waveform {
    /// Current time for animation (incremented each frame)
    pub t: f64,
    /// Audio energy level [0.0, 1.0] — drives wave amplitude
    pub energy: f64,
}

struct WaveLayer {
    color: Color,
    freq: f64,
    phase: f64,
    speed: f64,
    amplitude: f64,
}

const LAYERS: [WaveLayer; 3] = [
    WaveLayer {
        color: Color::Rgb(10, 132, 255),   // Lark blue
        freq: 2.0,
        phase: 0.0,
        speed: 1.0,
        amplitude: 1.0,
    },
    WaveLayer {
        color: Color::Rgb(255, 55, 95),    // Lark pink
        freq: 2.5,
        phase: std::f64::consts::FRAC_PI_2,
        speed: 0.8,
        amplitude: 0.75,
    },
    WaveLayer {
        color: Color::Rgb(48, 220, 155),   // Lark green
        freq: 1.8,
        phase: std::f64::consts::PI,
        speed: 1.2,
        amplitude: 0.65,
    },
];

// Upper-half block characters for sub-cell vertical resolution.
// We use full block + upper-half block for 2-level resolution per cell.
const BLOCK_FULL: &str = "█";
const BLOCK_UPPER: &str = "▀";
const BLOCK_LOWER: &str = "▄";

impl Widget for &Waveform {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let w = area.width as usize;
        let h = area.height as usize;
        let mid_y = h as f64 / 2.0;

        // Breathing modulation — gentle amplitude oscillation
        let breath = 0.6 + 0.4 * (self.t * 0.5).sin();
        // Energy boost from audio input
        let energy_boost = self.energy * 0.6;

        for layer in &LAYERS {
            let base_amp = layer.amplitude * breath * mid_y * 0.7;
            let amp = base_amp + energy_boost * mid_y * 0.5;
            let freq = layer.freq + self.energy * 0.3;

            for x in 0..w {
                let norm_x = x as f64 / w as f64;

                // Hanning-like taper at edges
                let taper = (norm_x * std::f64::consts::PI).sin();
                let taper = taper * taper;

                // Composite wave: primary + secondary harmonic
                let primary = (norm_x * std::f64::consts::TAU * freq
                    + self.t * layer.speed * 2.0
                    + layer.phase)
                    .sin();
                let secondary = (norm_x * std::f64::consts::TAU * freq * 2.0
                    + self.t * layer.speed * 3.0
                    + layer.phase)
                    .sin()
                    * 0.3;

                let y_offset = (primary + secondary) * amp * taper;
                let y_pos = mid_y - y_offset;

                // Draw the wave point
                let cell_y = y_pos.floor() as i32;
                let frac = y_pos - cell_y as f64;

                let row = cell_y.clamp(0, h as i32 - 1) as u16;
                let col = x as u16;

                if col < area.width && row < area.height {
                    let cell = &mut buf[(area.x + col, area.y + row)];
                    // Use half-block for sub-cell precision
                    if frac < 0.5 {
                        cell.set_symbol(BLOCK_UPPER);
                    } else {
                        cell.set_symbol(BLOCK_LOWER);
                    }
                    cell.set_fg(layer.color);
                }

                // Draw a second point for thickness when energy is high
                if self.energy > 0.2 {
                    let row2 = (cell_y + 1).clamp(0, h as i32 - 1) as u16;
                    if col < area.width && row2 < area.height && row2 != row {
                        let cell2 = &mut buf[(area.x + col, area.y + row2)];
                        if cell2.symbol() == " " {
                            cell2.set_symbol(BLOCK_FULL);
                            // Dimmer for the trail
                            cell2.set_fg(dim_color(layer.color, 0.35));
                        }
                    }
                }
            }
        }
    }
}

fn dim_color(c: Color, factor: f64) -> Color {
    match c {
        Color::Rgb(r, g, b) => Color::Rgb(
            (r as f64 * factor) as u8,
            (g as f64 * factor) as u8,
            (b as f64 * factor) as u8,
        ),
        other => other,
    }
}

/// A subtle idle animation — single dim wave, minimal motion.
pub struct IdleWave {
    pub t: f64,
}

impl Widget for &IdleWave {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let w = area.width as usize;
        let h = area.height as usize;
        let mid_y = h as f64 / 2.0;

        let breath = 0.3 + 0.15 * (self.t * 0.4).sin();
        let color = Color::Rgb(60, 60, 80);

        for x in 0..w {
            let norm_x = x as f64 / w as f64;
            let taper = (norm_x * std::f64::consts::PI).sin();
            let taper = taper * taper;

            let y_offset = (norm_x * std::f64::consts::TAU * 1.5 + self.t * 0.6).sin()
                * breath
                * mid_y
                * 0.4
                * taper;
            let y_pos = mid_y - y_offset;
            let cell_y = y_pos.round().clamp(0.0, (h - 1) as f64) as u16;
            let col = x as u16;

            if col < area.width && cell_y < area.height {
                let cell = &mut buf[(area.x + col, area.y + cell_y)];
                cell.set_symbol("─");
                cell.set_fg(color);
            }
        }
    }
}
