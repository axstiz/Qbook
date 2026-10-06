//! Рендер читалки: btop-оболочка — левая колонка глав, по центру рамка текста
//! с метриками, справа заметки/заметка/команды; слот и бар внизу.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState,
};

use crate::app::{App, COMMANDS, InputPurpose, LEFT_W, MIN_CENTER, RIGHT_W, ReaderFocus};
use crate::model::{BlockKind, LineInfo};
use crate::parse::txt::{list_marker, strip_heading};
use crate::ui::note_color;

/// Сколько команд показываем в блоке «Команды».
const COMMANDS_MAX_ROWS: usize = 12;

/// Ниже этого покрытия в заголовке рамки появляется бейдж качества.
const QUALITY_WARN: f32 = 0.9;
/// Хвост клавиатурного бара: подписанные клавиши после цифровых панелей.
const BAR_WIDE_AT: u16 = 70;
/// Ширина панели прогресса в заголовке.
const PROGRESS_CELLS: usize = 14;
/// Минимальная высота блока «Заметки» при делении правой колонки.
const BOOKMARKS_MIN: u16 = 3;

/// Динамическая подсказка бара: команды текущего блока + общие хвосты.
fn bar_hint(focus: ReaderFocus, wide: bool) -> &'static str {
    match (focus, wide) {
        (ReaderFocus::Text, true) => " · b заметка · v выд · j/k · t язык · h полка · q выход",
        (ReaderFocus::Toc, true) => " · j/k · Enter — к разделу · t язык · h полка · q выход",
        (ReaderFocus::Bookmarks, true) => {
            " · j/k · Enter — к заметке · c/C цвет · D удалить · q выход"
        }
        (ReaderFocus::Commands, true) => " · j/k · Enter — подставить · t язык · h полка · q выход",
        (ReaderFocus::Text, false) => " · b v t h q",
        (ReaderFocus::Toc, false) => " · j k Enter q",
        (ReaderFocus::Bookmarks, false) => " · j k c D q",
        (ReaderFocus::Commands, false) => " · j k Enter q",
    }
}

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
    let (left, center, right) = split_columns(
        frame_area,
        app.toc_visible(),
        app.bookmarks_visible() || app.commands_visible(),
    );
    render_text(app, frame, center, app.focus() == ReaderFocus::Text);
    if let Some(left) = left {
        render_toc(app, frame, left, app.focus() == ReaderFocus::Toc);
    }
    if let Some(right) = right {
        render_right(app, frame, right);
    }
    render_slot(app, frame, slot_y, area.width);
    render_bar(app, frame, area, bar_y);
    if app.help_open() {
        render_help(frame, center);
    }
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

/// Разбить верхнюю область на колонки. При малой ширине сначала прячется правая
/// колонка, затем левая, остаётся только растянутый текст.
fn split_columns(
    area: Rect,
    show_left: bool,
    show_right: bool,
) -> (Option<Rect>, Rect, Option<Rect>) {
    let full = area.x + area.width;
    if show_left && show_right && area.width >= LEFT_W + RIGHT_W + 2 + MIN_CENTER {
        let left = Rect { x: area.x, y: area.y, width: LEFT_W, height: area.height };
        let right = Rect { x: full - RIGHT_W, y: area.y, width: RIGHT_W, height: area.height };
        let center = Rect {
            x: area.x + LEFT_W + 1,
            y: area.y,
            width: area.width - LEFT_W - RIGHT_W - 2,
            height: area.height,
        };
        (Some(left), center, Some(right))
    } else if show_left && area.width >= LEFT_W + 1 + MIN_CENTER {
        let left = Rect { x: area.x, y: area.y, width: LEFT_W, height: area.height };
        let center = Rect {
            x: area.x + LEFT_W + 1,
            y: area.y,
            width: area.width - LEFT_W - 1,
            height: area.height,
        };
        (Some(left), center, None)
    } else if show_right && area.width >= 1 + MIN_CENTER + RIGHT_W {
        let right = Rect { x: full - RIGHT_W, y: area.y, width: RIGHT_W, height: area.height };
        let center =
            Rect { x: area.x, y: area.y, width: area.width - RIGHT_W - 1, height: area.height };
        (None, center, Some(right))
    } else {
        (None, area, None)
    }
}

