//! Рендер читалки: btop-оболочка — рамка текста со скруглёнными углами,
//! метрики в заголовке, слот тоста/команд над клавиатурным баром.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState,
};

use crate::app::{App, InputPurpose};
use crate::model::{BlockKind, LineInfo};
use crate::parse::txt::{list_marker, strip_heading};
use crate::ui::note_color;
use crate::ui::shelf::format_date;

/// Ниже этого покрытия в заголовке рамки появляется бейдж качества.
const QUALITY_WARN: f32 = 0.9;
/// Хвост клавиатурного бара: подписанные клавиши после цифровых панелей.
const BAR_WIDE_AT: u16 = 70;
const BAR_HINT_WIDE: &str = " · t язык · h полка · ? справка · q выход";
const BAR_HINT_NARROW: &str = " · t h ? q";
/// Ширина панели прогресса в заголовке.
const PROGRESS_CELLS: usize = 8;

pub fn render(app: &App, frame: &mut Frame) {
    let area = frame.area();
    if area.width == 0 || area.height == 0 {
        return;
    }
    // Компактный режим для очень низких терминалов: старый status-бар.
    if area.height < 6 {
        render_compact(app, frame, area);
        return;
    }
    let bar_y = area.y + area.height - 1;
    let slot_y = bar_y - 1;
    let frame_area = Rect { x: area.x, y: area.y, width: area.width, height: slot_y - area.y };
    render_text(app, frame, frame_area);
    render_slot(app, frame, slot_y, area.width);
    render_bar(app, frame, area, bar_y);
    render_panels(app, frame, area);
}

/// Минимальный рендер для окна ниже шести строк: текст и status-строка.
fn render_compact(app: &App, frame: &mut Frame, area: Rect) {
    let text_area = Rect { height: area.height.saturating_sub(1), ..area };
    if text_area.height > 0 {
        render_text_lines(app, frame, text_area);
    }
    let status_area = Rect { x: area.x, y: area.y + area.height - 1, width: area.width, height: 1 };
    frame.render_widget(Paragraph::new(compact_status(app, area.width)), status_area);
}

fn render_text(app: &App, frame: &mut Frame, area: Rect) {
    let inner = Rect {
        x: area.x + 1,
        y: area.y + 1,
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    };
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let block = Block::bordered().border_type(BorderType::Rounded).title(title_line(app));
    render_text_lines(app, frame, inner);
    let mut state = ScrollbarState::new(app.max_scroll()).position(app.scroll());
    frame.render_stateful_widget(
        Scrollbar::new(ScrollbarOrientation::VerticalRight),
        inner,
        &mut state,
    );
    frame.render_widget(block, area);
}

fn render_text_lines(app: &App, frame: &mut Frame, area: Rect) {
    let lines: Vec<Line> = app
        .layout()
        .lines()
        .iter()
        .skip(app.scroll())
        .take(area.height as usize)
        .map(|info| text_line(app, info))
        .collect();
    frame.render_widget(Paragraph::new(lines), area);
}

fn text_line(app: &App, info: &LineInfo) -> Line<'static> {
    let block = app.document().block(info.block).expect("блок из раскладки");
    let (pad, marker, style) = decoration(block.kind);
    let mut text = info.slice(block).trim_end().to_owned();
    if info.line_in_block == 0 {
        text = strip_marker(block.kind, &text);
    }
    let prefix = if info.line_in_block == 0 && !marker.is_empty() {
        marker.to_owned()
    } else {
        " ".repeat(pad)
    };
    let mut spans = Vec::new();
    if let Some(color) = app.note_color(info.block) {
        spans.push(Span::styled("▎", Style::new().fg(note_color(color))));
    }
    spans.push(Span::styled(prefix, style));
    spans.push(Span::styled(text, style));
    Line::from(spans)
}

/// Метрики в заголовке рамки: книга, языки, прогресс-бар, позиция, качество,
/// метка заметки на текущем блоке.
fn title_line(app: &App) -> Line<'static> {
    let total = app.document().len();
    let block = app.anchor().block + 1;
    let percent = app.percent();
    let mut spans = vec![
        Span::raw(format!(" {} · ", app.title())),
        Span::styled(langs(app), Style::new().add_modifier(Modifier::BOLD)),
    ];
    if let Some(coverage) = app.coverage().filter(|value| *value < QUALITY_WARN) {
        spans.push(Span::styled(
            format!(" ⚠{:.0}%", coverage * 100.0),
            Style::new().fg(Color::Yellow),
        ));
    }
    spans.push(Span::raw(" "));
    spans.push(Span::styled(progress_bar(percent), Style::new().fg(Color::Cyan)));
    spans.push(Span::raw(format!("{percent:.0}%")));
    spans.push(Span::raw(format!(" {block}/{total}")));
    if let Some(bookmark) = app.bookmark_at() {
        spans.push(Span::styled(
            format!(" · {}", bookmark.label),
            Style::new().fg(note_color(bookmark.color)),
        ));
    }
    Line::from(spans)
}

