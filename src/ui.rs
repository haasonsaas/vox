use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;

use crate::app::{AppState, Mode};
use crate::constants::colors::*;
use crate::constants::*;
use crate::waveform::{IdleWave, TranscribingWave, Waveform};

const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

pub fn draw(f: &mut Frame, state: &AppState) {
    let size = f.area();

    // Fill background
    let bg_block = Block::default().style(Style::default().bg(SURFACE));
    f.render_widget(bg_block, size);

    // Result mode: waveform area becomes scrollable result text
    let show_result_text = matches!(&state.mode, Mode::Result { .. });

    // History gets 0 rows if empty, otherwise a compact section
    let history_rows = if state.history.is_empty() {
        0
    } else {
        (state.history.len() as u16 + 1).min(8) // cap at 8 rows
    };

    // Layout: thin header, big waveform, thin status, compact history
    let chunks = Layout::vertical([
        Constraint::Length(1),          // header: just one line
        Constraint::Length(1),          // spacer
        Constraint::Min(6),             // waveform: fills available space
        Constraint::Length(1),          // spacer
        Constraint::Length(1),          // status bar
        Constraint::Length(history_rows),
    ])
    .split(size);

    draw_header(f, chunks[0], state);

    if show_result_text {
        draw_result_text(f, chunks[2], state);
    } else {
        draw_waveform(f, chunks[2], state);
    }

    draw_status(f, chunks[4], state);

    if history_rows > 0 {
        draw_history(f, chunks[5], state);
    }

    if state.show_device_picker {
        draw_device_picker(f, size, state);
    }
}

fn draw_header(f: &mut Frame, area: Rect, state: &AppState) {
    // Gentle pulsing in idle
    let pulse = |offset: f64| -> f64 {
        if matches!(state.mode, Mode::Idle) {
            0.6 + 0.4 * ((state.tick as f64 * 0.05 + offset).sin())
        } else {
            1.0
        }
    };

    // Mode accent dot — reflects current state with subtle animation
    let (dot_char, dot_color) = match &state.mode {
        Mode::Idle => {
            let dim = 0.3 + 0.15 * (state.tick as f64 * 0.04).sin();
            ("◦", dim_color(Color::Rgb(80, 85, 100), dim))
        }
        Mode::Recording { .. } => {
            let pulse_t = (state.tick as f64 * 0.15).sin() * 0.5 + 0.5;
            ("●", lerp_color(Color::Rgb(180, 40, 35), RED_PULSE, pulse_t))
        }
        Mode::Transcribing { .. } => {
            let pulse_t = (state.tick as f64 * 0.1).sin() * 0.5 + 0.5;
            ("◉", lerp_color(Color::Rgb(8, 90, 180), BLUE, pulse_t))
        }
        Mode::Result { .. } => ("◉", GREEN),
        Mode::Error { .. } => ("◉", RED_PULSE),
    };

    let title = vec![
        Span::styled(
            format!(" {dot_char} "),
            Style::default().fg(dot_color),
        ),
        Span::styled(
            "v",
            Style::default()
                .fg(dim_color(BLUE, pulse(0.0)))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "o",
            Style::default()
                .fg(dim_color(PINK, pulse(1.2)))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "x",
            Style::default()
                .fg(dim_color(GREEN, pulse(2.4)))
                .add_modifier(Modifier::BOLD),
        ),
    ];

    // Contextual hints — minimal, lowercase
    let hint = match state.mode {
        Mode::Idle => {
            if state.history_selected.is_some() {
                "enter copy  x del  X clear  esc deselect"
            } else {
                "space record  d device  q quit"
            }
        }
        Mode::Recording { .. } => "space stop  esc cancel",
        Mode::Transcribing { .. } => "",
        Mode::Result { copied: true, .. } => "space new  r redo  s save  w wav",
        Mode::Result { .. } => "space new  r redo  c copy  s save  w wav",
        Mode::Error { .. } => "space retry  q quit",
    };

    let hint_span = Span::styled(
        format!("{hint} "),
        Style::default().fg(Color::Rgb(65, 65, 75)),
    );

    let header_cols =
        Layout::horizontal([Constraint::Min(0), Constraint::Length(hint.len() as u16 + 1)])
            .split(area);

    f.render_widget(
        Paragraph::new(Line::from(title)).style(Style::default().bg(SURFACE)),
        header_cols[0],
    );
    f.render_widget(
        Paragraph::new(hint_span)
            .alignment(Alignment::Right)
            .style(Style::default().bg(SURFACE)),
        header_cols[1],
    );
}

