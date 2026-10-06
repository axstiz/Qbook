//! Рендер читалки: текст по раскладке с оформлением блоков, статус-бар
//! и скроллбар.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState};

use crate::app::App;
use crate::model::{BlockKind, LineInfo};
use crate::parse::txt::{list_marker, strip_heading};

/// Ниже этого покрытия статус-бар подсвечивает качество выравнивания.
const QUALITY_WARN: f32 = 0.9;
/// Полная подсказка нуждается в ширине; узкий терминал получает только неё.
const HINT_WIDE: &str = " · j/k прокрутка · L язык · q выход";
const HINT_NARROW: &str = " j/k L q";
const HINT_WIDE_AT: u16 = 70;

pub fn render(app: &App, frame: &mut Frame) {
    let area = frame.area();
    if area.width == 0 || area.height == 0 {
        return;
    }
    let text_area = Rect { height: area.height.saturating_sub(1), ..area };
    if text_area.height > 0 {
        render_text(app, frame, text_area);
    }
    let status_area = Rect { x: area.x, y: area.y + area.height - 1, width: area.width, height: 1 };
    frame.render_widget(Paragraph::new(status_line(app, area.width)), status_area);
}

fn render_text(app: &App, frame: &mut Frame, area: Rect) {
    let lines: Vec<Line> = app
        .layout()
        .lines()
        .iter()
        .skip(app.scroll())
        .take(area.height as usize)
        .map(|info| text_line(app, info))
        .collect();
    frame.render_widget(Paragraph::new(lines), area);
    if app.layout().len() > area.height as usize {
        let mut state = ScrollbarState::new(app.max_scroll()).position(app.scroll());
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight),
            area,
            &mut state,
        );
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
    Line::from(vec![Span::styled(prefix, style), Span::styled(text, style)])
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

fn status_line(app: &App, width: u16) -> Line<'static> {
    let total = app.document().len();
    let block = app.anchor().block + 1;
    let max = app.max_scroll();
    let percent = if max == 0 { 100.0 } else { app.scroll() as f32 / max as f32 * 100.0 };
    let mut spans = vec![
        Span::raw(format!(" {} · ", app.title())),
        Span::styled(langs(app), Style::new().add_modifier(Modifier::BOLD)),
        Span::raw(format!(" · {percent:.1}% · блок {block}/{total}")),
    ];
    if let Some(coverage) = app.coverage().filter(|value| *value < QUALITY_WARN) {
        spans.push(Span::styled(
            format!(" · ⚠ {:.0}%", coverage * 100.0),
            Style::new().fg(Color::Yellow),
        ));
    }
    let hint = if width >= HINT_WIDE_AT { HINT_WIDE } else { HINT_NARROW };
    spans.push(Span::styled(hint, Style::new().add_modifier(Modifier::DIM)));
    Line::from(spans)
}

fn langs(app: &App) -> String {
    let base = app.base_lang();
    let current = app.current_lang();
    if base == current { base.to_owned() } else { format!("{base}▸{current}") }
}
