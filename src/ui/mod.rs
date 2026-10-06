pub mod reader;
pub mod shelf;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::{App, InputPurpose, Screen};

/// Точка входа рендера: полка либо читалка с оверлеями панели закладок и prompt.
pub fn render(app: &App, frame: &mut Frame) {
    if app.screen() == Screen::Shelf {
        shelf::render(app, frame);
        return;
    }
    let area = frame.area();
    if area.width == 0 || area.height == 0 {
        return;
    }
    reader::render(app, frame);
    // Статус занимает последнюю строку; prompt и панель — над ней.
    let mut top = area.y + area.height - 1;
    if let Some(buffer) = app.typing_buffer() {
        let y = top.saturating_sub(1).max(area.y);
        if y < top {
            let label = match app.typing_purpose() {
                Some(InputPurpose::AddBook) => "Путь:",
                _ => "Метка:",
            };
            let line = Line::from(vec![
                Span::styled(label, Style::new().add_modifier(Modifier::BOLD)),
                Span::raw(format!(" {buffer}_")),
            ]);
            frame.render_widget(
                Paragraph::new(line),
                Rect { x: area.x, y, width: area.width, height: 1 },
            );
        }
        top = y;
    }
    if app.bookmarks_open() {
        render_bookmarks(app, frame, area, top);
    }
}

fn render_bookmarks(app: &App, frame: &mut Frame, area: Rect, below: u16) {
    let available = below.saturating_sub(area.y);
    if available == 0 {
        return;
    }
    let bookmarks = app.bookmarks();
    let rows = bookmarks.len().min(available.saturating_sub(1) as usize);
    let height = rows.saturating_add(1) as u16;
    let y = below - height;
    let start = app.bookmark_cursor().saturating_sub(rows.saturating_sub(1));

    let mut lines =
        vec![Line::from(Span::styled("Закладки:", Style::new().add_modifier(Modifier::DIM)))];
    for (index, bookmark) in bookmarks.iter().enumerate().skip(start).take(rows) {
        let selected = index == app.bookmark_cursor();
        let (marker, style) = if selected {
            ("> ", Style::new().add_modifier(Modifier::BOLD))
        } else {
            ("  ", Style::new())
        };
        lines.push(Line::from(vec![
            Span::styled(marker, style),
            Span::styled(bookmark.label.clone(), style),
        ]));
    }
    frame.render_widget(Paragraph::new(lines), Rect { x: area.x, y, width: area.width, height });
}
