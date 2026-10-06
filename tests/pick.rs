use std::path::Path;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use qbook::app::App;
use qbook::model::scroll_to_anchor;
use qbook::store::Store;
use tempfile::TempDir;

fn dir() -> TempDir {
    tempfile::tempdir().expect("каталог")
}

fn write(path: &Path, text: &str) {
    std::fs::write(path, text).expect("записать файл");
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn load(base: &Path, store: Option<Store>) -> App {
    App::load(base, "en", &[], store).expect("загрузка")
}

fn paragraphs(count: usize) -> String {
    (0..count).map(|i| format!("Абзац номер {i}")).collect::<Vec<_>>().join("\n\n")
}

#[test]
fn b_opens_row_picker_moving_within_visible_area() {
    let tmp = dir();
    let base = tmp.path().join("book.md");
    write(&base, &paragraphs(30));
    let store = Store::open(tmp.path().join("qbook.db")).expect("хранилище");
    let mut app = load(&base, Some(store));
    app.set_size(40, 12);
    app.set_scroll(4);
    let vp = app.viewport_height();

    app.handle_key(key(KeyCode::Char('b')));
    let start = app.pick_row().expect("курсор открыт");
    assert!((app.scroll()..=app.scroll() + vp - 1).contains(&start), "курсор в видимой области");

    app.handle_key(key(KeyCode::Down));
    app.handle_key(key(KeyCode::Down));
    let moved = app.pick_row().expect("курсор активен");
    assert_eq!(moved, start + 2, "стрелки двигают курсор");

    app.handle_key(key(KeyCode::Enter));
    assert!(app.pick_row().is_none(), "курсор закрылся");
    assert!(app.typing_buffer().is_some(), "открылся ввод метки");
    app.handle_key(key(KeyCode::Enter));
    assert_eq!(app.bookmarks().len(), 1, "заметка создана на выбранной строке");
    let expected = scroll_to_anchor(app.layout(), app.document(), moved).block;
    assert_eq!(app.bookmarks()[0].anchor.block, expected, "якорь на выбранной строке");
}

#[test]
fn pick_cursor_stays_within_the_visible_area() {
    let tmp = dir();
    let base = tmp.path().join("book.md");
    write(&base, &paragraphs(30));
    let store = Store::open(tmp.path().join("qbook.db")).expect("хранилище");
    let mut app = load(&base, Some(store));
    app.set_size(40, 12);
    app.set_scroll(6);

    app.handle_key(key(KeyCode::Char('b')));
    let bottom = app.scroll() + app.viewport_height() - 1;
    for _ in 0..50 {
        app.handle_key(key(KeyCode::Down));
        let row = app.pick_row().expect("курсор активен");
        assert!(row <= bottom, "курсор не уходит за видимый низ: {row} > {bottom}");
    }
}

#[test]
fn v_selects_and_y_copies_rows_to_clipboard() {
    let tmp = dir();
    let base = tmp.path().join("book.md");
    write(&base, &paragraphs(30));
    let mut app = load(&base, None);
    app.set_size(40, 12);
    app.set_scroll(0);

    app.handle_key(key(KeyCode::Char('v')));
    assert!(app.pick_row().is_some(), "выделение активно");
    assert_eq!(app.selection_range(), Some((0, 0)), "выделение началось с одной точки");

    app.handle_key(key(KeyCode::Char('j')));
    app.handle_key(key(KeyCode::Char('j')));
    let (top, bottom) = app.selection_range().expect("диапазон");
    assert_eq!(top, 0, "якорь на первой строке чтения");
    assert_eq!(bottom, 2, "курсор опустился на две строки");

    app.handle_key(key(KeyCode::Char('y')));
    assert!(app.pick_row().is_none(), "выделение снялось");
    let text = app.copied_text().expect("скопировано");
    assert!(text.contains("Абзац номер 0"), "первая строка: {text:?}");
    assert!(text.contains("Абзац номер 1"), "последняя строка: {text:?}");
    assert!(text.contains('\n'), "строки разделились: {text:?}");
    assert!(app.take_clipboard().is_some(), "текст отдан main для OSC 52");
    assert!(app.pick_hint().is_none(), "подсказка погасла вместе с курсором");
}

#[test]
fn held_v_reanchors_the_selection_at_the_cursor() {
    let tmp = dir();
    let base = tmp.path().join("book.md");
    write(&base, &paragraphs(30));
    let mut app = load(&base, None);
    app.set_size(40, 12);
    app.set_scroll(0);

    app.handle_key(key(KeyCode::Char('v')));
    for _ in 0..3 {
        app.handle_key(key(KeyCode::Down));
    }
    assert_eq!(app.selection_range(), Some((0, 3)), "растянули до курсора 3");

    app.handle_key(key(KeyCode::Char('v')));
    for _ in 0..2 {
        app.handle_key(key(KeyCode::Up));
    }
    let (top, bottom) = app.selection_range().expect("диапазон");
    assert_eq!((top, bottom), (1, 3), "v перезаякорил на 3, две стрелки вверх — 1..=3");

    app.handle_key(key(KeyCode::Char('v')));
    app.handle_key(key(KeyCode::Down));
    let (top, bottom) = app.selection_range().expect("диапазон");
    assert!((top..=bottom).contains(&1), "повторный v снова перезаякорил");
    assert!(bottom > top, "стрелки тянут от нового якоря");
}

#[test]
fn note_marker_sits_on_the_anchored_line_across_blocks() {
    let tmp = dir();
    let base = tmp.path().join("book.md");
    write(&base, &paragraphs(10));
    let store = Store::open(tmp.path().join("qbook.db")).expect("хранилище");
    let mut app = load(&base, Some(store));
    app.set_size(40, 12);
    app.set_scroll(0);

    // Выбираем строку второго абзаца курсором `b`.
    app.handle_key(key(KeyCode::Char('b')));
    for _ in 0..2 {
        app.handle_key(key(KeyCode::Down));
    }
    app.handle_key(key(KeyCode::Enter));
    app.handle_key(key(KeyCode::Enter));
    assert_eq!(app.bookmarks().len(), 1);

    let anchored = app.bookmarks()[0].anchor;
    assert!(anchored.frac > 0.0 || anchored.block > 0, "якорь не на первой строке");
    let colored: Vec<usize> = (0..app.layout().len())
        .filter(|&row| {
            app.layout().line(row).is_some_and(|info| app.note_line_color(info).is_some())
        })
        .collect();
    assert_eq!(colored.len(), 1, "подсвечена ровно одна строка: {colored:?}");
}
