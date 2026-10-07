//! Рендер читалки: btop-оболочка — левая колонка глав, по центру рамка текста
//! с метриками, справа заметки/заметка/команды; слот и бар внизу.

use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Clear, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState,
};

use crate::app::{
    App, COMMANDS, InputPurpose, LEFT_W, MIN_CENTER, RIGHT_W, ReaderFocus, ResizeLimit,
    TEXT_INDENT, WIN_PAD,
};
use crate::model::{BlockKind, LineInfo};
use crate::parse::txt::{list_marker, strip_heading};
use crate::ui::{btop_gauge, note_color};

/// Сколько команд показываем в блоке «Команды».
const COMMANDS_MAX_ROWS: usize = 12;

/// Ниже этого покрытия в заголовке рамки появляется бейдж качества.
const QUALITY_WARN: f32 = 0.9;
/// Хвост клавиатурного бара: подписанные клавиши после цифровых панелей.
const BAR_WIDE_AT: u16 = 70;
/// Ширина панели прогресса в заголовке (в 3 раза больше прежних 8 ячеек).
const FRAME_PROGRESS_CELLS: usize = 24;
/// Минимальная высота блока «Заметки» при делении правой колонки.
const BOOKMARKS_MIN: u16 = 3;

/// Динамическая подсказка бара: клавиши текущего блока жёлтым (как на полке),
/// описания — тусклые. `t язык` только у текста, `s полка` и `h справка`
/// работают из любого блока.
fn bar_hint(app: &App, wide: bool) -> Line<'static> {
    let focus = app.focus();
    let pairs: &[(&str, &str)] = match (focus, wide) {
        (ReaderFocus::Text, true) => &[
            ("b", "заметка"),
            ("v", "выд"),
            ("t", "язык"),
            ("s", "полка"),
            ("h", "справка"),
            ("q", "выход"),
        ],
        (ReaderFocus::Toc, true) => {
            &[("j/k", ""), ("Enter", "— к разделу"), ("s", "полка"), ("q", "выход")]
        }
        (ReaderFocus::Bookmarks, true) => &[
            ("j/k", ""),
            ("Enter", "— к заметке"),
            ("c/C", "цвет"),
            ("D", "удалить"),
            ("s", "полка"),
            ("q", "выход"),
        ],
        (ReaderFocus::Commands, true) => {
            &[("j/k", ""), ("Enter", "— подставить"), ("s", "полка"), ("q", "выход")]
        }
        (ReaderFocus::Text, false) => {
            &[("b", ""), ("v", ""), ("t", ""), ("s", ""), ("h", ""), ("q", "")]
        }
        (ReaderFocus::Toc, false) => &[("j", ""), ("k", ""), ("Enter", ""), ("q", "")],
        (ReaderFocus::Bookmarks, false) => &[("j", ""), ("k", ""), ("c", ""), ("D", ""), ("q", "")],
        (ReaderFocus::Commands, false) => &[("j", ""), ("k", ""), ("Enter", ""), ("q", "")],
    };
    let sep = if wide { " · " } else { " " };
    let mut spans = Vec::new();
    for (i, (key, desc)) in pairs.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw(sep));
        }
        spans.push(Span::styled(*key, Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD)));
        if !desc.is_empty() {
            spans.push(Span::styled(format!(" {desc}"), Style::new().add_modifier(Modifier::DIM)));
        }
    }
    if let Some(limit) = app.column_limit()
        && focus == ReaderFocus::Text
    {
        let msg = match limit {
            ResizeLimit::Narrow => " [ — уже предел ",
            ResizeLimit::Wide => " ] — шире некуда ",
        };
        spans.push(Span::styled(msg, Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD)));
    }
    Line::from(spans)
}

