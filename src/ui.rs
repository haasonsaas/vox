use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;

use crate::app::AppState;
use crate::waveform::{IdleWave, Waveform};

// Lark-inspired color palette
const BLUE: Color = Color::Rgb(10, 132, 255);
const PINK: Color = Color::Rgb(255, 55, 95);
const GREEN: Color = Color::Rgb(48, 220, 155);
const DIM: Color = Color::Rgb(100, 100, 110);
const SURFACE: Color = Color::Rgb(24, 24, 28);
const SURFACE_LIGHT: Color = Color::Rgb(35, 35, 42);
const TEXT: Color = Color::Rgb(220, 220, 230);
const TEXT_DIM: Color = Color::Rgb(130, 130, 145);
const RED_PULSE: Color = Color::Rgb(255, 59, 48);

const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

pub fn draw(f: &mut Frame, state: &AppState) {
    let size = f.area();

    // Fill background
    let bg_block = Block::default().style(Style::default().bg(SURFACE));
    f.render_widget(bg_block, size);

    // Layout: header, waveform, status, history
    let chunks = Layout::vertical([
        Constraint::Length(3),  // header
        Constraint::Length(7),  // waveform area
        Constraint::Length(3),  // status bar
        Constraint::Min(0),    // transcript history
    ])
    .split(size);

    draw_header(f, chunks[0], state);
    draw_waveform(f, chunks[1], state);
    draw_status(f, chunks[2], state);
    draw_history(f, chunks[3], state);
}

fn draw_header(f: &mut Frame, area: Rect, state: &AppState) {
    let title_spans = vec![
        Span::styled("v", Style::default().fg(BLUE).add_modifier(Modifier::BOLD)),
        Span::styled("o", Style::default().fg(PINK).add_modifier(Modifier::BOLD)),
        Span::styled("x", Style::default().fg(GREEN).add_modifier(Modifier::BOLD)),
    ];

    let auth_info = Span::styled(
        format!("  {}", state.auth_source),
        Style::default().fg(TEXT_DIM),
    );

    let mut spans = title_spans;
    spans.push(auth_info);

    let help = match state.mode {
        Mode::Idle => " SPACE record  Q quit ",
        Mode::Recording { .. } => " SPACE stop  Q quit ",
        Mode::Transcribing { .. } => " transcribing... ",
        Mode::Result { .. } => " SPACE again  C copy  Q quit ",
        Mode::Error { .. } => " SPACE retry  Q quit ",
    };

    let help_span = Span::styled(help, Style::default().fg(DIM));

    let header_block = Block::default()
        .borders(Borders::BOTTOM)
        .border_style(Style::default().fg(SURFACE_LIGHT))
        .style(Style::default().bg(SURFACE));

    let inner = header_block.inner(area);
    f.render_widget(header_block, area);

    // Split header: left = title, right = help
    let header_cols = Layout::horizontal([Constraint::Min(0), Constraint::Length(help.len() as u16)])
        .split(inner);

    let title_line = Line::from(spans);
    f.render_widget(
        Paragraph::new(title_line).style(Style::default().bg(SURFACE)),
        header_cols[0],
    );
    f.render_widget(
        Paragraph::new(help_span)
            .alignment(Alignment::Right)
            .style(Style::default().bg(SURFACE)),
        header_cols[1],
    );
}

fn draw_waveform(f: &mut Frame, area: Rect, state: &AppState) {
    // Pad the waveform area slightly
    let inner = if area.width > 4 && area.height > 2 {
        Rect {
            x: area.x + 2,
            y: area.y,
            width: area.width - 4,
            height: area.height,
        }
    } else {
        area
    };

    match &state.mode {
        Mode::Recording { energy, .. } => {
            let wf = Waveform {
                t: state.tick as f64 * 0.08,
                energy: *energy,
            };
            f.render_widget(&wf, inner);
        }
        Mode::Transcribing { .. } => {
            // Pulsing blue wave while transcribing
            let pulse = ((state.tick as f64 * 0.1).sin() * 0.5 + 0.5) * 0.4;
            let wf = Waveform {
                t: state.tick as f64 * 0.04,
                energy: pulse,
            };
            f.render_widget(&wf, inner);
        }
        _ => {
            let idle = IdleWave {
                t: state.tick as f64 * 0.05,
            };
            f.render_widget(&idle, inner);
        }
    }
}

