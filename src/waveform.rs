use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::widgets::Widget;

use crate::constants::colors::*;
use crate::constants::*;

// ─── Braille rendering ───────────────────────────────────
//
// Each terminal cell maps to a 2×4 braille dot grid (U+2800–U+28FF).
// This gives us 2× horizontal and 4× vertical sub-pixel resolution,
// making curves look dramatically smoother than block characters.
//
//   Bit layout per cell:
//     col0  col1
//     ─────────
//     0     3      row 0
//     1     4      row 1
//     2     5      row 2
//     6     7      row 3

const BRAILLE_BASE: u32 = 0x2800;
const DOT_BITS: [[u8; 2]; 4] = [
    [0, 3], // row 0: bits 0, 3
    [1, 4], // row 1: bits 1, 4
    [2, 5], // row 2: bits 2, 5
    [6, 7], // row 3: bits 6, 7
];

/// A virtual high-res canvas that renders to braille characters.
/// Resolution: (width * 2) × (height * 4) sub-pixels.
struct BrailleCanvas {
    width: usize,       // terminal columns
    height: usize,      // terminal rows
    brightness: Vec<f64>, // flat array: [py * px_width + px], values 0.0–1.0
    color_r: Vec<f64>,
    color_g: Vec<f64>,
    color_b: Vec<f64>,
    px_w: usize,
    px_h: usize,
}

impl BrailleCanvas {
    fn new(width: usize, height: usize) -> Self {
        let px_w = width * 2;
        let px_h = height * 4;
        let n = px_w * px_h;
        Self {
            width,
            height,
            brightness: vec![0.0; n],
            color_r: vec![0.0; n],
            color_g: vec![0.0; n],
            color_b: vec![0.0; n],
            px_w,
            px_h,
        }
    }

    /// Plot a curve point — a vertical soft line at sub-pixel x.
    /// This is more efficient than plot() for drawing continuous curves.
    #[inline]
    fn plot_curve(&mut self, px: f64, py: f64, color: Color, intensity: f64, thickness: f64) {
        let (cr, cg, cb) = match color {
            Color::Rgb(r, g, b) => (r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0),
            _ => (1.0, 1.0, 1.0),
        };

        let sx = px.round() as i32;
        if sx < 0 || sx >= self.px_w as i32 {
            return;
        }

        let spread = (thickness * 2.5).ceil() as i32;
        let py_i = py.round() as i32;

        for dy in -spread..=spread {
            let sy = py_i + dy;
            if sy < 0 || sy >= self.px_h as i32 {
                continue;
            }
            let dist = (sy as f64 - py).abs();
            let falloff = (-dist * dist / (thickness * thickness * 0.5)).exp() * intensity;
            if falloff < 0.01 {
                continue;
            }
            let idx = sy as usize * self.px_w + sx as usize;
            self.brightness[idx] = (self.brightness[idx] + falloff).min(1.0);
            self.color_r[idx] += cr * falloff;
            self.color_g[idx] += cg * falloff;
            self.color_b[idx] += cb * falloff;
        }
    }

    /// Render the canvas into the terminal buffer.
    fn render_to(&self, area: Rect, buf: &mut Buffer, threshold: f64) {
        for cy in 0..self.height.min(area.height as usize) {
            for cx in 0..self.width.min(area.width as usize) {
                let mut dots: u8 = 0;
                let mut total_r = 0.0f64;
                let mut total_g = 0.0f64;
                let mut total_b = 0.0f64;
                let mut total_bright = 0.0f64;
                let mut any_lit = false;

                for row in 0..4 {
                    for col in 0..2 {
                        let px = cx * 2 + col;
                        let py = cy * 4 + row;
                        let idx = py * self.px_w + px;
                        if self.brightness[idx] >= threshold {
                            dots |= 1 << DOT_BITS[row][col];
                            total_r += self.color_r[idx];
                            total_g += self.color_g[idx];
                            total_b += self.color_b[idx];
                            total_bright += self.brightness[idx];
                            any_lit = true;
                        }
                    }
                }

                if !any_lit {
                    continue;
                }

                let ch = char::from_u32(BRAILLE_BASE + dots as u32).unwrap_or(' ');
                let cell = &mut buf[(area.x + cx as u16, area.y + cy as u16)];
                cell.set_char(ch);

                // Average color of lit pixels, scaled by brightness
                let scale = 255.0 / total_bright.max(0.001);
                let r = (total_r * scale).min(255.0) as u8;
                let g = (total_g * scale).min(255.0) as u8;
                let b = (total_b * scale).min(255.0) as u8;
                cell.set_fg(Color::Rgb(r, g, b));
            }
        }
    }
}