pub fn render(app: &App, frame: &mut Frame) {
    let area = frame.area();
    if area.width == 0 || area.height == 0 {
        return;
    }
    // Компактный режим для очень низких терминалов: старый status-бар.
    if area.height < 7 {
        render_compact(app, frame, area);
        return;
    }
    let bar_y = area.y + area.height - 1;
    let slot_y = bar_y - 1;
    let pad = WIN_PAD;
    let header_width = area.width.saturating_sub(2 * pad);
    render_header(app, frame, area.x + pad, header_width, area.y);
    let frame_area = Rect {
        x: area.x + pad,
        y: area.y + pad + 1,
        width: header_width,
        height: slot_y.saturating_sub(area.y + pad + 1).saturating_sub(pad),
    };
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
    render_slot(app, frame, slot_y, area.x + pad, area.width.saturating_sub(2 * pad));
    render_bar(app, frame, area.x + pad, area.width.saturating_sub(2 * pad), bar_y);
    if app.help_open() {
        // Справка рисуется поверх всего, а остальной буфер тускнеет: под
        // модальным окном читатель видит тёмно-серый фон, а не контент.
        if let Some(area) = help_area(center) {
            render_help(frame, area);
            dim_except(frame, area);
        }
    }
}

/// Модальная рамка справки: центрируется в колонке текста и не выходит за неё.
fn help_area(center: Rect) -> Option<Rect> {
    let rows = HELP.len();
    let max_rows = center.height.saturating_sub(4) as usize;
    let height = rows.min(max_rows) + 2;
    if height < 2 {
        return None;
    }
    let width = center.width.min(64);
    let height = height as u16;
    let x = center.x + (center.width.saturating_sub(width)) / 2;
    let y = center.y + (center.height.saturating_sub(height)) / 2;
    Some(Rect { x, y, width, height })
}

/// Затемняет всё, кроме модальной справки: подложка под неё становится
/// тёмно-серой и приглушённой.
fn dim_except(frame: &mut Frame, keep: Rect) {
    let area = frame.area();
    let buffer = frame.buffer_mut();
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            if keep.contains(Position::new(x, y)) {
                continue;
            }
            buffer[(x, y)].set_style(Style::new().fg(Color::DarkGray).add_modifier(Modifier::DIM));
        }
    }
}

/// Минимальный рендер для окна ниже шести строк: текст и status-строка.
fn render_compact(app: &App, frame: &mut Frame, area: Rect) {
    let text_area = Rect { height: area.height.saturating_sub(1), ..area };
    if text_area.height > 0 {
        render_text_lines(app, frame, text_area, text_area.width);
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
        x: area.x + 1 + TEXT_INDENT,
        y: area.y + 1,
        width: area.width.saturating_sub(2).saturating_sub(2 * TEXT_INDENT),
        height: area.height.saturating_sub(2),
    };
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    // Честная колонка: текст занимает `wrap_width`, а не всю рамку — справа
    // остаётся пустое место, показывающее ручное сужение `[`/`]`. Плюс два
    // столбца на однострочные префиксы списков и цитат, чтобы они не переносились.
    let col = app.wrap_width().min(inner.width).saturating_add(2).min(inner.width);
    let text_area = Rect { x: inner.x, y: inner.y, width: col, height: inner.height };
    frame.render_widget(Clear, inner);
    render_text_lines(app, frame, text_area, inner.width);
    if app.content_max_scroll() > 0 {
        let mut state = ScrollbarState::new(app.max_scroll()).position(app.scroll());
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight),
            text_area,
            &mut state,
        );
    }
    frame.render_widget(block_frame("1", text_title(app, area.width), active), area);
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

