use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use qbook::app::App;
use qbook::store::Store;
use tempfile::TempDir;

fn dir() -> TempDir {
    tempfile::tempdir().expect("каталог")
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn paragraphs(count: usize) -> String {
    (0..count).map(|i| format!("Абзац номер {i}")).collect::<Vec<_>>().join("\n\n")
}

/// Книга с `count` абзацами и открытым хранилищем.
fn loaded(count: usize) -> (TempDir, PathBuf, App) {
    let tmp = dir();
    let path = tmp.path().join("book.md");
    std::fs::write(&path, paragraphs(count)).expect("записать");
    let store = Store::open(tmp.path().join("qbook.db")).expect("хранилище");
    let app = App::load(&path, "en", &[], Some(store)).expect("загрузка");
    (tmp, path, app)
}

fn bookmark_labels(app: &App) -> Vec<String> {
    app.bookmarks().iter().map(|b| b.label.clone()).collect()
}

fn store_bookmark_labels(app: &App, path: &Path) -> Vec<String> {
    let store = app.store().expect("store");
    let id = store.book_id(&path.display().to_string()).expect("id").expect("есть");
    store.list_bookmarks(id).expect("список").iter().map(|b| b.label.clone()).collect()
}

#[test]
fn b_places_a_bookmark_with_a_text_label() {
    let (_tmp, path, mut app) = loaded(20);
    app.set_size(40, 10);
    app.set_scroll(0);
    app.handle_key(key(KeyCode::Char('b')));

    assert_eq!(app.bookmarks().len(), 1, "закладка в памяти");
    assert_eq!(app.bookmarks()[0].label, "Абзац номер 0", "метка — фрагмент абзаца");
    assert_eq!(app.bookmarks()[0].anchor.block, 0);

    let labels = store_bookmark_labels(&app, &path);
    assert_eq!(labels, vec!["Абзац номер 0".to_owned()], "закладка в БД");
}

#[test]
fn bookmarks_without_a_store_are_noops() {
    let tmp = dir();
    let path = tmp.path().join("book.md");
    std::fs::write(&path, paragraphs(10)).expect("файл");
    let mut app = App::load(&path, "en", &[], None).expect("загрузка");

    app.handle_key(key(KeyCode::Char('b')));
    app.handle_key(key(KeyCode::Char('n')));
    app.handle_key(key(KeyCode::Char('p')));
    app.handle_key(key(KeyCode::Char('D')));
    assert!(app.bookmarks().is_empty());
    assert!(!app.bookmarks_open(), "без store список не открывается сам");

    app.handle_key(key(KeyCode::Char('B')));
    assert!(app.bookmarks_open(), "панель открыта даже пустой");
    app.handle_key(key(KeyCode::Char('B')));
    assert!(!app.bookmarks_open(), "B переключает");
}

#[test]
fn panel_walks_the_cursor_and_closes_on_esc() {
    let (_tmp, _path, mut app) = loaded(60);
    app.set_size(40, 10);
    app.set_scroll(0);
    app.handle_key(key(KeyCode::Char('b')));
    app.set_scroll(app.max_scroll());
    app.handle_key(key(KeyCode::Char('b')));
    assert_eq!(app.bookmarks().len(), 2);

    app.set_scroll(0);
    app.handle_key(key(KeyCode::Char('B')));
    assert!(app.bookmarks_open());
    assert_eq!(app.bookmark_cursor(), 0);
    app.handle_key(key(KeyCode::Char('j')));
    assert_eq!(app.bookmark_cursor(), 1);
    app.handle_key(key(KeyCode::Char('k')));
    assert_eq!(app.bookmark_cursor(), 0);
    app.handle_key(key(KeyCode::Esc));
    assert!(!app.bookmarks_open());
    assert!(!app.should_quit(), "Esc закрывает только панель");
}

#[test]
fn panel_enter_jumps_to_the_selected_bookmark() {
    let (_tmp, _path, mut app) = loaded(60);
    app.set_size(40, 10);
    app.set_scroll(0);
    app.handle_key(key(KeyCode::Char('b')));
    app.set_scroll(app.max_scroll());
    app.handle_key(key(KeyCode::Char('b')));
    app.set_scroll(0);

    app.handle_key(key(KeyCode::Char('B')));
    app.handle_key(key(KeyCode::Char('j')));
    app.handle_key(key(KeyCode::Enter));
    assert!(!app.bookmarks_open(), "панель закрылась");
    assert!(app.scroll() > 0, "перешли к поздней закладке, scroll={}", app.scroll());
}

#[test]
fn n_and_p_jump_between_bookmarks() {
    let (_tmp, _path, mut app) = loaded(60);
    app.set_size(40, 10);
    app.set_scroll(0);
    app.handle_key(key(KeyCode::Char('b')));
    app.set_scroll(app.max_scroll());
    app.handle_key(key(KeyCode::Char('b')));
    app.set_scroll(0);

    app.handle_key(key(KeyCode::Char('n')));
    assert!(app.scroll() > 0, "n — к следующей закладке");
    app.handle_key(key(KeyCode::Char('p')));
    assert_eq!(app.scroll(), 0, "p — к предыдущей закладке");
    app.handle_key(key(KeyCode::Char('p')));
    assert_eq!(app.scroll(), 0, "раньше первой закладки — без изменений");
}

#[test]
fn r_renames_the_selected_bookmark() {
    let (_tmp, path, mut app) = loaded(20);
    app.set_size(40, 10);
    app.set_scroll(0);
    app.handle_key(key(KeyCode::Char('b')));
    app.handle_key(key(KeyCode::Char('B')));
    app.handle_key(key(KeyCode::Char('r')));
    assert!(app.typing_buffer().is_some(), "режим ввода метки");

    for c in "Моя метка".chars() {
        app.handle_key(key(KeyCode::Char(c)));
    }
    app.handle_key(key(KeyCode::Enter));
    assert!(app.typing_buffer().is_none());
    assert_eq!(bookmark_labels(&app), vec!["Моя метка".to_owned()], "в памяти");
    assert_eq!(store_bookmark_labels(&app, &path), vec!["Моя метка".to_owned()], "в БД");
}

#[test]
fn panel_d_deletes_the_selected_bookmark() {
    let (_tmp, path, mut app) = loaded(60);
    app.set_size(40, 10);
    app.set_scroll(0);
    app.handle_key(key(KeyCode::Char('b')));
    app.set_scroll(app.max_scroll());
    app.handle_key(key(KeyCode::Char('b')));
    assert_eq!(app.bookmarks().len(), 2);
    let before = store_bookmark_labels(&app, &path);

    app.set_scroll(0);
    app.handle_key(key(KeyCode::Char('B')));
    assert_eq!(app.bookmark_cursor(), 0, "курсор на ближайшей к началу");
    app.handle_key(key(KeyCode::Char('D')));
    assert_eq!(app.bookmarks().len(), 1, "удалена выбранная (первая)");
    assert_eq!(app.bookmark_cursor(), 0, "курсор сжат");
    assert_eq!(store_bookmark_labels(&app, &path), vec![before[1].clone()], "осталась вторая");
}

#[test]
fn b_updates_navigation_when_position_changes() {
    let (_tmp, _path, mut app) = loaded(60);
    app.set_size(40, 10);
    app.set_scroll(0);
    app.handle_key(key(KeyCode::Char('b')));
    app.set_scroll(30);
    app.handle_key(key(KeyCode::Char('b')));
    assert_eq!(app.bookmarks().len(), 2);
    let blocks: Vec<usize> = app.bookmarks().iter().map(|b| b.anchor.block).collect();
    let mut sorted = blocks.clone();
    sorted.sort_unstable();
    assert_eq!(blocks, sorted, "список отсортирован по позиции");
    assert!(blocks[1] > blocks[0]);
}