fn draw_status(f: &mut Frame, area: Rect, state: &AppState) {
    let status_block = Block::default()
        .borders(Borders::TOP | Borders::BOTTOM)
        .border_style(Style::default().fg(SURFACE_LIGHT))
        .style(Style::default().bg(SURFACE));

    let inner = status_block.inner(area);
    f.render_widget(status_block, area);

    let line = match &state.mode {
        Mode::Idle => {
            Line::from(vec![
                Span::styled("● ", Style::default().fg(DIM)),
                Span::styled("ready", Style::default().fg(TEXT_DIM)),
            ])
        }
        Mode::Recording { duration_secs, energy, .. } => {
            let secs = *duration_secs;
            let dot_color = if (state.tick / 8) % 2 == 0 { RED_PULSE } else { Color::Rgb(180, 40, 35) };
            // Level bar
            let bar_len = ((*energy * 20.0) as usize).min(20);
            let bar: String = "█".repeat(bar_len);
            let bar_empty: String = "░".repeat(20 - bar_len);
            Line::from(vec![
                Span::styled("● ", Style::default().fg(dot_color)),
                Span::styled("recording ", Style::default().fg(PINK).add_modifier(Modifier::BOLD)),
                Span::styled(format!("{secs:.1}s "), Style::default().fg(TEXT)),
                Span::styled(bar, Style::default().fg(PINK)),
                Span::styled(bar_empty, Style::default().fg(SURFACE_LIGHT)),
            ])
        }
        Mode::Transcribing { duration_secs } => {
            let spinner = SPINNER[(state.tick / 4) as usize % SPINNER.len()];
            Line::from(vec![
                Span::styled(format!("{spinner} "), Style::default().fg(BLUE)),
                Span::styled("transcribing", Style::default().fg(BLUE).add_modifier(Modifier::BOLD)),
                Span::styled(format!("  {duration_secs:.1}s of audio"), Style::default().fg(TEXT_DIM)),
            ])
        }
        Mode::Result { ref text, copied } => {
            let copy_status = if *copied {
                Span::styled("  ✓ copied", Style::default().fg(GREEN))
            } else {
                Span::styled("  C to copy", Style::default().fg(TEXT_DIM))
            };
            let truncated: String = text.chars().take(60).collect();
            let ellipsis = if text.len() > 60 { "..." } else { "" };
            Line::from(vec![
                Span::styled("✓ ", Style::default().fg(GREEN)),
                Span::styled(format!("{truncated}{ellipsis}"), Style::default().fg(TEXT)),
                copy_status,
            ])
        }
        Mode::Error { ref message } => {
            Line::from(vec![
                Span::styled("✗ ", Style::default().fg(RED_PULSE)),
                Span::styled(message.clone(), Style::default().fg(Color::Rgb(255, 120, 120))),
            ])
        }
    };

    f.render_widget(
        Paragraph::new(line).style(Style::default().bg(SURFACE)),
        inner,
    );
}

fn draw_history(f: &mut Frame, area: Rect, state: &AppState) {
    if state.history.is_empty() {
        return;
    }

    let block = Block::default()
        .style(Style::default().bg(SURFACE));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();
    for (i, entry) in state.history.iter().enumerate().rev().take(20).collect::<Vec<_>>().into_iter().rev() {
        let num = format!(" {} ", i + 1);
        lines.push(Line::from(vec![
            Span::styled(num, Style::default().fg(DIM).bg(SURFACE_LIGHT)),
            Span::styled(" ", Style::default()),
            Span::styled(entry.clone(), Style::default().fg(TEXT)),
        ]));
        lines.push(Line::from(""));
    }

    let para = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .style(Style::default().bg(SURFACE));
    f.render_widget(para, inner);
}

#[derive(Clone)]
pub enum Mode {
    Idle,
    Recording {
        duration_secs: f32,
        energy: f64,
    },
    Transcribing {
        duration_secs: f32,
    },
    Result {
        text: String,
        copied: bool,
    },
    Error {
        message: String,
    },
}
