use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use qbook::app::{App, Screen};
use qbook::model::Anchor;
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

fn store_of(tmp: &TempDir) -> Store {
    Store::open(tmp.path().join("qbook.db")).expect("хранилище")
}

/// Две книги в хранилище с реальными файлами; у первой фиксированная дата.
fn seeded(tmp: &TempDir, store: &Store) -> (PathBuf, PathBuf) {
    let a = tmp.path().join("a.md");
    let b = tmp.path().join("b.md");
    write(&a, "первая книга");
    write(&b, "вторая книга");
    let a_id = store
        .add_book(&a.display().to_string(), "Первая", "en", 1_700_000_000, 100)
        .expect("книга");
    store.set_progress(a_id, Anchor::START, "en", 42.5).expect("прогресс");
    store.add_book(&b.display().to_string(), "Вторая", "ru", 1_700_100_000, 200).expect("книга");
    (a, b)
}

fn type_text(app: &mut App, text: &str) {
    for c in text.chars() {
        app.handle_key(key(KeyCode::Char(c)));
    }
}

#[test]
fn shelf_lists_books_with_title_langs_percent_and_date() {
    let tmp = dir();
    let store = store_of(&tmp);
    let a = store.add_book("/books/a.md", "Война и мир", "en", 1_700_000_000, 100).expect("книга");
    store.set_variant(a, "ru", "/books/a.ru.md", "md").expect("вариант");
    store.set_progress(a, Anchor::START, "en", 42.5).expect("прогресс");

    let app = App::shelf(Some(store), "en").expect("полка");
    assert_eq!(app.screen(), Screen::Shelf);
    let books = app.shelf_books();
    assert_eq!(books.len(), 1);
    assert_eq!(books[0].title, "Война и мир");
    assert_eq!(books[0].langs, ["en", "ru"]);
    assert!((books[0].percent - 42.5).abs() < 1e-6);
    assert_eq!(books[0].date, 1_700_000_000, "дата файла, пока книгу не открывали");
}

#[test]
fn shelf_cursor_walks_and_clamps() {
    let tmp = dir();
    let store = store_of(&tmp);
    seeded(&tmp, &store);
    let mut app = App::shelf(Some(store), "en").expect("полка");

    assert_eq!(app.shelf_cursor(), 0);
    app.handle_key(key(KeyCode::Char('k')));
    assert_eq!(app.shelf_cursor(), 0, "вверх на верхней строке");
    app.handle_key(key(KeyCode::Char('j')));
    assert_eq!(app.shelf_cursor(), 1);
    app.handle_key(key(KeyCode::Down));
    assert_eq!(app.shelf_cursor(), 1, "вниз на нижней строке");
    app.handle_key(key(KeyCode::Char('k')));
    assert_eq!(app.shelf_cursor(), 0);
    assert!(!app.should_quit(), "j/k не завершают приложение");
}

#[test]
fn shelf_enter_opens_the_selected_book() {
    let tmp = dir();
    let store = store_of(&tmp);
    seeded(&tmp, &store);
    let mut app = App::shelf(Some(store), "en").expect("полка");

    app.handle_key(key(KeyCode::Char('j')));
    app.handle_key(key(KeyCode::Enter));
    assert_eq!(app.screen(), Screen::Reader);
    assert_eq!(app.title(), "b", "открыт файл книги под курсором");
    assert!(app.document().blocks()[0].text.contains("вторая книга"), "содержимое второй книги");
}

#[test]
fn shelf_quits_on_q() {
    let tmp = dir();
    let mut app = App::shelf(Some(store_of(&tmp)), "en").expect("полка");
    app.handle_key(key(KeyCode::Char('q')));
    assert!(app.should_quit());
}

#[test]
fn prompt_adds_a_book_and_keeps_the_shelf() {
    let tmp = dir();
    let book = tmp.path().join("new.md");
    write(&book, "свежая книга");
    let store = store_of(&tmp);

    let mut app = App::shelf(Some(store), "en").expect("полка");
    app.handle_key(key(KeyCode::Char('a')));
    assert!(app.typing_buffer().is_some(), "prompt открыт");

    type_text(&mut app, &book.display().to_string());
    app.handle_key(key(KeyCode::Enter));
    assert_eq!(app.screen(), Screen::Shelf, "книга добавлена без открытия");
    assert!(app.shelf_error().is_none(), "ошибки нет");
    assert!(app.typing_buffer().is_none(), "prompt закрыт");
    let books = app.shelf_books();
    assert_eq!(books.len(), 1);
    assert_eq!(books[0].title, "new", "заголовок взят из файла");
    assert_eq!(books[0].path, book.display().to_string());
}

#[test]
fn prompt_reports_a_missing_file_and_keeps_typing() {
    let tmp = dir();
    let store = store_of(&tmp);
    let mut app = App::shelf(Some(store), "en").expect("полка");

    app.handle_key(key(KeyCode::Char('a')));
    type_text(&mut app, "/нет/такого.md");
    app.handle_key(key(KeyCode::Enter));
    assert!(app.shelf_error().is_some(), "ошибка показана");
    assert!(app.typing_buffer().is_some(), "буфер сохранён для исправления");
    assert!(app.shelf_books().is_empty());

    app.handle_key(key(KeyCode::Esc));
    assert!(app.typing_buffer().is_none());
    assert!(app.shelf_error().is_none());
}

#[test]
fn shelf_d_deletes_the_selected_book() {
    let tmp = dir();
    let store = store_of(&tmp);
    let (a, _b) = seeded(&tmp, &store);
    let mut app = App::shelf(Some(store), "en").expect("полка");

    app.handle_key(key(KeyCode::Char('d')));
    let books = app.shelf_books();
    assert_eq!(books.len(), 1);
    assert_eq!(books[0].title, "Вторая", "удалена книга под курсором");
    let store = app.store().expect("store");
    assert_eq!(store.book_id(&a.display().to_string()).expect("поиск"), None, "удалена из БД");
}

#[test]
fn shelf_d_on_an_empty_shelf_is_a_noop() {
    let tmp = dir();
    let mut app = App::shelf(Some(store_of(&tmp)), "en").expect("полка");
    app.handle_key(key(KeyCode::Char('d')));
    app.handle_key(key(KeyCode::Enter));
    assert_eq!(app.screen(), Screen::Shelf);
    assert!(!app.should_quit());
}
