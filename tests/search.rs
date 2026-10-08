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

#[test]
fn live_filter_updates_results_while_typing() {
    let (_tmp, _path, mut app) = loaded(40);
    app.set_size(50, 10);
    app.set_scroll(0);

    app.handle_key(key(KeyCode::Char('/')));
    assert_eq!(app.search_count(), 0, "пустой запрос — пустой список");

    app.handle_key(key(KeyCode::Char('н')));
    assert_eq!(app.search_count(), 40, "по символу найдены все абзацы до Enter");
    assert!(
        app.search_row_label(app.search_cursor()).is_some_and(|l| l.contains('н')),
        "курсор на первом совпадении: {:?}",
        app.search_row_label(app.search_cursor())
    );

    // Список сужается при дораскладке, Enter не нужен.
    for c in "омер 3".chars() {
        app.handle_key(key(KeyCode::Char(c)));
    }
    assert!(app.search_count() < 40, "фильтр сузился: {}", app.search_count());
    assert!(app.search_row_label(app.search_cursor()).is_some_and(|l| l.contains("номер 3")));

    app.handle_key(key(KeyCode::Backspace));
    assert!(app.search_count() > 1, "Backspace возвращает совпадения");
}

#[test]
fn arrow_keys_walk_the_live_list_with_wrap() {
    let (_tmp, _path, mut app) = loaded(40);
    app.set_size(50, 10);
    app.set_scroll(0);

    app.handle_key(key(KeyCode::Char('/')));
    for c in "абзац".chars() {
        app.handle_key(key(KeyCode::Char(c)));
    }
    assert!(app.search_count() > 3, "несколько совпадений");

    let first = app.search_cursor();
    let scroll_before = app.scroll();
    app.handle_key(key(KeyCode::Down));
    assert_eq!(app.search_cursor(), first + 1, "↓ двигает курсор");
    assert!(app.scroll() != scroll_before, "живой прыжок: скролл изменился");

    app.handle_key(key(KeyCode::Up));
    assert_eq!(app.search_cursor(), first, "↑ возвращает курсор");

    // Проходим до начала списка и проверяем зацикливание назад.
    for _ in 0..first {
        app.handle_key(key(KeyCode::Up));
    }
    assert_eq!(app.search_cursor(), 0, "курсор в начале списка");
    app.handle_key(key(KeyCode::Up));
    assert_eq!(app.search_cursor(), app.search_count() - 1, "↑ зацикливается назад");

    app.handle_key(key(KeyCode::Down));
    assert_eq!(app.search_cursor(), 0, "↓ зацикливается вперёд");
}

#[test]
fn esc_closes_the_panel_then_the_search_in_two_steps() {
    let (_tmp, _path, mut app) = loaded(20);
    app.set_size(50, 10);

    app.handle_key(key(KeyCode::Char('/')));
    for c in "абзац".chars() {
        app.handle_key(key(KeyCode::Char(c)));
    }
    app.handle_key(key(KeyCode::Enter));
    assert_eq!(app.focus(), qbook::app::ReaderFocus::Text, "Enter — к тексту");
    assert!(app.search_active(), "панель поиска осталась");

    app.handle_key(key(KeyCode::Char('3')));
    assert_eq!(app.focus(), qbook::app::ReaderFocus::SearchResults, "цифра 3 — в панель");

    app.handle_key(key(KeyCode::Esc));
    assert_eq!(app.focus(), qbook::app::ReaderFocus::Text, "первый Esc — к тексту");
    assert!(app.search_active(), "панель ещё видна");

    app.handle_key(key(KeyCode::Esc));
    assert!(!app.search_active(), "второй Esc — поиск закрыт");
    assert_eq!(app.focus(), qbook::app::ReaderFocus::Text, "фокус у текста");
}

#[test]
fn digits_and_aliases_focus_the_results_panel_while_searching() {
    let (_tmp, _path, mut app) = loaded(10);
    app.handle_key(key(KeyCode::Char('/')));
    for c in "абзац".chars() {
        app.handle_key(key(KeyCode::Char(c)));
    }
    app.handle_key(key(KeyCode::Enter));

    app.handle_key(key(KeyCode::Char('4')));
    assert_eq!(app.focus(), qbook::app::ReaderFocus::SearchResults, "4 → результаты");
    app.handle_key(key(KeyCode::Char('1')));
    assert_eq!(app.focus(), qbook::app::ReaderFocus::Text, "1 → текст");
}

#[test]
fn panel_keys_step_the_cursor_and_enter_returns_to_text() {
    let (_tmp, _path, mut app) = loaded(20);
    app.set_size(50, 10);

    app.handle_key(key(KeyCode::Char('/')));
    for c in "абзац".chars() {
        app.handle_key(key(KeyCode::Char(c)));
    }
    app.handle_key(key(KeyCode::Enter));
    app.handle_key(key(KeyCode::Char('3')));
    assert_eq!(app.focus(), qbook::app::ReaderFocus::SearchResults);

    let cursor = app.search_cursor();
    app.handle_key(key(KeyCode::Char('j')));
    assert_eq!(app.search_cursor(), cursor + 1, "j шагает курсором списка");
    app.handle_key(key(KeyCode::Char('k')));
    assert_eq!(app.search_cursor(), cursor, "k возвращает");

    app.handle_key(key(KeyCode::Enter));
    assert_eq!(app.focus(), qbook::app::ReaderFocus::Text, "Enter — к тексту");
    assert!(app.search_active(), "панель остаётся");

    // n/N синхронны с курсором панели.
    app.handle_key(key(KeyCode::Char('n')));
    assert_eq!(app.search_cursor(), cursor + 1, "n двигает тот же курсор");
    app.handle_key(key(KeyCode::Char('N')));
    assert_eq!(app.search_cursor(), cursor, "N обратно");
}

#[test]
fn shift_digit_toggles_are_ignored_while_search_is_active() {
    let shift = |c: char| KeyEvent::new(KeyCode::Char(c), KeyModifiers::SHIFT);
    let (_tmp, _path, mut app) = loaded(20);
    app.set_size(110, 14);

    app.handle_key(key(KeyCode::Char('/')));
    for c in "абзац".chars() {
        app.handle_key(key(KeyCode::Char(c)));
    }
    app.handle_key(key(KeyCode::Enter));
    assert!(app.search_active(), "поиск активен после Enter");

    app.handle_key(shift('3'));
    assert!(app.bookmarks_visible(), "Shift+3 не скрывает заметки при поиске");
    app.handle_key(shift('4'));
    assert!(app.commands_visible(), "Shift+4 не скрывает команды при поиске");

    app.handle_key(key(KeyCode::Esc));
    assert!(!app.search_active(), "поиск закрыт");
    app.handle_key(shift('3'));
    assert!(!app.bookmarks_visible(), "после закрытия Shift+3 снова скрывает заметки");
    app.handle_key(shift('3'));
    assert!(app.bookmarks_visible(), "и возвращает их");
}
