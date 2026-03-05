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

        // Sparkle particles at wave peaks when energy is high
        if self.energy > 0.15 {
            let spark_intensity = ((self.energy - 0.15) / 0.85).min(1.0);
            // Use tick as seed for deterministic but changing sparkle positions
            let seed = self.tick;
            for i in 0..((spark_intensity * 18.0) as u64) {
                // Simple hash for pseudo-random placement
                let h = seed.wrapping_mul(2654435761).wrapping_add(i.wrapping_mul(40503));
                let norm_x = (h % 1000) as f64 / 1000.0;
                let px = norm_x * canvas.px_w as f64;

                // Place sparkle on the primary wave curve + small offset
                let y = wave_y(norm_x, self.t, primary, self.energy);
                let offset = ((h / 1000 % 200) as f64 - 100.0) / 100.0 * 3.0;
                let py = mid_y - y * amp + offset;

                // Sparkle lifecycle: bright flash then fade
                let life = ((h / 200000 % 8) as f64) / 8.0;
                let brightness = (1.0 - life) * spark_intensity;

                if brightness > 0.05 {
                    let spark_color = lerp_color(
                        Color::Rgb(180, 220, 255),
                        Color::Rgb(255, 255, 255),
                        brightness,
                    );
                    canvas.plot_curve(px, py, spark_color, brightness * 0.8, 0.6);
                }
            }
        }

        canvas.render_to(area, buf, 0.08);
    }
}

// ─── Transcribing Wave ───────────────────────────────────

pub struct TranscribingWave {
    pub t: f64,
    pub tick: u64,
    pub pulse_boost: f64,
    pub morph_from_energy: f64,
    pub morph_progress: f64, // 0.0 = recording style, 1.0 = transcribing style
}

impl Widget for &TranscribingWave {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width < 4 || area.height < 2 {
            return;
        }

        let mut canvas = BrailleCanvas::new(area.width as usize, area.height as usize);
        let mid_y = canvas.px_h as f64 / 2.0;

        let mp = self.morph_progress; // 0=recording, 1=transcribing

        let pulse = ((self.t * 2.5).sin() * 0.5 + 0.5) * 0.35;
        let trans_energy = pulse + self.pulse_boost * 0.3;
        // Blend energy: recording energy fades into transcribing pulse
        let energy = self.morph_from_energy * (1.0 - mp) + trans_energy * mp;

        let breath = WAVEFORM_BREATH_MIN + WAVEFORM_BREATH_RANGE * (self.t * 0.5).sin();
        let trans_amp = breath * mid_y * 0.45 * (0.6 + trans_energy * 0.5);
        let rec_amp = {
            let base = breath * mid_y * 0.65;
            base + self.morph_from_energy * WAVEFORM_ENERGY_BOOST * mid_y * 0.4
        };
        let amp = rec_amp * (1.0 - mp) + trans_amp * mp;

        // Scan-line fades in with morph
        let scan_pos = (self.tick % SCANLINE_PERIOD) as f64 / SCANLINE_PERIOD as f64;

        // Recording-style colors (blue→cyan multi-layer)
        let rec_colors: [(Color, Color, f64); 3] = [
            (BLUE, Color::Rgb(40, 180, 255), 1.0),
            (PINK, Color::Rgb(255, 100, 160), 0.45),
            (Color::Rgb(40, 200, 170), Color::Rgb(80, 255, 210), 0.3),
        ];

        for px in 0..canvas.px_w {
            let norm_x = px as f64 / canvas.px_w as f64;

            // Scan-line proximity (fades in with morph)
            let scan_dist = (norm_x - scan_pos).abs();
            let scan_glow = (1.0 - (scan_dist * 6.0).min(1.0)).max(0.0) * mp;

            let trans_color = lerp_color(
                Color::Rgb(8, 90, 180),
                Color::Rgb(60, 200, 255),
                scan_glow,
            );
            let trans_brightness = 0.4 + scan_glow * 0.6;

            for (li, layer) in WAVE_LAYERS.iter().enumerate() {
                let y = wave_y(norm_x, self.t, layer, energy * 0.3);
                let py = mid_y - y * amp * layer.amplitude;

                // Blend recording and transcribing styles
                let (rec_l, rec_r, rec_int) = &rec_colors[li];
                let rec_color = lerp_color(*rec_l, *rec_r, norm_x);
                let rec_thick = if li == 0 {
                    1.8 + self.morph_from_energy * 2.5
                } else {
                    1.0 + self.morph_from_energy * 0.8
                };

                let trans_layer_dim = [1.0, 0.3, 0.15][li];
                let trans_thick = if li == 0 { 1.5 } else { 0.8 };

                let color = lerp_color(rec_color, trans_color, mp);
                let intensity =
                    rec_int * (1.0 - mp) + trans_brightness * trans_layer_dim * mp;
                let thickness = rec_thick * (1.0 - mp) + trans_thick * mp;

                canvas.plot_curve(px as f64, py, color, intensity, thickness);
            }
        }

