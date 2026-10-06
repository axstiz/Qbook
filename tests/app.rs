use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use qbook::app::App;
use qbook::store::Store;
use tempfile::TempDir;

fn dir() -> TempDir {
    tempfile::tempdir().expect("каталог")
}

fn write(path: &Path, text: &str) {
    std::fs::write(path, text).expect("записать файл");
}

/// Книга из `count` абзацев, каждый короче строки, и перевод с тем же числом абзацев.
fn book_pair(dir: &Path, count: usize) -> PathBuf {
    let base = dir.join("book.md");
    let russian = dir.join("book.ru.md");
    let body = |lang: &str| {
        (0..count).map(|i| format!("{lang} абзац {i}")).collect::<Vec<_>>().join("\n\n")
    };
    write(&base, &body("base"));
    write(&russian, &body("перевод"));
    base
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn ctrl(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::CONTROL)
}

fn load(base: &Path, store: Option<Store>) -> App {
    App::load(base, "en", &[], store).expect("загрузка")
}

#[test]
fn loads_base_and_sorted_sidecar_languages() {
    let tmp = dir();
    let base = tmp.path().join("book.md");
    write(&base, "base text");
    write(&tmp.path().join("book.fr.md"), "texte");
    write(&tmp.path().join("book.ru.md"), "перевод");
    let app = load(&base, None);
    assert_eq!(app.languages(), ["en", "fr", "ru"]);
    assert_eq!(app.current_lang(), "en");
    assert_eq!(app.title(), "book");
}

#[test]
fn explicit_variant_beats_the_sidecar_with_the_same_language() {
    let tmp = dir();
    let base = tmp.path().join("book.md");
    write(&base, "base");
    write(&tmp.path().join("book.ru.md"), "сайдкар");
    write(&tmp.path().join("other.md"), "ЯВНЫЙ");
    let explicit = vec![("ru".to_owned(), tmp.path().join("other.md"))];
    let mut app = App::load(&base, "en", &explicit, None).expect("загрузка");
    assert_eq!(app.languages(), ["en", "ru"]);
    app.switch_lang(1);
    assert!(app.document().blocks()[0].text.contains("ЯВНЫЙ"));
}

#[test]
fn load_registers_the_book_in_the_store() {
    let tmp = dir();
    let base = book_pair(tmp.path(), 1);
    let store = Store::open_in(tmp.path().join("data")).expect("store");
    let app = load(&base, Some(store));
    let store = app.store().expect("store подключён");
    let books = store.list_books().expect("список");
    assert_eq!(books.len(), 1);
    assert_eq!(books[0].path, base.display().to_string());
    assert_eq!(books[0].base_lang, "en");
}

#[test]
fn navigation_keys_move_the_viewport() {
    let tmp = dir();
    let base = book_pair(tmp.path(), 60);
    let mut app = load(&base, None);
    app.set_size(40, 10);
    assert_eq!(app.viewport_height(), 9);

    app.handle_key(key(KeyCode::Char('j')));
    assert_eq!(app.scroll(), 1);
    app.handle_key(key(KeyCode::Down));
    assert_eq!(app.scroll(), 2);
    app.handle_key(key(KeyCode::Up));
    assert_eq!(app.scroll(), 1);
    app.handle_key(key(KeyCode::Char('k')));
    assert_eq!(app.scroll(), 0);
    app.handle_key(key(KeyCode::Char('k')));
    assert_eq!(app.scroll(), 0, "верх документа зажат");

    app.handle_key(key(KeyCode::Char(' ')));
    assert_eq!(app.scroll(), 9);
    app.handle_key(key(KeyCode::PageDown));
    assert_eq!(app.scroll(), 18);
    app.handle_key(key(KeyCode::PageUp));
    assert_eq!(app.scroll(), 9);

    app.handle_key(ctrl(KeyCode::Char('d')));
    assert_eq!(app.scroll(), 13);
    app.handle_key(ctrl(KeyCode::Char('u')));
    assert_eq!(app.scroll(), 9);

    app.handle_key(key(KeyCode::Char('G')));
    assert_eq!(app.scroll(), 51, "низ документа зажат");
    app.handle_key(key(KeyCode::Char('g')));
    assert_eq!(app.scroll(), 0);

    app.set_scroll(51);
    app.handle_wheel(true);
    assert_eq!(app.scroll(), 48, "колесо вверх — три строки");
    app.handle_wheel(false);
    assert_eq!(app.scroll(), 51);
    app.handle_wheel(false);
    assert_eq!(app.scroll(), 51, "конец документа зажат");
}