fn draw_waveform(f: &mut Frame, area: Rect, state: &AppState) {
    // Slight horizontal padding
    let inner = if area.width > 6 {
        Rect {
            x: area.x + 1,
            y: area.y,
            width: area.width - 2,
            height: area.height,
        }
    } else {
        area
    };

    let age = state.transition_age();

    match &state.mode {
        Mode::Recording { energy, .. } => {
            let wf = Waveform {
                t: state.tick as f64 * WAVEFORM_TIME_SCALE,
                energy: *energy,
                tick: state.tick,
            };
            f.render_widget(&wf, inner);
        }
        Mode::Transcribing { .. } => {
            let pulse_boost = if age <= TRANSITION_PULSE_TICKS {
                1.0 - (age as f64 / TRANSITION_PULSE_TICKS as f64)
            } else {
                0.0
            };
            let morph_progress = if age < WAVEFORM_MORPH_TICKS {
                age as f64 / WAVEFORM_MORPH_TICKS as f64
            } else {
                1.0
            };
            // Smooth easing (ease-in-out)
            let morph_progress = morph_progress * morph_progress * (3.0 - 2.0 * morph_progress);
            let tw = TranscribingWave {
                t: state.tick as f64 * WAVEFORM_TRANSCRIBING_TIME_SCALE,
                tick: state.tick,
                pulse_boost,
                morph_from_energy: state.transition_energy,
                morph_progress,
            };
            f.render_widget(&tw, inner);
        }
        _ => {
            let idle = IdleWave {
                t: state.tick as f64 * WAVEFORM_IDLE_TIME_SCALE,
                tick: state.tick,
            };
            f.render_widget(&idle, inner);
        }
    }
}

fn draw_result_text(f: &mut Frame, area: Rect, state: &AppState) {
    if let Mode::Result { ref text, .. } = state.mode {
        let age = state.transition_age();

        // Typewriter reveal: characters appear progressively
        let max_chars = if age < (text.chars().count() / TYPEWRITER_CHARS_PER_TICK + TRANSITION_FADE_IN_TICKS as usize) as u64 {
            (age as usize * TYPEWRITER_CHARS_PER_TICK).min(text.chars().count())
        } else {
            text.chars().count()
        };
        let revealed: String = text.chars().take(max_chars).collect();
        let fully_revealed = max_chars >= text.chars().count();

        let alpha = if age < TRANSITION_FADE_IN_TICKS {
            age as f64 / TRANSITION_FADE_IN_TICKS as f64
        } else {
            1.0
        };

        // Center text vertically if it fits in the area
        let inner = Rect {
            x: area.x + 3,
            y: area.y + 1,
            width: area.width.saturating_sub(6),
            height: area.height.saturating_sub(1),
        };

        // Accent line on the left — grows with reveal, subtle gradient
        if inner.height > 0 && inner.x > area.x + 1 {
            let accent_x = area.x + 1;
            let accent_height = if fully_revealed {
                inner.height.min(3)
            } else {
                inner.height.min(3).min((age as u16 / 2).max(1))
            };
            for row in 0..accent_height {
                if accent_x < area.x + area.width && (inner.y + row) < area.y + area.height {
                    let row_t = row as f64 / accent_height.max(1) as f64;
                    let accent_color = lerp_color(GREEN, Color::Rgb(30, 140, 120), row_t);
                    let cell =
                        &mut f.buffer_mut()[(accent_x, inner.y + row)];
                    cell.set_char('│');
                    cell.set_fg(dim_color(accent_color, alpha * 0.6));
                }
            }
        }

        // Build styled text: revealed chars in full color, cursor blink at end
        let text_color = lerp_color(SURFACE, TEXT, alpha);
        let mut spans = vec![Span::styled(&revealed, Style::default().fg(text_color))];

        // Blinking cursor at the typing edge (while still revealing)
        if !fully_revealed && (age / 4) % 2 == 0 {
            spans.push(Span::styled("▎", Style::default().fg(dim_color(GREEN, 0.6))));
        }

        let para = Paragraph::new(Line::from(spans))
            .style(Style::default().bg(SURFACE))
            .wrap(Wrap { trim: false })
            .scroll((state.result_scroll, 0));
        f.render_widget(para, inner);
    }
}