fn render_text(app: &App, frame: &mut Frame, area: Rect, active: bool) {
    let inner = Rect {
        x: area.x + 1,
        y: area.y + 1,
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    };
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let border = if active { Style::new() } else { Style::new().fg(Color::DarkGray) };
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(border)
        .title(title_line(app));
    render_text_lines(app, frame, inner);
    let mut state = ScrollbarState::new(app.max_scroll()).position(app.scroll());
    frame.render_stateful_widget(
        Scrollbar::new(ScrollbarOrientation::VerticalRight),
        inner,
        &mut state,
    );
    frame.render_widget(block, area);
    fade_bottom(frame, inner);
}

/// Градиент внизу текста: последние строки плавно гаснут к нижнему краю.
fn fade_bottom(frame: &mut Frame, inner: Rect) {
    let rows = 3.min(inner.height);
    for i in 0..rows {
        let y = inner.y + inner.height - 1 - i;
        for x in inner.x..inner.x + inner.width {
            let cell = &mut frame.buffer_mut()[(x, y)];
            let mut style = cell.style();
            style = style.add_modifier(Modifier::DIM);
            if i == 0 {
                style = style.fg(Color::DarkGray);
            }
            cell.set_style(style);
        }
    }
}

fn render_text_lines(app: &App, frame: &mut Frame, area: Rect) {
    let selection = app.selection_range();
    let pick_row = app.pick_row();
    let lines: Vec<Line> = app
        .layout()
        .lines()
        .iter()
        .enumerate()
        .skip(app.scroll())
        .take(area.height as usize)
        .map(|(index, info)| {
            let mut line = text_line(app, info);
            if selection.is_some_and(|(top, bottom)| index >= top && index <= bottom) {
                line.style = Style::new().bg(Color::DarkGray);
            }
            if pick_row == Some(index) {
                line.style = Style::new().add_modifier(Modifier::REVERSED);
            }
            line
        })
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
    if let Some(color) = app.note_line_color(info) {
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

/// Рамка в правой колонке со скруглёнными углами и подписанным заголовком.
/// Неактивный блок приглушается, активный остаётся ярким.
fn block_frame(title: &str, active: bool) -> Block<'static> {
    let title_style = if active {
        Style::new().add_modifier(Modifier::BOLD)
    } else {
        Style::new().fg(Color::DarkGray).add_modifier(Modifier::DIM)
    };
    let border = if active { Style::new() } else { Style::new().fg(Color::DarkGray) };
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(border)
        .title(Line::from(Span::styled(title.to_string(), title_style)))
}

/// Рамка с внутренней областью для одного из блоков среды колонки.
fn inner_of(area: Rect) -> Rect {
    Rect {
        x: area.x + 1,
        y: area.y + 1,
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    }
}

/// Левая колонка «Главы»: активный раздел подсвечен, навигационный курсор `►`.
fn render_toc(app: &App, frame: &mut Frame, area: Rect, active: bool) {
    let inner = inner_of(area);
    let items = app.document().toc();
    let mut lines: Vec<Line> = Vec::new();
    if items.is_empty() {
        lines.push(Line::from(Span::styled(
            "Заголовков нет",
            Style::new().add_modifier(Modifier::DIM),
        )));
    } else {
        let nav = app.toc_cursor();
        let active = app.active_heading().unwrap_or(0);
        let anchor = if app.focus() == ReaderFocus::Toc { nav } else { active };
        let rows = inner.height as usize;
        let start = anchor.saturating_sub(rows.saturating_sub(1));
        for (index, item) in items.iter().enumerate().skip(start).take(rows) {
            let selected = index == nav;
            let is_active = index == active;
            let marker = if selected { "►" } else { " " };
            let title_style = if is_active {
                Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD)
            } else {
                Style::new()
            };
            let indent = "  ".repeat(item.level.saturating_sub(1) as usize);
            lines.push(Line::from(vec![
                Span::styled(marker, Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
                Span::raw(" "),
                Span::styled(format!("{indent}{}", item.title), title_style),
            ]));
        }
    }
    frame.render_widget(Paragraph::new(lines), inner);
    frame.render_widget(block_frame(" Главы ", active), area);
}

/// Правая колонка: снизу список команд, над ним заметки. Когда места мало,
/// блок команд прячется сам — заметки остаются; в фокусе команд он всегда виден.
fn render_right(app: &App, frame: &mut Frame, area: Rect) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let mut above = area;
    let focused = app.focus() == ReaderFocus::Commands;
    if app.commands_visible() && (focused || above.height >= BOOKMARKS_MIN + 3) {
        let limit = if focused { above.height } else { above.height - BOOKMARKS_MIN };
        let cmds_height = (COMMANDS.len().min(COMMANDS_MAX_ROWS) as u16 + 2).min(limit);
        let cmds_area = Rect {
            x: area.x,
            y: above.y + above.height - cmds_height,
            width: area.width,
            height: cmds_height,
        };
        render_commands(app, frame, cmds_area, focused);
        above.height -= cmds_height;
    }
    if above.height > 0 {
        render_bookmarks(app, frame, above, app.focus() == ReaderFocus::Bookmarks);
    }
}

