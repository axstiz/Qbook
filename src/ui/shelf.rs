//! Рендер полки в btop-стиле: рамка, курсор `►`, мини-прогресс `▓▓░`,
//! клавиатурный бар в нижней строке.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Paragraph};

use crate::app::{App, InputPurpose};
use crate::ui::btop_gauge;

const PROGRESS_CELLS: usize = 14;

/// Цветная «клавиша» в баре: жёлтая подпись на тёмном фоне.
fn key_span<'a>(label: &'a str) -> Span<'a> {
    Span::styled(label, Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD))
}

/// Пометки между клавишами в баре.
fn dim<'a>(text: &'a str) -> Span<'a> {
    Span::styled(text, Style::new().add_modifier(Modifier::DIM))
}

pub fn render(app: &App, frame: &mut Frame) {
    let area = frame.area();
    if area.width < 4 || area.height < 3 {
        return;
    }
    let shelf_area = Rect { height: area.height - 1, ..area };
    let inner = Rect {
        x: area.x + 1,
        y: area.y + 1,
        width: area.width.saturating_sub(2),
        height: shelf_area.height.saturating_sub(2),
    };
    let mut lines: Vec<Line> = Vec::new();
    if app.shelf_books().is_empty() {
        lines.push(Line::from(Span::styled(
            "— пусто: добавьте книгу клавишей a",
            Style::new().add_modifier(Modifier::DIM),
        )));
    } else {
        for (index, book) in app.shelf_books().iter().enumerate() {
            lines.push(book_line(book, index == app.shelf_cursor()));
        }
    }
    lines.truncate(inner.height as usize);
    frame.render_widget(Paragraph::new(lines), inner);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .title(Line::from(Span::styled(" Полка ", Style::new().add_modifier(Modifier::BOLD))));
    frame.render_widget(block, shelf_area);
    let bar = Rect { x: area.x, y: area.y + area.height - 1, width: area.width, height: 1 };
    frame.render_widget(Paragraph::new(footer_line(app)), bar);
}

fn book_line(book: &crate::app::ShelfBook, selected: bool) -> Line<'static> {
    let title_style =
        if selected { Style::new().add_modifier(Modifier::BOLD) } else { Style::new() };
    let dim = Style::new().add_modifier(Modifier::DIM);
    let marker = if selected {
        Span::styled("► ", Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD))
    } else {
        Span::raw("  ")
    };
    let mut spans = vec![
        marker,
        Span::styled(book.title.clone(), title_style),
        Span::styled(format!(" · {}", book.langs.join(" ")), dim),
        Span::raw(" "),
    ];
    spans.extend(btop_gauge(book.percent, PROGRESS_CELLS).spans);
    spans.push(Span::raw(" "));
    spans.push(Span::styled(format!("{:5.1}% · {}", book.percent, format_date(book.date)), dim));
    Line::from(spans)
}

fn footer_line(app: &App) -> Line<'static> {
    if let Some(buffer) = app.typing_buffer() {
        let label = match app.typing_purpose() {
            Some(InputPurpose::RenameBookmark | InputPurpose::NewBookmark) => "Заметка:",
            _ => "Путь:",
        };
        Line::from(vec![
            Span::styled(label, Style::new().add_modifier(Modifier::BOLD)),
            Span::raw(format!(" {buffer}_")),
        ])
    } else if let Some(error) = app.shelf_error() {
        Line::from(Span::styled(format!("⚠ {error}"), Style::new().fg(Color::Yellow)))
    } else {
        Line::from(vec![
            key_span("Enter"),
            dim(" открыть · "),
            key_span("a"),
            dim(" добавить · "),
            key_span("d"),
            dim(" удалить · "),
            key_span("q"),
            dim(" выход"),
        ])
    }
}

/// unix-время в UTC → `YYYY-MM-DD`.
pub fn format_date(secs: i64) -> String {
    let (year, month, day) = civil_from_days(secs.div_euclid(86_400));
    format!("{year:04}-{month:02}-{day:02}")
}

/// Дни с эпохи → календарная дата (алгоритм Говарда Хиннанта).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let doe = shifted.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}