fn draw_status(f: &mut Frame, area: Rect, state: &AppState) {
    // Flash message overrides normal status
    if let Some(flash) = state.active_flash() {
        let spans = vec![
            Span::styled(" ✓ ", Style::default().fg(GREEN)),
            Span::styled(flash, Style::default().fg(Color::Rgb(80, 80, 95))),
        ];
        f.render_widget(
            Paragraph::new(Line::from(spans)).style(Style::default().bg(SURFACE)),
            area,
        );
        return;
    }

    let spans = match &state.mode {
        Mode::Idle => {
            let mut spans = vec![
                Span::styled(" ○ ", Style::default().fg(Color::Rgb(55, 55, 65))),
                Span::styled("ready", Style::default().fg(Color::Rgb(80, 80, 95))),
            ];
            if state.auto_copy {
                spans.push(Span::styled(
                    "  ⎘ auto-copy",
                    Style::default().fg(Color::Rgb(45, 45, 55)),
                ));
            }
            if state.silence_timeout_ticks > 0 {
                spans.push(Span::styled(
                    "  ◇ vad",
                    Style::default().fg(Color::Rgb(45, 45, 55)),
                ));
            }
            spans
        }
        Mode::Recording {
            duration_secs,
            energy,
            ..
        } => {
            let secs = *duration_secs;

            // Pulsing red dot
            let pulse_t = (state.tick as f64 * 0.15).sin() * 0.5 + 0.5;
            let dot_color = lerp_color(Color::Rgb(140, 30, 25), RED_PULSE, pulse_t);

            // Compact level meter — 12 chars, gradient
            let bar_len = ((*energy * 12.0) as usize).min(12);
            let mut spans = vec![
                Span::styled(" ● ", Style::default().fg(dot_color)),
                Span::styled(
                    format!("{secs:5.1}s  "),
                    Style::default().fg(TEXT_DIM),
                ),
            ];
            for i in 0..12 {
                let t = i as f64 / 11.0;
                let c = if t < 0.5 {
                    lerp_color(Color::Rgb(30, 160, 120), Color::Rgb(200, 180, 20), t * 2.0)
                } else {
                    lerp_color(Color::Rgb(200, 180, 20), Color::Rgb(230, 60, 70), (t - 0.5) * 2.0)
                };
                if i < bar_len {
                    spans.push(Span::styled("▮", Style::default().fg(c)));
                } else {
                    spans.push(Span::styled("▯", Style::default().fg(Color::Rgb(35, 35, 42))));
                }
            }
            spans
        }
        Mode::Transcribing {
            duration_secs,
            ref partial_text,
        } => {
            let spinner = SPINNER[(state.tick / 3) as usize % SPINNER.len()];
            let mut spans = vec![
                Span::styled(format!(" {spinner} "), Style::default().fg(BLUE)),
                Span::styled(
                    "transcribing",
                    Style::default().fg(Color::Rgb(60, 140, 230)),
                ),
                Span::styled(
                    format!("  {duration_secs:.1}s"),
                    Style::default().fg(Color::Rgb(70, 70, 85)),
                ),
            ];
            if !partial_text.is_empty() {
                let preview: String = partial_text.chars().rev().take(40).collect::<Vec<_>>().into_iter().rev().collect();
                spans.push(Span::styled(
                    format!("  {preview}"),
                    Style::default().fg(Color::Rgb(90, 90, 105)),
                ));
            }
            spans
        }
        Mode::Result { ref text, copied: true } => {
            let words = text.split_whitespace().count();
            vec![
                Span::styled(" ✓ ", Style::default().fg(GREEN)),
                Span::styled("copied to clipboard", Style::default().fg(Color::Rgb(80, 80, 95))),
                Span::styled(
                    format!("  {words} words"),
                    Style::default().fg(Color::Rgb(55, 55, 65)),
                ),
            ]
        }
        Mode::Result { ref text, .. } => {
            let words = text.split_whitespace().count();
            vec![
                Span::styled(" ✓ ", Style::default().fg(GREEN)),
                Span::styled("done", Style::default().fg(Color::Rgb(80, 80, 95))),
                Span::styled(
                    format!("  {words} words"),
                    Style::default().fg(Color::Rgb(55, 55, 65)),
                ),
            ]
        }
        Mode::Error { ref message } => {
            let truncated: String = message.chars().take(area.width as usize - 4).collect();
            vec![
                Span::styled(" ✗ ", Style::default().fg(RED_PULSE)),
                Span::styled(truncated, Style::default().fg(Color::Rgb(200, 90, 80))),
            ]
        }
    };

    f.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::default().bg(SURFACE)),
        area,
    );
}