// ─── Wave math ───────────────────────────────────────────

struct WaveParams {
    freq: f64,
    phase: f64,
    speed: f64,
    amplitude: f64,
}

const WAVE_LAYERS: [WaveParams; 3] = [
    WaveParams { freq: 2.0, phase: 0.0, speed: 1.0, amplitude: 1.0 },
    WaveParams { freq: 2.8, phase: 1.2, speed: 0.75, amplitude: 0.55 },
    WaveParams { freq: 1.6, phase: 2.8, speed: 1.3, amplitude: 0.35 },
];

fn wave_y(norm_x: f64, t: f64, layer: &WaveParams, energy: f64) -> f64 {
    let freq = layer.freq + energy * 0.2;
    let primary = (norm_x * std::f64::consts::TAU * freq
        + t * layer.speed * 2.0
        + layer.phase)
        .sin();
    let harmonic = (norm_x * std::f64::consts::TAU * freq * 2.3
        + t * layer.speed * 2.8
        + layer.phase * 1.5)
        .sin() * 0.25;
    // Hanning taper
    let taper = (norm_x * std::f64::consts::PI).sin();
    let taper = taper * taper;

    (primary + harmonic) * taper * layer.amplitude
}

// ─── Active Waveform (Recording) ─────────────────────────

pub struct Waveform {
    pub t: f64,
    pub energy: f64,
    pub tick: u64,
}

impl Widget for &Waveform {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width < 4 || area.height < 2 {
            return;
        }

        let mut canvas = BrailleCanvas::new(area.width as usize, area.height as usize);
        let mid_y = canvas.px_h as f64 / 2.0;

        let breath = WAVEFORM_BREATH_MIN + WAVEFORM_BREATH_RANGE * (self.t * 0.5).sin();
        let energy_boost = self.energy * WAVEFORM_ENERGY_BOOST;
        let base_amp = breath * mid_y * 0.65;
        let amp = base_amp + energy_boost * mid_y * 0.4;

        // Layer colors: primary is blue→cyan, secondary is pink (dimmer), tertiary is green (dimmest)
        let colors: [(Color, Color, f64); 3] = [
            (BLUE, Color::Rgb(40, 180, 255), 1.0),                  // primary: full brightness
            (PINK, Color::Rgb(255, 100, 160), 0.45),                // secondary: subtle
            (Color::Rgb(40, 200, 170), Color::Rgb(80, 255, 210), 0.3), // tertiary: faint
        ];

        // Draw each layer
        for (li, layer) in WAVE_LAYERS.iter().enumerate() {
            let (color_l, color_r, layer_intensity) = &colors[li];
            let thickness = if li == 0 {
                1.8 + self.energy * 2.5 // primary wave gets thicker with energy
            } else {
                1.0 + self.energy * 0.8
            };

            for px in 0..canvas.px_w {
                let norm_x = px as f64 / canvas.px_w as f64;
                let color = lerp_color(*color_l, *color_r, norm_x);
                let y = wave_y(norm_x, self.t, layer, self.energy);
                let py = mid_y - y * amp;

                canvas.plot_curve(px as f64, py, color, *layer_intensity, thickness);
            }
        }

        // Glow: re-render primary layer with wider spread at low brightness
        let primary = &WAVE_LAYERS[0];
        let glow_thickness = 4.0 + self.energy * 6.0;
        for px in (0..canvas.px_w).step_by(2) {
            let norm_x = px as f64 / canvas.px_w as f64;
            let color = lerp_color(BLUE, Color::Rgb(40, 180, 255), norm_x);
            let y = wave_y(norm_x, self.t, primary, self.energy);
            let py = mid_y - y * amp;
            canvas.plot_curve(px as f64, py, dim_color(color, 0.25), 0.3, glow_thickness);
        }

        canvas.render_to(area, buf, 0.08);
    }
}

// ─── Transcribing Wave ───────────────────────────────────

pub struct TranscribingWave {
    pub t: f64,
    pub tick: u64,
    pub pulse_boost: f64,
}

impl Widget for &TranscribingWave {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width < 4 || area.height < 2 {
            return;
        }

        let mut canvas = BrailleCanvas::new(area.width as usize, area.height as usize);
        let mid_y = canvas.px_h as f64 / 2.0;