#[test]
fn q_and_ctrl_c_request_exit() {
    let tmp = dir();
    let base = book_pair(tmp.path(), 3);
    let mut app = load(&base, None);
    assert!(!app.should_quit());
    app.handle_key(key(KeyCode::Char('q')));
    assert!(app.should_quit());

    let mut app = load(&base, None);
    app.handle_key(ctrl(KeyCode::Char('c')));
    assert!(app.should_quit());
}

#[test]
fn progress_survives_reopen_including_language_and_position() {
    let tmp = dir();
    let base = book_pair(tmp.path(), 40);
    let data = tmp.path().join("data");

    let mut app = load(&base, Some(Store::open_in(&data).expect("store")));
    app.set_size(40, 10);
    app.switch_lang(1);
    app.set_scroll(30);
    app.save_progress().expect("сохранение");
    drop(app);

    let mut app = load(&base, Some(Store::open_in(&data).expect("store")));
    assert_eq!(app.current_lang(), "ru", "запомненный вариант");
    app.set_size(40, 10);
    assert_eq!(app.scroll(), 30, "запомненная позиция");
    drop(app);

    let store = Store::open_in(&data).expect("store");
    let progress = store.get_progress(1).expect("прогресс").expect("записан");
    assert_eq!(progress.variant_lang, "ru");
}

#[test]
fn switching_language_keeps_the_block_in_view() {
    let tmp = dir();
    let base = book_pair(tmp.path(), 40);
    let mut app = load(&base, None);
    app.set_size(40, 20);
    app.set_scroll(25);

    let before = app.anchor();
    app.handle_key(key(KeyCode::Char('2')));
    assert_eq!(app.current_lang(), "ru");
    let after = app.anchor();
    assert_eq!(after.block.abs_diff(before.block), 0, "M1: |Δ block| = 0");

    app.handle_key(key(KeyCode::Char('1')));
    assert_eq!(app.current_lang(), "en");
    assert_eq!(app.anchor().block, before.block, "возврат на базу");
}

#[test]
fn language_switching_stays_under_fifty_milliseconds() {
    let tmp = dir();
    let base = book_pair(tmp.path(), 40);
    let mut app = load(&base, None);
    app.set_size(80, 24);

    let start = Instant::now();
    for _ in 0..20 {
        app.switch_lang(1);
        app.switch_lang(0);
    }
    let per_switch = start.elapsed() / 20;
    assert!(per_switch < Duration::from_millis(50), "M3: переключение заняло {per_switch:?}");
}

#[test]
fn l_cycles_languages_and_out_of_range_keys_are_ignored() {
    let tmp = dir();
    let base = book_pair(tmp.path(), 4);
    let mut app = load(&base, None);
    app.handle_key(key(KeyCode::Char('L')));
    assert_eq!(app.current_lang(), "ru");
    app.handle_key(key(KeyCode::Char('L')));
    assert_eq!(app.current_lang(), "en", "цикл по кругу");

    app.handle_key(key(KeyCode::Char('9')));
    assert_eq!(app.current_lang(), "en");
    assert!(!app.switch_lang(5));
    assert_eq!(app.current_lang(), "en");
}

#[test]
fn resize_keeps_the_block_in_view() {
    let tmp = dir();
    let base = book_pair(tmp.path(), 60);
    let mut app = load(&base, None);
    app.set_size(40, 20);
    app.set_scroll(30);
    let block = app.anchor().block;

    app.set_size(17, 20);
    assert_eq!(app.anchor().block, block, "ресайз не теряет абзац");
}