/// Полоска прогресса `▓▓▓░` из восьми клеток.
fn progress_bar(percent: f32) -> String {
    let filled = ((percent / 100.0) * PROGRESS_CELLS as f32).round() as usize;
    let filled = filled.min(PROGRESS_CELLS);
    "▓".repeat(filled) + &"░".repeat(PROGRESS_CELLS - filled)
}

/// Слот над баром: командная строка, тост или пустая строка.
fn render_slot(app: &App, frame: &mut Frame, y: u16, width: u16) {
    let slot = Rect { x: 0, y, width, height: 1 };
    let line = if let Some(buffer) = app.typing_buffer() {
        let label = match app.typing_purpose() {
            Some(InputPurpose::AddBook) => "Путь:",
            Some(InputPurpose::Command) => ":",
            _ => "Заметка:",
        };
        Some(Line::from(vec![
            Span::styled(label, Style::new().add_modifier(Modifier::BOLD)),
            Span::raw(format!(" {buffer}_")),
        ]))
    } else {
        app.notice().map(|notice| Line::from(Span::styled(notice, Style::new().fg(Color::Green))))
    };
    frame.render_widget(Paragraph::new(line.unwrap_or_default()), slot);
}

/// Клавиатурный бар btop в нижней строке. Числовая панель подсвечивается
/// жёлтым, когда открыта.
fn render_bar(app: &App, frame: &mut Frame, area: Rect, y: u16) {
    let bar_area = Rect { x: area.x, y, width: area.width, height: 1 };
    let wide = area.width >= BAR_WIDE_AT;
    let mut spans = Vec::new();
    push_digit(&mut spans, "1", "Текст", true);
    spans.push(Span::raw(" "));
    push_digit(&mut spans, "2", "Главы", app.toc_open());
    spans.push(Span::raw(" "));
    push_digit(&mut spans, "3", "Заметки", app.bookmarks_open());
    if wide {
        spans.push(Span::raw(" "));
        let command_active = app.typing_purpose() == Some(InputPurpose::Command);
        push_digit(&mut spans, "5", "Команды", command_active);
        let hint = BAR_HINT_WIDE;
        spans.push(Span::styled(hint, Style::new().add_modifier(Modifier::DIM)));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), bar_area);
}

/// Цифра панели и её подпись; активная — жёлтая.
fn push_digit<'a>(spans: &mut Vec<Span<'a>>, digit: &'a str, name: &'a str, active: bool) {
    let style = if active {
        Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD)
    } else {
        Style::new().fg(Color::Cyan)
    };
    spans.push(Span::styled(format!("{digit}:{name}"), style));
}

fn render_panels(app: &App, frame: &mut Frame, area: Rect) {
    let mut bottom = area.y + area.height - 2;
    if app.bookmarks_open() {
        bottom = render_bookmarks(app, frame, area.x, area.width, bottom);
    }
    if app.toc_open() {
        bottom = render_toc(app, frame, area.x, area.width, bottom);
    }
    if app.help_open() {
        render_help(frame, area.x, area.width, bottom);
    }
}

/// Панель в рамке со скруглёнными углами: список строк + нижняя граница.
fn render_panel(
    frame: &mut Frame,
    x: u16,
    width: u16,
    bottom: u16,
    title: &str,
    items: Vec<Line<'static>>,
) -> u16 {
    let max_rows = bottom.saturating_sub(4) as usize;
    let content_rows = items.len().min(max_rows);
    let height = (content_rows + 2) as u16;
    let y = bottom - height;
    let block = Block::bordered().border_type(BorderType::Rounded).title(Line::from(Span::styled(
        title.to_string(),
        Style::new().add_modifier(Modifier::BOLD),
    )));
    let inner =
        Rect { x: x + 1, y: y + 1, width: width.saturating_sub(2), height: content_rows as u16 };
    frame.render_widget(Paragraph::new(items), inner);
    frame.render_widget(block, Rect { x, y, width, height });
    y
}

fn render_bookmarks(app: &App, frame: &mut Frame, x: u16, width: u16, bottom: u16) -> u16 {
    let bookmarks = app.bookmarks();
    let rows = bookmarks.len().min(bottom.saturating_sub(2).saturating_sub(1) as usize);
    let start = app.bookmark_cursor().saturating_sub(rows.saturating_sub(1));
    let mut lines = Vec::with_capacity(rows);
    for (index, bookmark) in bookmarks.iter().enumerate().skip(start).take(rows) {
        let selected = index == app.bookmark_cursor();
        let (marker, style) = if selected {
            ("►", Style::new().add_modifier(Modifier::BOLD))
        } else {
            (" ", Style::new())
        };
        lines.push(Line::from(vec![
            Span::styled(marker, style),
            Span::styled(" ", style),
            Span::styled("● ", Style::new().fg(note_color(bookmark.color))),
            Span::styled(bookmark.label.clone(), style),
            Span::styled(format!(" · {}", format_date(bookmark.created_at)), style),
        ]));
    }
    render_panel(frame, x, width, bottom, "Заметки", lines)
}