        // Wider glow around scan-line (fades in)
        if mp > 0.1 {
            let scan_px = (scan_pos * canvas.px_w as f64) as i32;
            for py in 0..canvas.px_h {
                let norm_y = py as f64 / canvas.px_h as f64;
                let edge_fade = (norm_y * std::f64::consts::PI).sin();
                for dx in -6i32..=6 {
                    let px = scan_px + dx;
                    if px >= 0 && px < canvas.px_w as i32 {
                        let dist = dx.unsigned_abs() as f64;
                        let glow = (-dist * dist / 12.0).exp() * 0.15 * edge_fade * mp;
                        if glow > 0.01 {
                            let idx = py * canvas.px_w + px as usize;
                            canvas.brightness[idx] =
                                (canvas.brightness[idx] + glow).min(1.0);
                            canvas.color_r[idx] += 0.15 * glow;
                            canvas.color_g[idx] += 0.55 * glow;
                            canvas.color_b[idx] += 1.0 * glow;
                        }
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

        // Slow color cycling — hue drifts over time
        let hue_t = (self.t * 0.08).sin() * 0.5 + 0.5; // 0..1 slowly
        let color_edge = lerp_color(Color::Rgb(50, 55, 75), Color::Rgb(60, 45, 80), hue_t);
        let color_center = lerp_color(Color::Rgb(70, 80, 110), Color::Rgb(90, 65, 130), hue_t);

        // Primary wave
        for px in 0..canvas.px_w {
            let norm_x = px as f64 / canvas.px_w as f64;
            let taper = (norm_x * std::f64::consts::PI).sin();
            let taper = taper * taper;

            let center_dist = (norm_x - 0.5).abs() * 2.0;
            let c = lerp_color(color_center, color_edge, center_dist);

            let y = (norm_x * std::f64::consts::TAU * 1.5 + self.t * 0.6).sin();
            let py = mid_y - y * amp * taper;

            canvas.plot_curve(px as f64, py, c, 0.7, 1.2);
        }

        // Secondary wave — counter-moving, fainter, warmer
        let amp2 = amp * 0.5;
        let color2 = lerp_color(Color::Rgb(55, 40, 60), Color::Rgb(40, 55, 65), hue_t);
        for px in 0..canvas.px_w {
            let norm_x = px as f64 / canvas.px_w as f64;
            let taper = (norm_x * std::f64::consts::PI).sin();
            let taper = taper * taper;

            let y = (norm_x * std::f64::consts::TAU * 2.2 - self.t * 0.4 + 1.8).sin();
            let py = mid_y - y * amp2 * taper;

            canvas.plot_curve(px as f64, py, color2, 0.35, 1.0);
        }

        // Soft glow around center of primary wave
        for px in (0..canvas.px_w).step_by(3) {
            let norm_x = px as f64 / canvas.px_w as f64;
            let taper = (norm_x * std::f64::consts::PI).sin();
            let y = (norm_x * std::f64::consts::TAU * 1.5 + self.t * 0.6).sin();
            let py = mid_y - y * amp * taper * taper;
            let glow_color = lerp_color(Color::Rgb(40, 45, 65), Color::Rgb(50, 35, 60), hue_t);
            canvas.plot_curve(px as f64, py, glow_color, 0.15, 3.5);
        }

        // Ambient floating particles — dust motes drifting slowly
        let num_particles = 12u64;
        for i in 0..num_particles {
            // Each particle has a unique slow orbit
            let seed = i.wrapping_mul(7919);
            let base_x = (seed % 1000) as f64 / 1000.0;
            let base_y = ((seed / 1000) % 1000) as f64 / 1000.0;
            let speed_x = 0.015 + (seed % 17) as f64 * 0.002;
            let speed_y = 0.008 + (seed % 13) as f64 * 0.001;
            let phase = (seed % 31) as f64 * 0.2;

            let px = ((base_x + self.t * speed_x + phase).sin() * 0.4 + 0.5) * canvas.px_w as f64;
            let py = ((base_y + self.t * speed_y + phase * 1.3).sin() * 0.35 + 0.5)
                * canvas.px_h as f64;

            // Twinkle: slow brightness oscillation
            let twinkle = ((self.t * 0.3 + phase * 2.0).sin() * 0.5 + 0.5) * 0.3 + 0.1;
            let particle_color = lerp_color(
                Color::Rgb(45, 50, 70),
                Color::Rgb(65, 55, 85),
                hue_t,
            );

            canvas.plot_curve(px, py, particle_color, twinkle, 0.5);
        }

        canvas.render_to(area, buf, 0.10);
    }
}