fn draw_history(f: &mut Frame, area: Rect, state: &AppState) {
    if state.history.is_empty() || area.height == 0 {
        return;
    }

    let mut lines: Vec<Line> = Vec::new();

    // Gradient-fading divider
    let divider_width = area.width.min(30) as usize;
    let divider_spans: Vec<Span> = std::iter::once(Span::raw(" "))
        .chain((0..divider_width).map(|i| {
            let t = i as f64 / divider_width as f64;
            let fade = 1.0 - t; // fades from left to right
            let c = dim_color(Color::Rgb(50, 55, 65), fade * 0.7 + 0.15);
            Span::styled("─", Style::default().fg(c))
        }))
        .collect();
    lines.push(Line::from(divider_spans));

    let visible_count = (area.height as usize).saturating_sub(1); // -1 for divider
    let entries: Vec<_> = state
        .history
        .iter()
        .enumerate()
        .rev()
        .take(visible_count)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();

    for (i, entry) in &entries {
        let is_selected = state.history_selected == Some(*i);

        // Truncate to fit
        let max_chars = (area.width as usize).saturating_sub(4);
        let truncated: String = entry.chars().take(max_chars).collect();
        let ellipsis = if entry.chars().count() > max_chars {
            "…"
        } else {
            ""
        };

        if is_selected {
            lines.push(Line::from(vec![
                Span::styled(" ▸ ", Style::default().fg(BLUE)),
                Span::styled(
                    format!("{truncated}{ellipsis}"),
                    Style::default().fg(TEXT).bg(Color::Rgb(30, 32, 40)),
                ),
            ]));
        } else {
            lines.push(Line::from(vec![
                Span::styled("   ", Style::default()),
                Span::styled(
                    format!("{truncated}{ellipsis}"),
                    Style::default().fg(Color::Rgb(80, 80, 95)),
                ),
            ]));
        }
    }

    let para = Paragraph::new(lines).style(Style::default().bg(SURFACE));
    f.render_widget(para, area);
}

fn draw_device_picker(f: &mut Frame, area: Rect, state: &AppState) {
    let popup_width = 48u16.min(area.width.saturating_sub(4));
    let popup_height =
        (state.input_devices.len() as u16 + 4).min(area.height.saturating_sub(4));
    let popup_x = (area.width.saturating_sub(popup_width)) / 2 + area.x;
    let popup_y = (area.height.saturating_sub(popup_height)) / 2 + area.y;
    let popup_area = Rect::new(popup_x, popup_y, popup_width, popup_height);

    f.render_widget(Clear, popup_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Rgb(50, 55, 70)))
        .title(Span::styled(
            " device ",
            Style::default()
                .fg(BLUE)
                .add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(Color::Rgb(20, 20, 24)));

    let inner = block.inner(popup_area);
    f.render_widget(block, popup_area);

    let mut lines: Vec<Line> = Vec::new();
    for (i, (name, is_default)) in state.input_devices.iter().enumerate() {
        let is_selected = i == state.selected_device;
        let indicator = if is_selected { " ▸ " } else { "   " };
        let bg = if is_selected {
            Color::Rgb(30, 32, 40)
        } else {
            Color::Rgb(20, 20, 24)
        };
        let fg = if is_selected { TEXT } else { TEXT_DIM };
        let default_tag = if *is_default { " *" } else { "" };

        let max_name = (popup_width as usize).saturating_sub(8);
        let truncated: String = name.chars().take(max_name).collect();
        lines.push(Line::from(vec![
            Span::styled(indicator, Style::default().fg(BLUE).bg(bg)),
            Span::styled(truncated, Style::default().fg(fg).bg(bg)),
            Span::styled(
                default_tag,
                Style::default()
                    .fg(Color::Rgb(70, 70, 85))
                    .bg(bg),
            ),
        ]));
    }

    if state.input_devices.is_empty() {
        lines.push(Line::from(Span::styled(
            "   no devices found",
            Style::default().fg(Color::Rgb(200, 90, 80)),
        )));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "   ↑↓ enter esc",
        Style::default().fg(Color::Rgb(55, 55, 65)),
    )));

    let para = Paragraph::new(lines).style(Style::default().bg(Color::Rgb(20, 20, 24)));
    f.render_widget(para, inner);
}