fn render_toc(app: &App, frame: &mut Frame, x: u16, width: u16, bottom: u16) -> u16 {
    let items = app.document().toc();
    if items.is_empty() {
        let lines = vec![Line::from(Span::styled(
            "Заголовков нет",
            Style::new().add_modifier(Modifier::DIM),
        ))];
        return render_panel(frame, x, width, bottom, "Главы", lines);
    }
    let rows = items.len().min(bottom.saturating_sub(2).saturating_sub(1) as usize);
    let start = app.toc_cursor().saturating_sub(rows.saturating_sub(1));
    let mut lines = Vec::with_capacity(rows);
    for (index, item) in items.iter().enumerate().skip(start).take(rows) {
        let selected = index == app.toc_cursor();
        let (marker, style) = if selected {
            ("►", Style::new().add_modifier(Modifier::BOLD))
        } else {
            (" ", Style::new())
        };
        let indent = "  ".repeat(item.level.saturating_sub(1) as usize);
        lines.push(Line::from(vec![
            Span::styled(marker, style),
            Span::styled(" ".to_owned(), style),
            Span::styled(format!("{indent}{}", item.title), style),
        ]));
    }
    render_panel(frame, x, width, bottom, "Главы", lines)
}

const HELP: &[&str] = &[
    "j/k, Space, PgUp/PgDn, g/G — прокрутка",
    "Ctrl+D/U — полстраницы, колесо мыши",
    "1 — только текст, 2/Главы, 3/Заметки, 5/: команды",
    "в : open <путь>, lang t|<код>, goto <N>, shelf, b, q",
    "t — язык, b — закладка, B — список, n/p — переход",
    "в списке: c/C — цвет дальше/назад, r — переименовать, D — удалить",
    "o — оглавление, Enter — к разделу",
    "[ / ] — уже/шире колонку",
    "h — полка, a — добавить, d — удалить",
    "q — выход",
];

fn render_help(frame: &mut Frame, x: u16, width: u16, bottom: u16) {
    let lines: Vec<Line> = HELP.iter().map(|text| Line::from(*text)).collect();
    render_panel(frame, x, width, bottom, "Справка", lines);
}

fn compact_status(app: &App, width: u16) -> Line<'static> {
    let total = app.document().len();
    let block = app.anchor().block + 1;
    let percent = app.percent();
    let hint = if width >= BAR_WIDE_AT { BAR_HINT_WIDE } else { BAR_HINT_NARROW };
    Line::from(vec![
        Span::raw(format!(" {} · ", app.title())),
        Span::styled(langs(app), Style::new().add_modifier(Modifier::BOLD)),
        Span::raw(format!(" · {percent:.1}% · блок {block}/{total}")),
        Span::styled(hint, Style::new().add_modifier(Modifier::DIM)),
    ])
}

fn langs(app: &App) -> String {
    let base = app.base_lang();
    let current = app.current_lang();
    if base == current { base.to_owned() } else { format!("{base}▸{current}") }
}

/// Отступ и маркер первого ряда по типу блока: продолжение блока получает
/// пустой префикс той же ширины, чтобы текст выровнен по левому краю.
fn decoration(kind: BlockKind) -> (usize, &'static str, Style) {
    match kind {
        BlockKind::Heading(_) => (0, "", Style::new().add_modifier(Modifier::BOLD)),
        BlockKind::Quote => (2, "▌ ", Style::new().add_modifier(Modifier::ITALIC)),
        BlockKind::Code => (2, "", Style::new().add_modifier(Modifier::DIM)),
        BlockKind::Verse => (4, "", Style::new().add_modifier(Modifier::ITALIC)),
        _ => (0, "", Style::new()),
    }
}

/// Убирает служебную разметку первой строки блока: решётки заголовка,
/// маркер цитаты и пункта списка.
fn strip_marker(kind: BlockKind, text: &str) -> String {
    match kind {
        BlockKind::Heading(_) => strip_heading(text).to_owned(),
        BlockKind::Quote => text
            .strip_prefix('>')
            .map_or_else(|| text.to_owned(), |rest| rest.trim_start().to_owned()),
        BlockKind::ListItem => list_marker(text).unwrap_or(text).to_owned(),
        _ => text.to_owned(),
    }
}