fn render_text_lines(app: &App, frame: &mut Frame, area: Rect, rule_width: u16) {
    let selection = app.selection_range();
    let pick_row = app.pick_row();
    let mut rule_rows: Vec<usize> = Vec::new();
    let lines: Vec<Line> = app
        .layout()
        .lines()
        .iter()
        .enumerate()
        .skip(app.scroll())
        .take(area.height as usize)
        .enumerate()
        .map(|(row, (index, info))| {
            if app.document().block(info.block).is_some_and(|b| b.kind == BlockKind::Rule) {
                rule_rows.push(row);
                return Line::default();
            }
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
    // Линия-разделитель занимает всю ширину рамки, а не только колонку текста.
    let rule_style = Style::new().fg(Color::Gray);
    for &row in &rule_rows {
        let y = area.y + row as u16;
        for x in area.x..area.x + rule_width.max(1) {
            let cell = &mut frame.buffer_mut()[(x, y)];
            cell.set_symbol("─");
            cell.set_style(rule_style);
        }
    }
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
    spans.extend(style_inline(&text, style));
    Line::from(spans)
}

/// Разметка усиления `**жирный**`/`__жирный__` и `*курсив*`/`_курсив_`:
/// маркеры съедаются, текст делится на спаны с базовым стилем. `_` считается
/// разделителем только на границах слов, чтобы не ломать `snake_case`.
fn style_inline(text: &str, base: Style) -> Vec<Span<'static>> {
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Mark {
        Bold,
        Italic,
    }
    fn styled(text: &str, base: Style, mark: Mark) -> Span<'static> {
        let modifier = match mark {
            Mark::Bold => Modifier::BOLD,
            Mark::Italic => Modifier::ITALIC,
        };
        Span::styled(text.to_owned(), base.add_modifier(modifier))
    }
    fn word_boundary(prev: Option<char>, next: Option<char>) -> bool {
        let prev_word = prev.is_some_and(|c| c.is_alphanumeric());
        let next_word = next.is_some_and(|c| c.is_alphanumeric());
        prev_word ^ next_word
    }
    let push_plain = |out: &mut Vec<Span<'static>>, plain: &mut String| {
        if !plain.is_empty() {
            out.push(Span::styled(plain.clone(), base));
            plain.clear();
        }
    };
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut plain = String::new();
    let mut open: Option<(Mark, String)> = None;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let prev = (i > 0).then(|| chars[i - 1]);
        let next = chars.get(i + 1).copied();
        let delim = match c {
            '*' if next == Some('*') => Some((Mark::Bold, 2)),
            '_' if next == Some('_') && word_boundary(prev, chars.get(i + 2).copied()) => {
                Some((Mark::Bold, 2))
            }
            '*' => Some((Mark::Italic, 1)),
            '_' if word_boundary(prev, next) => Some((Mark::Italic, 1)),
            _ => None,
        };
        if let Some((mark, width)) = delim {
            match open.take() {
                Some((open_mark, buf)) => {
                    if !buf.is_empty() {
                        out.push(styled(&buf, base, open_mark));
                    }
                }
                None => {
                    push_plain(&mut out, &mut plain);
                    open = Some((mark, String::new()));
                }
            }
            i += width;
        } else {
            if let Some((_, buf)) = open.as_mut() {
                buf.push(c);
            } else {
                plain.push(c);
            }
            i += 1;
        }
    }
    if let Some((_, buf)) = open
        && !buf.is_empty()
    {
        out.push(Span::styled(buf, base));
    }
    push_plain(&mut out, &mut plain);
    if out.is_empty() { vec![Span::styled(text.to_owned(), base)] } else { out }
}

/// Шапка книги: строка над всеми блоками с названием, языками, прогрессом,
/// качеством и меткой заметки на текущем блоке.
fn header_line(app: &App) -> Line<'static> {
    let total = app.document().len();
    let block = app.anchor().block + 1;
    let mut spans =
        vec![Span::styled(app.title().to_owned(), Style::new().add_modifier(Modifier::BOLD))];
    if let Some(coverage) = app.coverage().filter(|value| *value < QUALITY_WARN) {
        spans.push(Span::styled(
            format!(" · ⚠{:.0}%", coverage * 100.0),
            Style::new().fg(Color::Yellow),
        ));
    }
    spans.push(Span::raw(format!(" · {block}/{total}")));
    if let Some(bookmark) = app.bookmark_at() {
        spans.push(Span::styled(
            format!(" · {}", bookmark.label),
            Style::new().fg(note_color(bookmark.color)),
        ));
    }
    Line::from(spans)
}

