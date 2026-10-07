pub mod reader;
pub mod shelf;

use ratatui::Frame;
use ratatui::style::Color;
use ratatui::text::{Line, Span};

use crate::app::{App, Screen};
use crate::config::Theme;

/// Цвет заметки по индексу хранилища из текущей темы.
pub fn note_color(theme: &Theme, index: u8) -> Color {
    theme.notes[usize::from(index).min(theme.notes.len() - 1)]
}

/// Градиент прогресса из цветов темы: заполненная часть интерполируется от
/// `gauge_start` к `gauge_end`, пустая — штрихи `gauge_empty`.
pub fn btop_gauge(theme: &Theme, percentage: f32, width: usize) -> Line<'static> {
    let (start, end, empty) = theme.gauge_colors();
    let percent = percentage.clamp(0.0, 100.0);
    let filled = ((width as f32) * (percent / 100.0)).round() as usize;
    let mix =
        |a: u8, b: u8, factor: f32| (f32::from(a) + (f32::from(b) - f32::from(a)) * factor) as u8;
    let mut spans = Vec::with_capacity(width);
    for i in 0..width {
        if i < filled {
            let factor = i as f32 / width.max(1) as f32;
            spans.push(Span::styled(
                "▮",
                ratatui::style::Style::new().fg(Color::Rgb(
                    mix(start.0, end.0, factor),
                    mix(start.1, end.1, factor),
                    mix(start.2, end.2, factor),
                )),
            ));
        } else {
            spans.push(Span::styled(
                "▯",
                ratatui::style::Style::new().fg(Color::Rgb(empty.0, empty.1, empty.2)),
            ));
        }
    }
    Line::from(spans)
}

/// Точка входа рендера: полка либо btop-оболочка читалки.
pub fn render(app: &App, frame: &mut Frame) {
    if app.screen() == Screen::Shelf {
        shelf::render(app, frame);
        return;
    }
    reader::render(app, frame);
}
