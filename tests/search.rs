//! Поиск: `/`, режимы через Tab, навигация `n`/`N` и переходы к результатам.

use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use qbook::app::{App, InputPurpose, SearchMode};
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

fn loaded(count: usize) -> (TempDir, PathBuf, App) {
    let tmp = dir();
    let path = tmp.path().join("book.md");
    std::fs::write(&path, paragraphs(count)).expect("записать");
    let store = Store::open(tmp.path().join("qbook.db")).expect("хранилище");
    let app = App::load(&path, "en", &[], Some(store)).expect("загрузка");
    (tmp, path, app)
}

fn type_query(app: &mut App, text: &str) {
    for c in text.chars() {
        app.handle_key(key(KeyCode::Char(c)));
    }
    app.handle_key(key(KeyCode::Enter));
}

/// Индекс строки раскладки с текущим совпадением и её текст.
fn hit_line(app: &App) -> Option<(usize, String)> {
    let index = app.search_line()?;
    let info = app.layout().line(index)?;
    let block = app.document().block(info.block)?;
    Some((index, info.slice(block).trim().to_owned()))
}

#[test]
fn slash_opens_search_and_enter_jumps_to_the_first_match() {
    let (_tmp, _path, mut app) = loaded(40);
    app.set_size(50, 10);
    app.set_scroll(0);

    app.handle_key(key(KeyCode::Char('/')));
    assert_eq!(app.typing_purpose(), Some(InputPurpose::Search), "prompt поиска");
    assert!(app.search_active(), "поиск активен сразу после /");

    type_query(&mut app, "номер 33");
    assert_eq!(app.typing_purpose(), None, "Enter закрывает prompt");
    let (index, text) = hit_line(&app).expect("совпадение найдено");
    assert!(text.contains("номер 33"), "строка совпадения: {text}");
    assert!(index > 0, "прыжок вглубь документа, индекс {index}");
    assert!(app.scroll() > 0, "скролл сдвинулся к совпадению");
}

#[test]
fn search_starts_after_the_current_position_and_wraps_around() {
    let (_tmp, _path, mut app) = loaded(40);
    app.set_size(50, 10);
    // Уводим видимую область вглубь: первый результат — после текущей позиции.
    app.set_scroll(60);

    app.handle_key(key(KeyCode::Char('/')));
    type_query(&mut app, "номер");
    let (first, _) = hit_line(&app).expect("совпадение есть");
    assert!(first > 60, "первый результат после текущей позиции: {first} <= 60");

    // Переходим n до полного круга: один спад индексов — это wrap на hits[0],
    // затем индексы снова растут до исходной позиции.
    let mut visited = vec![first];
    for _ in 0..200 {
        app.handle_key(key(KeyCode::Char('n')));
        let (index, _) = hit_line(&app).expect("совпадение есть");
        if index == first {
            break;
        }
        visited.push(index);
    }
    let (wrapped, _) = hit_line(&app).expect("совпадение есть");
    assert_eq!(wrapped, first, "после полного круга n возвращает к первому");
    assert!(visited.len() > 1, "было больше одного перехода");
    let drops = visited.windows(2).filter(|w| w[0] >= w[1]).count();
    assert_eq!(drops, 1, "ровно один спад — wrap на начало списка");
}

#[test]
fn tab_cycles_search_modes() {
    let (_tmp, _path, mut app) = loaded(10);
    app.handle_key(key(KeyCode::Char('/')));
    assert_eq!(app.search_mode(), SearchMode::Text, "стартовый режим — текст");

    app.handle_key(key(KeyCode::Tab));
    assert_eq!(app.search_mode(), SearchMode::Bookmarks, "Tab → заметки");
    app.handle_key(key(KeyCode::Tab));
    assert_eq!(app.search_mode(), SearchMode::Toc, "Tab → главы");
    app.handle_key(key(KeyCode::Tab));
    assert_eq!(app.search_mode(), SearchMode::Text, "Tab по кругу");

    app.handle_key(key(KeyCode::Esc));
    assert_eq!(app.typing_purpose(), None, "Esc закрывает prompt");
    assert!(!app.search_active(), "Esc гасит поиск");
    assert!(app.search_line().is_none(), "результатов нет");
}

#[test]
fn bookmark_mode_finds_labels_and_jumps_to_them() {
    let (_tmp, _path, mut app) = loaded(30);
    app.set_size(50, 10);
    // Заметка на первом абзаце с готовой меткой.
    app.handle_key(key(KeyCode::Char('b')));
    app.handle_key(key(KeyCode::Enter));
    app.handle_key(key(KeyCode::Enter));
    assert_eq!(app.bookmarks().len(), 1, "заметка создана");

    app.handle_key(key(KeyCode::Char('G')));
    assert_eq!(app.anchor().block, 29, "ушли в конец книги");

    app.handle_key(key(KeyCode::Char('/')));
    app.handle_key(key(KeyCode::Tab));
    assert_eq!(app.search_mode(), SearchMode::Bookmarks);
    type_query(&mut app, "абзац номер 0");
    assert_eq!(app.anchor().block, 0, "прыжок к найденной заметке");
}

#[test]
fn toc_mode_finds_headings() {
    let tmp = dir();
    let base = tmp.path().join("toc.md");
    std::fs::write(
        &base,
        "# Раздел первый\n\nтекст\n\n# Раздел второй\n\nещё текст\n\n# Раздел третий\n\nконец",
    )
    .expect("записать");
    let mut app = App::load(&base, "en", &[], None).expect("загрузка");
    app.set_size(50, 10);

    app.handle_key(key(KeyCode::Char('/')));
    app.handle_key(key(KeyCode::Tab));
    app.handle_key(key(KeyCode::Tab));
    assert_eq!(app.search_mode(), SearchMode::Toc);
    type_query(&mut app, "третий");
    assert_eq!(app.anchor().block, 4, "прыжок к заголовку третьего раздела");
}

#[test]
fn empty_query_and_no_matches_show_toasts() {
    let (_tmp, _path, mut app) = loaded(5);
    app.handle_key(key(KeyCode::Char('/')));
    app.handle_key(key(KeyCode::Enter));
    assert!(app.notice().is_some_and(|n| n.contains("пустой")), "тост: {:?}", app.notice());

    app.handle_key(key(KeyCode::Char('/')));
    type_query(&mut app, "zzz");
    assert!(app.notice().is_some_and(|n| n.contains("не найдено")), "тост: {:?}", app.notice());
    assert!(app.search_active(), "режим поиска остаётся для нового запроса");
    assert!(app.search_line().is_none(), "совпадений нет");
}