/// Заголовок рамки текста: язык и процент прочтения вместо слова «Текст».
/// Бар прогресса занимает 24 ячейки, но не шире рамки, чтобы процент не
/// обрезался при правом выравнивании титула (офсет рамки +2).
fn text_title(app: &App, width: u16) -> Line<'static> {
    let percent = app.percent();
    let lang_txt = format!("{} ", langs(app));
    let pct_txt = format!(" {percent:.0}%");
    let fixed = 2 + lang_txt.chars().count() + pct_txt.chars().count();
    let cells = (width as usize).saturating_sub(fixed + 3).clamp(1, FRAME_PROGRESS_CELLS);
    let mut spans = vec![Span::raw(lang_txt)];
    spans.extend(btop_gauge(percent, cells).spans);
    spans.push(Span::raw(pct_txt));
    Line::from(spans)
}

/// Название книги над рамками — «шапка» окна.
fn render_header(app: &App, frame: &mut Frame, x: u16, width: u16, y: u16) {
    let area = Rect { x, y, width, height: 1 };
    frame.render_widget(Paragraph::new(header_line(app)), area);
}

/// Рамка со скруглёнными углами и подписанным заголовком.
/// Неактивный блок приглушается, активный остаётся ярким. В начале заголовка —
/// цифра панели (btop): активная жёлтая, скрытые панели не рисуются вовсе.
fn block_frame(digit: &str, title: Line<'static>, active: bool) -> Block<'static> {
    let border = if active { Style::new() } else { Style::new().fg(Color::DarkGray) };
    let title_style = if active {
        Style::new().add_modifier(Modifier::BOLD)
    } else {
        Style::new().fg(Color::DarkGray).add_modifier(Modifier::DIM)
    };
    let mut spans = Vec::new();
    if !digit.is_empty() {
        spans.push(Span::styled(digit.to_string(), digit_style(active)));
        spans.push(Span::raw(" "));
    }
    for span in title.spans {
        spans.push(Span::styled(span.content, span.style.patch(title_style)));
    }
    Block::bordered().border_type(BorderType::Rounded).border_style(border).title(Line::from(spans))
}

/// Стиль цифры панели на рамке: активная белая, неактивные оранжевые.
fn digit_style(active: bool) -> Style {
    if active {
        Style::new().fg(Color::White).add_modifier(Modifier::BOLD)
    } else {
        Style::new().fg(Color::Yellow)
    }
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
        let current = app.active_heading().unwrap_or(0);
        let anchor = if active { nav } else { current };
        let rows = inner.height as usize;
        let start = anchor.saturating_sub(rows.saturating_sub(1));
        for (index, item) in items.iter().enumerate().skip(start).take(rows) {
            let selected = index == nav;
            let is_active = index == current;
            let marker = if selected { "►" } else { " " };
            let title_style = if is_active {
                Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD)
            } else if active {
                Style::new()
            } else {
                Style::new().fg(Color::DarkGray)
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
    frame.render_widget(block_frame("2", Line::raw(" Главы "), active), area);
}

/// Правая колонка: список команд и заметок. Когда видны оба блока, команды
/// снизу, над ними заметки; при нехватке места команды прячутся сами
/// (минимум BOOKMARKS_MIN строк заметок) и даже в фокусе не съедают колонку.
/// Если какой-то блок скрыт — другой занимает колонку целиком.
fn render_right(app: &App, frame: &mut Frame, area: Rect) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let focused = app.focus() == ReaderFocus::Commands;
    let show_bookmarks = app.bookmarks_visible();
    let show_commands = app.commands_visible();

    if show_commands && show_bookmarks {
        if focused || area.height >= BOOKMARKS_MIN + 3 {
            let half = area.height / 2;
            let cmds_height = (COMMANDS.len().min(COMMANDS_MAX_ROWS) as u16 + 2).min(half.max(3));
            let cmds_area = Rect {
                x: area.x,
                y: area.y + area.height - cmds_height,
                width: area.width,
                height: cmds_height,
            };
            render_commands(app, frame, cmds_area, focused);
            let above = Rect { y: area.y, height: area.height - cmds_height, ..area };
            render_bookmarks(app, frame, above, app.focus() == ReaderFocus::Bookmarks);
        } else {
            render_bookmarks(app, frame, area, app.focus() == ReaderFocus::Bookmarks);
        }
    } else if show_commands {
        render_commands(app, frame, area, focused);
    } else if show_bookmarks {
        render_bookmarks(app, frame, area, app.focus() == ReaderFocus::Bookmarks);
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
        let marker = if selected { "► " } else { "  " };
        let style = if selected {
            Style::new().add_modifier(Modifier::BOLD)
        } else if active {
            Style::new()
        } else {
            Style::new().fg(Color::DarkGray)
        };
        lines.push(Line::from(vec![
            Span::styled(marker, Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            Span::styled("● ", Style::new().fg(note_color(bookmark.color))),
            Span::styled(bookmark.label.clone(), style),
            Span::styled(format!(" · {}", format_date(bookmark.created_at)), style),
        ]));
    }
    frame.render_widget(Paragraph::new(lines), inner);
    frame.render_widget(block_frame("3", Line::raw(" Заметки "), active), area);
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
            Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD)
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
    frame.render_widget(block_frame("4", Line::raw(" Команды "), active), area);
}

/// Слот над баром: командная строка `:`, ввод метки заметки или тост.
fn render_slot(app: &App, frame: &mut Frame, y: u16, x: u16, width: u16) {
    let slot = Rect { x, y, width, height: 1 };
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

/// Клавиатурный бар btop в нижней строке: подсказка цифровых панелей и
/// клавиши текущего блока. Сами цифры живут в рамках колонок.
fn render_bar(app: &App, frame: &mut Frame, x: u16, width: u16, y: u16) {
    let bar_area = Rect { x, y, width, height: 1 };
    let wide = width >= BAR_WIDE_AT;
    let mut spans = vec![Span::styled(
        "(shift+№ для скрытия) 1–4 — блоки · ",
        Style::new().add_modifier(Modifier::DIM),
    )];
    spans.extend(bar_hint(app, wide).spans);
    frame.render_widget(Paragraph::new(Line::from(spans)), bar_area);
}

/// Оверлей справки по центру окна: одна фича на строку, рамка и метрики
/// шапки остаются видимыми.
fn render_help(frame: &mut Frame, area: Rect) {
    let shown = (area.height - 2) as usize;
    let lines: Vec<Line> = HELP.iter().take(shown).map(|t| Line::from(*t)).collect();
    let inner = Rect {
        x: area.x + 1,
        y: area.y + 1,
        width: area.width.saturating_sub(2),
        height: area.height - 2,
    };
    // Сперва полностью стираем модальную область: иначе сквозь справку
    // просвечивают символы текста, оставшиеся в её пустых ячейках.
    frame.render_widget(Clear, area);
    frame.render_widget(Paragraph::new(lines), inner);
    frame.render_widget(block_frame("", Line::raw(" Справка "), true), area);
}

const HELP: &[&str] = &[
    "j/k, Space, PgUp/PgDn, g/G — прокрутка",
    "цифра 1–4 — фокус блока",
    "Shift+цифра — показать/скрыть колонку",
    "2/o — оглавление",
    "3/B — заметки",
    "4 — команды",
    "b — закладка на строке",
    "c/C — цвет закладки, D — удалить",
    "n/p — переход по закладкам",
    "[ — уже колонку, ] — шире",
    ": open <путь> — открыть книгу",
    ": lang t|<код> — язык, : goto <N> — переход",
    "s — полка",
    "h — справка",
    "Esc — к тексту · q — выход",
];

fn compact_status(app: &App, width: u16) -> Line<'static> {
    let total = app.document().len();
    let block = app.anchor().block + 1;
    let percent = app.percent();
    let hint = bar_hint(app, width >= BAR_WIDE_AT);
    let mut spans = vec![
        Span::raw(format!(" {} · ", app.title())),
        Span::styled(langs(app), Style::new().add_modifier(Modifier::BOLD)),
        Span::raw(format!(" · {percent:.1}% · блок {block}/{total}")),
    ];
    spans.extend(hint.spans);
    Line::from(spans)
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
