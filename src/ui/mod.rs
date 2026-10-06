pub mod reader;
pub mod shelf;

use ratatui::Frame;
use ratatui::style::Color;

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

/// Точка входа рендера: полка либо btop-оболочка читалки.
pub fn render(app: &App, frame: &mut Frame) {
    if app.screen() == Screen::Shelf {
        shelf::render(app, frame);
        return;
    }
    reader::render(app, frame);
}