/// Список заметок книги, упорядоченных по позиции, с курсором `►`.
fn render_bookmarks(app: &App, frame: &mut Frame, area: Rect, active: bool) {
    let inner = inner_of(area);
    let bookmarks = app.bookmarks();
    let rows = inner.height as usize;
    let start = app.bookmark_cursor().saturating_sub(rows.saturating_sub(1));
    let mut lines: Vec<Line> = Vec::new();
    if bookmarks.is_empty() {
        lines.push(Line::from(Span::styled("— пусто", Style::new().add_modifier(Modifier::DIM))));
    }
    for (index, bookmark) in bookmarks.iter().enumerate().skip(start).take(rows) {
        let selected = index == app.bookmark_cursor();
        let style = if selected { Style::new().add_modifier(Modifier::BOLD) } else { Style::new() };
        lines.push(Line::from(vec![
            Span::styled("► ", Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            Span::styled("● ", Style::new().fg(note_color(bookmark.color))),
            Span::styled(bookmark.label.clone(), style),
            Span::styled(format!(" · {}", format_date(bookmark.created_at)), style),
        ]));
    }
    frame.render_widget(Paragraph::new(lines), inner);
    frame.render_widget(block_frame(" Заметки ", active), area);
}

/// Список полезных команд: Enter подставляет выбранную в командную строку.
fn render_commands(app: &App, frame: &mut Frame, area: Rect, active: bool) {
    let inner = inner_of(area);
    let rows = inner.height as usize;
    let start = app.command_cursor().saturating_sub(rows.saturating_sub(1));
    let mut lines: Vec<Line> = Vec::new();
    for (index, (name, args, help)) in COMMANDS.iter().enumerate().skip(start).take(rows) {
        let selected = index == app.command_cursor();
        let marker = if selected { "►" } else { " " };
        let command = if args.is_empty() { name.to_string() } else { format!("{name} {args}") };
        let name_style = if selected {
            Style::new().add_modifier(Modifier::BOLD)
        } else {
            Style::new().add_modifier(Modifier::DIM)
        };
        lines.push(Line::from(vec![
            Span::styled(marker, Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            Span::raw(" "),
            Span::styled(format!(":{command}"), name_style),
            Span::styled(format!(" — {help}"), Style::new().add_modifier(Modifier::DIM)),
        ]));
    }
    frame.render_widget(Paragraph::new(lines), inner);
    frame.render_widget(block_frame(" Команды ", active), area);
}

/// Слот над баром: командная строка `:`, ввод метки заметки или тост.
fn render_slot(app: &App, frame: &mut Frame, y: u16, width: u16) {
    let slot = Rect { x: 0, y, width, height: 1 };
    let line = if let Some(buffer) = app.typing_buffer() {
        let purpose = app.typing_purpose();
        let label = match purpose {
            Some(InputPurpose::Command) => ":",
            _ => "Заметка:",
        };
        let mut spans = vec![
            Span::styled(label, Style::new().add_modifier(Modifier::BOLD)),
            Span::raw(format!(" {buffer}_")),
        ];
        match purpose {
            Some(InputPurpose::NewBookmark) => {
                if let Some(color) = app.pending_bookmark_color() {
                    spans.push(Span::styled(" ●", Style::new().fg(note_color(color))));
                    spans
                        .push(Span::raw(format!(" {} · c/C · Enter", App::note_color_name(color))));
                }
            }
            Some(InputPurpose::RenameBookmark) => {
                spans.push(Span::styled(
                    "  Enter — сохранить · Esc — отмена",
                    Style::new().add_modifier(Modifier::DIM),
                ));
            }
            _ => {}
        }
        Some(Line::from(spans))
    } else if let Some(hint) = app.pick_hint() {
        Some(Line::from(Span::styled(hint, Style::new().add_modifier(Modifier::DIM))))
    } else {
        app.notice().map(|notice| Line::from(Span::styled(notice, Style::new().fg(Color::Green))))
    };
    frame.render_widget(Paragraph::new(line.unwrap_or_default()), slot);
}

/// Клавиатурный бар btop в нижней строке. Цифра подсвечивается жёлтым, когда
/// соответствующая колонка включена.
fn render_bar(app: &App, frame: &mut Frame, area: Rect, y: u16) {
    let bar_area = Rect { x: area.x, y, width: area.width, height: 1 };
    let wide = area.width >= BAR_WIDE_AT;
    let mut spans = Vec::new();
    push_digit(&mut spans, "1", "Текст", app.focus() == ReaderFocus::Text);
    spans.push(Span::raw(" "));
    push_digit(&mut spans, "2", "Главы", app.focus() == ReaderFocus::Toc);
    spans.push(Span::raw(" "));
    push_digit(&mut spans, "3", "Заметки", app.focus() == ReaderFocus::Bookmarks);
    spans.push(Span::raw(" "));
    push_digit(&mut spans, "4", "Команды", app.focus() == ReaderFocus::Commands);
    if wide {
        let hint = bar_hint(app.focus(), true);
        spans.push(Span::styled(hint, Style::new().add_modifier(Modifier::DIM)));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), bar_area);
}

/// Цифра и подпись блока: выбранный тусклым, остальные — яркими.
fn push_digit<'a>(spans: &mut Vec<Span<'a>>, digit: &'a str, name: &'a str, active: bool) {
    let style = if active {
        Style::new().fg(Color::DarkGray)
    } else {
        Style::new().add_modifier(Modifier::BOLD)
    };
    spans.push(Span::styled(format!("{digit}:{name}"), style));
}

