//! Рендер полки: список книг с прогрессом, строка prompt и подсказка.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::{App, InputPurpose};

const HINT: &str = "Enter открыть · a добавить · d удалить · q выход";

pub fn render(app: &App, frame: &mut Frame) {
    let area = frame.area();
    if area.width == 0 || area.height == 0 {
        return;
    }
    let mut lines =
        vec![Line::from(Span::styled("Полка", Style::new().add_modifier(Modifier::BOLD)))];
    for (index, book) in app.shelf_books().iter().enumerate() {
        let selected = index == app.shelf_cursor();
        let style = if selected { Style::new().add_modifier(Modifier::BOLD) } else { Style::new() };
        let marker = if selected { "> " } else { "  " };
        lines.push(Line::from(vec![
            Span::styled(marker, style),
            Span::styled(book.title.clone(), style),
            Span::styled(
                format!(
                    " · {} · {:.1}% · {}",
                    book.langs.join(" "),
                    book.percent,
                    format_date(book.date)
                ),
                style,
            ),
        ]));
    }
    let text_height = area.height.saturating_sub(1);
    frame.render_widget(
        Paragraph::new(lines.into_iter().take(text_height as usize).collect::<Vec<_>>()),
        Rect { height: text_height, ..area },
    );
    let footer = footer_line(app);
    frame.render_widget(
        Paragraph::new(footer),
        Rect { x: area.x, y: area.y + text_height, width: area.width, height: 1 },
    );
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
        Line::from(Span::styled(HINT, Style::new().add_modifier(Modifier::DIM)))
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
