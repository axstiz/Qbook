use qbook::ui::btop_gauge;
use ratatui::style::Color;

#[test]
fn gauge_fills_from_dark_purple_to_violet() {
    let line = btop_gauge(50.0, 4);
    let spans: Vec<_> = line.spans;
    assert_eq!(spans.len(), 4, "ширина бара");
    assert_eq!(spans[0].content.as_ref(), "▮");
    assert_eq!(spans[3].content.as_ref(), "▯");

    let fg = |span: &ratatui::text::Span| match span.style.fg {
        Some(Color::Rgb(r, g, b)) => (r, g, b),
        other => panic!("ожидался Rgb, получили {other:?}"),
    };
    let first = fg(&spans[0]);
    let second = fg(&spans[1]);
    assert!(first.0 < second.0, "фиолетовый растёт к сиреневому: {first:?} -> {second:?}");
    assert!(first.1 <= second.1, "зелёный растёт: {first:?} -> {second:?}");
    assert!(first.2 <= second.2, "синий растёт: {first:?} -> {second:?}");
    assert!(first.2 > first.0, "синий доминирует — это фиолетовый: {first:?}");
    assert_eq!(fg(&spans[2]), (50, 50, 50), "пустая часть тускло-серая");
}

#[test]
fn gauge_handles_edges() {
    let empty = btop_gauge(0.0, 3);
    assert!(empty.spans.iter().all(|s| s.content.as_ref() == "▯"));
    let full = btop_gauge(100.0, 3);
    assert!(full.spans.iter().all(|s| s.content.as_ref() == "▮"));
    let over = btop_gauge(150.0, 3);
    assert!(over.spans.iter().all(|s| s.content.as_ref() == "▮"), "переполнение зажато");
}