        let pulse = ((self.t * 2.5).sin() * 0.5 + 0.5) * 0.35;
        let energy = pulse + self.pulse_boost * 0.3;

        let breath = WAVEFORM_BREATH_MIN + WAVEFORM_BREATH_RANGE * (self.t * 0.5).sin();
        let amp = breath * mid_y * 0.45 * (0.6 + energy * 0.5);

        // Scan-line position
        let scan_pos = (self.tick % SCANLINE_PERIOD) as f64 / SCANLINE_PERIOD as f64;

        // Single flowing wave with scan-line brightening
        for px in 0..canvas.px_w {
            let norm_x = px as f64 / canvas.px_w as f64;

            // Scan-line proximity → brightness boost
            let scan_dist = (norm_x - scan_pos).abs();
            let scan_glow = (1.0 - (scan_dist * 6.0).min(1.0)).max(0.0);
            let brightness = 0.4 + scan_glow * 0.6;

            let color = lerp_color(
                Color::Rgb(8, 90, 180),
                Color::Rgb(60, 200, 255),
                scan_glow,
            );

            for (li, layer) in WAVE_LAYERS.iter().enumerate() {
                let layer_dim = [1.0, 0.3, 0.15][li];
                let y = wave_y(norm_x, self.t, layer, energy * 0.3);
                let py = mid_y - y * amp * layer.amplitude;
                let thickness = if li == 0 { 1.5 } else { 0.8 };
                canvas.plot_curve(px as f64, py, color, brightness * layer_dim, thickness);
            }
        }

        // Wider glow around scan-line
        let scan_px = (scan_pos * canvas.px_w as f64) as i32;
        for py in 0..canvas.px_h {
            let norm_y = py as f64 / canvas.px_h as f64;
            let edge_fade = (norm_y * std::f64::consts::PI).sin(); // dim at top/bottom
            for dx in -6i32..=6 {
                let px = scan_px + dx;
                if px >= 0 && px < canvas.px_w as i32 {
                    let dist = dx.unsigned_abs() as f64;
                    let glow = (-dist * dist / 12.0).exp() * 0.15 * edge_fade;
                    if glow > 0.01 {
                        let idx = py * canvas.px_w + px as usize;
                        canvas.brightness[idx] = (canvas.brightness[idx] + glow).min(1.0);
                        canvas.color_r[idx] += 0.15 * glow;
                        canvas.color_g[idx] += 0.55 * glow;
                        canvas.color_b[idx] += 1.0 * glow;
                    }
                }
            }
        }

        canvas.render_to(area, buf, 0.06);
    }
}

// ─── Idle Wave ───────────────────────────────────────────

pub struct IdleWave {
    pub t: f64,
    pub tick: u64,
}

impl Widget for &IdleWave {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width < 4 || area.height < 2 {
            return;
        }

        let mut canvas = BrailleCanvas::new(area.width as usize, area.height as usize);
        let mid_y = canvas.px_h as f64 / 2.0;

        let breath = WAVEFORM_IDLE_BREATH_MIN + WAVEFORM_IDLE_BREATH_RANGE * (self.t * 0.4).sin();
        let amp = breath * mid_y * 0.35;

        // Single gentle wave
        let color = Color::Rgb(50, 55, 75);
        let color_center = Color::Rgb(70, 80, 110);

        for px in 0..canvas.px_w {
            let norm_x = px as f64 / canvas.px_w as f64;
            let taper = (norm_x * std::f64::consts::PI).sin();
            let taper = taper * taper;

            // Gradient: brighter at center
            let center_dist = (norm_x - 0.5).abs() * 2.0;
            let c = lerp_color(color_center, color, center_dist);

            let y = (norm_x * std::f64::consts::TAU * 1.5 + self.t * 0.6).sin();
            let py = mid_y - y * amp * taper;

            canvas.plot_curve(px as f64, py, c, 0.7, 1.2);
        }

        // Soft glow around center of wave
        for px in (0..canvas.px_w).step_by(3) {
            let norm_x = px as f64 / canvas.px_w as f64;
            let taper = (norm_x * std::f64::consts::PI).sin();
            let y = (norm_x * std::f64::consts::TAU * 1.5 + self.t * 0.6).sin();
            let py = mid_y - y * amp * taper * taper;
            canvas.plot_curve(px as f64, py, Color::Rgb(40, 45, 65), 0.15, 3.5);
        }

        canvas.render_to(area, buf, 0.10);
    }
}
