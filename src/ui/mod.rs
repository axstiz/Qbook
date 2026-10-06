pub mod reader;
pub mod shelf;

use ratatui::Frame;
use ratatui::style::Color;
use ratatui::text::{Line, Span};

use crate::app::{App, Screen};
use crate::store::NOTE_COLOR_COUNT;

/// Семь цветов заметки в порядке индексов хранилища: красный, зелёный,
/// жёлтый, синий, пурпурный, голубой, белый.
pub fn note_color(index: u8) -> Color {
    const COLORS: [Color; NOTE_COLOR_COUNT as usize] = [
        Color::Red,
        Color::Green,
        Color::Yellow,
        Color::Blue,
        Color::Magenta,
        Color::Cyan,
        Color::White,
    ];
    COLORS[usize::from(index).min(COLORS.len() - 1)]
}

/// btop-градиент прогресса: заполненная часть — от тёмно-красного к яркому
/// неоново-розовому, пустая — тускло-серые штрихи.
pub fn btop_gauge(percentage: f32, width: usize) -> Line<'static> {
    let percent = percentage.clamp(0.0, 100.0);
    let filled = ((width as f32) * (percent / 100.0)).round() as usize;
    let mut spans = Vec::with_capacity(width);
    for i in 0..width {
        if i < filled {
            let factor = i as f32 / width.max(1) as f32;
            let r = (90.0 + factor * 165.0) as u8;
            let g = (40.0 + factor * 30.0) as u8;
            let b = (50.0 + factor * 60.0) as u8;
            spans.push(Span::styled("▮", ratatui::style::Style::new().fg(Color::Rgb(r, g, b))));
        } else {
            spans.push(Span::styled("▯", ratatui::style::Style::new().fg(Color::Rgb(50, 50, 50))));
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