/// Оверлей справки над слотом, не перекрывая метрики в заголовке центра.
fn render_help(frame: &mut Frame, center: Rect) {
    let max_rows = center.height.saturating_sub(4) as usize;
    let height = (HELP.len().min(max_rows) + 2) as u16;
    if height < 2 {
        return;
    }
    let width = center.width.min(64);
    let area = Rect { x: center.x, y: center.y + center.height - height, width, height };
    let lines: Vec<Line> = HELP.iter().map(|text| Line::from(*text)).collect();
    let inner =
        Rect { x: area.x + 1, y: area.y + 1, width: width.saturating_sub(2), height: height - 2 };
    frame.render_widget(Paragraph::new(lines), inner);
    frame.render_widget(block_frame(" Справка ", true), area);
}

const HELP: &[&str] = &[
    "j/k, Space, PgUp/PgDn, g/G — прокрутка",
    "цифра 1–4 — фокус блока · Shift+цифра — показать/скрыть",
    "2/o — оглавление · 3/B — заметки · 4 — команды · 1 — текст",
    "заметки: b ввод, c/C цвет, r правка, D удалить, n/p переход",
    "в : open <путь>, lang t|<код>, goto <N>, shelf, b, q",
    "в колонках: j/k — курсор, Enter — к разделу/заметке",
    "[ / ] — уже/шире колонку · h — полка · ? — справка",
    "заметка видна в блоке слева · Esc — к тексту · q — выход",
];

fn compact_status(app: &App, width: u16) -> Line<'static> {
    let total = app.document().len();
    let block = app.anchor().block + 1;
    let percent = app.percent();
    let hint = bar_hint(app.focus(), width >= BAR_WIDE_AT);
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

/// unix-время в UTC → `YYYY-MM-DD`.
fn format_date(secs: i64) -> String {
    crate::ui::shelf::format_date(secs)
}
