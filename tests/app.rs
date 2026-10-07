use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use qbook::app::{App, InputPurpose};
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
fn load_auto_detects_language_from_content() {
    let tmp = dir();
    let russian = tmp.path().join("rho_index.md");
    write(&russian, "Это русский роман: глава первая, повествование кириллицей.");
    let app = App::load_auto(&russian, &[], None).expect("загрузка");
    assert_eq!(app.base_lang(), "ru", "кириллица распознаётся");
    assert_eq!(app.languages(), ["ru"]);

    let english = tmp.path().join("chapter.txt");
    write(&english, "This English novel opens with a long narrative in latin letters.");
    let app = App::load_auto(&english, &[], None).expect("загрузка");
    assert_eq!(app.base_lang(), "en");
}

#[test]
fn load_auto_prefers_the_filename_suffix_over_the_content() {
    let tmp = dir();
    let file = tmp.path().join("book.ru.txt");
    write(&file, "English words but the file name says it is Russian.");
    let app = App::load_auto(&file, &[], None).expect("загрузка");
    assert_eq!(app.base_lang(), "ru");
    assert_eq!(app.languages(), ["ru"]);
}

#[test]
fn load_auto_fixes_a_stale_base_lang_in_the_store() {
    let tmp = dir();
    let base = tmp.path().join("story.md");
    let key = base.display().to_string();
    write(&base, "Русская история с длинным кириллическим текстом.");

    let data = tmp.path().join("data");
    let store = Store::open_in(&data).expect("store");
    store.add_book(&key, "История", "en", 1, 2).expect("add");

    App::load_auto(&base, &[], Some(store)).expect("загрузка");

    let read = Store::open_in(&data).expect("store после");
    let book = read.book_id(&key).expect("id").and_then(|id| read.get_book(id).ok()).flatten();
    assert_eq!(book.map(|b| b.base_lang).as_deref(), Some("ru"), "полка видит уточнённый язык");

    let app = App::load_auto(&base, &[], None).expect("переоткрыть");
    assert_eq!(app.current_lang(), "ru");
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
fn several_sidecars_of_one_language_become_a_single_variant_with_the_base_extension() {
    let tmp = dir();
    let base = tmp.path().join("book.md");
    write(&base, "# Заголовок\n\nбазовый текст\n\nповтор");
    write(&tmp.path().join("book.ru.md"), "перевод в md");
    write(&tmp.path().join("book.ru.txt"), "перевод в txt");
    let mut app = load(&base, None);
    assert_eq!(app.languages(), ["en", "ru"], "один вариант на язык");

    app.switch_lang(1);
    assert!(
        app.document().blocks().iter().any(|b| b.text.contains("перевод в md")),
        "сайдкар с расширением исходника побеждает: {:?}",
        app.document().blocks().iter().map(|b| b.text.as_str()).collect::<Vec<_>>()
    );

    let mut app = load(&base, None);
    for expected in ["ru", "en", "ru"] {
        app.handle_key(key(KeyCode::Char('t')));
        assert_eq!(app.current_lang(), expected, "клавиша t по кругу без дублей");
    }
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
    assert_eq!(app.viewport_height(), 6);

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
    assert_eq!(app.scroll(), 6);
    app.handle_key(key(KeyCode::PageDown));
    assert_eq!(app.scroll(), 12);
    app.handle_key(key(KeyCode::PageUp));
    assert_eq!(app.scroll(), 6);

    app.handle_key(ctrl(KeyCode::Char('d')));
    assert_eq!(app.scroll(), 9);
    app.handle_key(ctrl(KeyCode::Char('u')));
    assert_eq!(app.scroll(), 6);

    app.handle_key(key(KeyCode::Char('G')));
    // 60 абзацев = 119 строк, видимых 6 — плюс 13 пустых строк хвоста.
    assert_eq!(app.scroll(), 113 + 13, "низ документа докручивается до хвоста");
    app.handle_key(key(KeyCode::Char('g')));
    assert_eq!(app.scroll(), 0);

    let end = app.max_scroll();
    app.set_scroll(end);
    app.handle_wheel(true);
    assert_eq!(app.scroll(), end - 3, "колесо вверх — три строки");
    app.handle_wheel(false);
    assert_eq!(app.scroll(), end);
    app.handle_wheel(false);
    assert_eq!(app.scroll(), end, "конец документа зажат");
}

#[test]
fn wrap_width_matches_the_column_layout_across_sizes() {
    for width in [60, 70, 78, 80, 96, 120] {
        let tmp = dir();
        let base = book_pair(tmp.path(), 40);
        let mut app = load(&base, None);
        app.set_size(width, 20);
        assert!(
            app.wrap_width() >= 20,
            "перенос не обжимается у левого края при ширине {width}: {}",
            app.wrap_width()
        );
    }
}

#[test]
fn scroll_tail_keeps_percent_at_the_end() {
    let tmp = dir();
    let base = book_pair(tmp.path(), 30);
    let mut app = load(&base, None);
    app.set_size(40, 10);

    assert_eq!(app.max_scroll() - app.content_max_scroll(), 13, "там 13 пустых строк");
    let content_end = app.content_max_scroll();
    app.set_scroll(content_end);
    let percent_at_end = app.percent();
    assert_eq!(percent_at_end, 100.0, "прогресс на конце содержимого");
    app.set_scroll(app.max_scroll());
    assert_eq!(app.percent(), percent_at_end, "хвост не влияет на проценты");
    assert_eq!(
        app.anchor().block,
        app.document().len() - 1,
        "даже в хвосте якорь на последнем блоке"
    );
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
    app.handle_key(key(KeyCode::Char('t')));
    assert_eq!(app.current_lang(), "ru");
    let after = app.anchor();
    assert_eq!(after.block.abs_diff(before.block), 0, "M1: |Δ block| = 0");

    app.handle_key(key(KeyCode::Char('t')));
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
fn m2_opens_three_thousand_blocks_under_half_a_second() {
    let tmp = dir();
    let base = tmp.path().join("big.md");
    let translation = tmp.path().join("big.ru.md");
    let body = |prefix: &str| {
        (0..3000).map(|i| format!("{prefix} абзац {i}")).collect::<Vec<_>>().join("\n\n")
    };
    write(&base, &body("base"));
    write(&translation, &body("перевод"));

    let start = Instant::now();
    let app = App::load(&base, "en", &[], None).expect("загрузка");
    let elapsed = start.elapsed();
    assert_eq!(app.document().len(), 3000, "все абзацы на месте");
    assert!(elapsed < Duration::from_millis(500), "M2: открытие заняло {elapsed:?}");
}

/// Книга с тремя заголовками: блоки 0 (H1), 2 (H2), 4 (H1) между абзацами,
/// и перевод с той же структурой.
fn toc_book(dir: &Path) -> PathBuf {
    let base = dir.join("toc.md");
    let text = |suffix: &str| {
        format!(
            "# Раздел 1\n\nпервый абзац{suffix}\n\n## Раздел 2\n\nвторой абзац{suffix}\n\n\
             # Раздел 3\n\nтретий абзац{suffix}"
        )
    };
    write(&base, &text(""));
    write(&dir.join("toc.ru.md"), &text(" (перевод)"));
    base
}

#[test]
fn o_focuses_toc_with_cursor_on_current_heading_and_enter_jumps() {
    let tmp = dir();
    let base = toc_book(tmp.path());
    let mut app = load(&base, None);
    app.set_size(60, 12);
    assert_eq!(app.document().toc().len(), 3, "три заголовка");
    assert_eq!(app.focus(), qbook::app::ReaderFocus::Text);

    app.handle_key(key(KeyCode::Char('o')));
    assert_eq!(app.focus(), qbook::app::ReaderFocus::Toc, "фокус на колонке глав");
    assert_eq!(app.toc_cursor(), 0, "курсор на текущем разделе");

    app.handle_key(key(KeyCode::Char('j')));
    assert_eq!(app.toc_cursor(), 1, "курсор вниз");
    app.handle_key(key(KeyCode::Char('k')));
    assert_eq!(app.toc_cursor(), 0, "курсор вверх");
    app.handle_key(key(KeyCode::Char('k')));
    assert_eq!(app.toc_cursor(), 0, "верх списка зажат");

    app.handle_key(key(KeyCode::Enter));
    assert_eq!(app.focus(), qbook::app::ReaderFocus::Text, "Enter возвращает к тексту");
    assert_eq!(app.anchor().block, 0, "прыжок к первому заголовку");

    app.set_size(60, 2);
    app.set_scroll(app.max_scroll());
    app.handle_key(key(KeyCode::Char('o')));
    assert_eq!(app.toc_cursor(), 2, "курсор на текущем разделе внизу книги");
    app.handle_key(key(KeyCode::Enter));
    assert_eq!(app.anchor().block, 4, "прыжок к третьему разделу");

    app.handle_key(key(KeyCode::Char('o')));
    app.handle_key(key(KeyCode::Esc));
    assert_eq!(app.focus(), qbook::app::ReaderFocus::Text, "Esc возвращает к тексту");
}

#[test]
fn toc_jumps_to_the_same_block_after_switching_language() {
    let tmp = dir();
    let base = toc_book(tmp.path());
    let mut app = load(&base, None);
    app.set_size(60, 12);
    assert!(app.switch_lang(1), "перевод подключён");

    app.handle_key(key(KeyCode::Char('o')));
    app.handle_key(key(KeyCode::Char('j')));
    app.handle_key(key(KeyCode::Enter));
    assert_eq!(app.anchor().block, 2, "заголовок второго раздела на переводе");
}

#[test]
fn toc_over_empty_document_is_safe() {
    let tmp = dir();
    let base = tmp.path().join("plain.txt");
    write(&base, "строка один\nещё строка");
    let mut app = load(&base, None);
    assert!(app.document().toc().is_empty(), "заголовков нет");

    app.handle_key(key(KeyCode::Char('o')));
    assert_eq!(app.focus(), qbook::app::ReaderFocus::Toc, "пустая колонка тоже фокусируется");
    app.handle_key(key(KeyCode::Char('j')));
    app.handle_key(key(KeyCode::Enter));
    assert_eq!(app.focus(), qbook::app::ReaderFocus::Text);
    assert_eq!(app.scroll(), 0, "прыжка не было");
}

#[test]
fn help_toggles_with_question_mark_h_and_esc() {
    let tmp = dir();
    let base = book_pair(tmp.path(), 3);
    let mut app = load(&base, None);
    assert!(!app.help_open());

    app.handle_key(key(KeyCode::Char('?')));
    assert!(app.help_open(), "справка открыта");
    app.handle_key(key(KeyCode::Char('?')));
    assert!(!app.help_open(), "второй ? закрывает");
    app.handle_key(key(KeyCode::Char('h')));
    assert!(app.help_open(), "h — аналог ?");
    app.handle_key(key(KeyCode::Char('h')));
    assert!(!app.help_open(), "второй h закрывает справку");
    app.handle_key(key(KeyCode::Char('h')));
    app.handle_key(key(KeyCode::Esc));
    assert!(!app.help_open(), "Esc закрывает справку");
}

#[test]
fn q_and_h_work_from_any_block() {
    let tmp = dir();
    let base = book_pair(tmp.path(), 3);

    let mut app = load(&base, None);
    app.handle_key(key(KeyCode::Char('o')));
    app.handle_key(key(KeyCode::Char('q')));
    assert!(app.should_quit(), "q в фокусе глав выходит");

    let mut app = load(&base, None);
    app.handle_key(key(KeyCode::Char('3')));
    app.handle_key(key(KeyCode::Char('q')));
    assert!(app.should_quit(), "q в фокусе заметок выходит");

    let mut app = load(&base, None);
    app.handle_key(key(KeyCode::Char('4')));
    app.handle_key(key(KeyCode::Char('q')));
    assert!(app.should_quit(), "q в фокусе команд выходит");

    let mut app = load(&base, None);
    app.handle_key(key(KeyCode::Char('o')));
    app.handle_key(key(KeyCode::Char('s')));
    assert_eq!(app.screen(), qbook::app::Screen::Shelf, "s в фокусе глав — на полку");

    let mut app = load(&base, None);
    app.handle_key(key(KeyCode::Char('o')));
    app.handle_key(key(KeyCode::Char('h')));
    assert!(app.help_open(), "h в фокусе глав открывает справку");

    let mut app = load(&base, None);
    app.handle_key(key(KeyCode::Char('?')));
    app.handle_key(key(KeyCode::Char('q')));
    assert!(!app.should_quit(), "q в справке не выходит");
    app.handle_key(key(KeyCode::Esc));
    app.handle_key(key(KeyCode::Char('q')));
    assert!(app.should_quit(), "после закрытия q выходит");
}

#[test]
fn digits_focus_panels_and_shift_digits_toggle_them() {
    let tmp = dir();
    let base = book_pair(tmp.path(), 4);
    let mut app = load(&base, None);
    use qbook::app::ReaderFocus;
    assert!(
        app.toc_visible() && app.bookmarks_visible() && app.commands_visible(),
        "колонки включены по умолчанию"
    );

    let shift = |c: char| KeyEvent::new(KeyCode::Char(c), KeyModifiers::SHIFT);

    app.handle_key(key(KeyCode::Char('2')));
    assert_eq!(app.focus(), ReaderFocus::Toc, "цифра 2 фокусирует главы");

    app.handle_key(shift('2'));
    assert!(!app.toc_visible(), "Shift+2 скрывает главы");
    assert_eq!(app.focus(), ReaderFocus::Text, "фокус вернулся к тексту");
    app.handle_key(shift('2'));
    assert!(app.toc_visible(), "повторный Shift+2 показывает главы");

    app.handle_key(key(KeyCode::Char('3')));
    assert_eq!(app.focus(), ReaderFocus::Bookmarks, "цифра 3 фокусирует заметки");
    app.handle_key(shift('3'));
    assert!(!app.bookmarks_visible(), "Shift+3 скрывает заметки");
    app.handle_key(shift('3'));
    assert!(app.bookmarks_visible(), "повторный Shift+3 показывает заметки");

    app.handle_key(key(KeyCode::Char('4')));
    assert_eq!(app.focus(), ReaderFocus::Commands, "цифра 4 фокусирует команды");
    app.handle_key(shift('4'));
    assert!(!app.commands_visible(), "Shift+4 скрывает команды");
    app.handle_key(shift('4'));
    assert!(app.commands_visible(), "повторный Shift+4 показывает команды");

    app.handle_key(key(KeyCode::Char('1')));
    assert_eq!(app.focus(), ReaderFocus::Text, "цифра 1 возвращает к тексту");

    app.handle_key(shift('1'));
    assert!(
        !app.toc_visible() && !app.bookmarks_visible() && !app.commands_visible(),
        "Shift+1 оставляет только текст"
    );
    assert_eq!(app.focus(), ReaderFocus::Text);
    app.handle_key(shift('1'));
    assert!(
        app.toc_visible() && app.bookmarks_visible() && app.commands_visible(),
        "повторный Shift+1 возвращает колонки"
    );
}

#[test]
fn shift_panels_work_from_any_block_and_cyrillic_layout() {
    use qbook::app::ReaderFocus;
    let tmp = dir();
    let base = book_pair(tmp.path(), 5);
    let mut app = load(&base, None);
    app.set_size(80, 20);
    let shift = |c: char| KeyEvent::new(KeyCode::Char(c), KeyModifiers::SHIFT);

    app.handle_key(key(KeyCode::Char('2')));
    assert_eq!(app.focus(), ReaderFocus::Toc, "фокус в главах");

    app.handle_key(key(KeyCode::Char('"')));
    assert!(!app.toc_visible(), "кириллический Shift+2 (\"\") скрывает главы");
    app.handle_key(key(KeyCode::Char('"')));
    assert!(app.toc_visible(), "повторная кавычка возвращает главы");

    app.handle_key(key(KeyCode::Char('№')));
    assert!(!app.bookmarks_visible(), "кириллический Shift+3 (№) скрывает заметки");
    app.handle_key(key(KeyCode::Char('№')));
    assert!(app.bookmarks_visible(), "повторный № возвращает заметки");

    app.handle_key(key(KeyCode::Char(';')));
    assert!(!app.commands_visible(), "кириллический Shift+4 (;) скрывает команды");
    app.handle_key(key(KeyCode::Char(';')));
    assert!(app.commands_visible(), "повторный ; возвращает команды");

    app.handle_key(key(KeyCode::Char('4')));
    assert_eq!(app.focus(), ReaderFocus::Commands, "фокус в командах");
    app.handle_key(shift('1'));
    assert!(
        !app.toc_visible() && !app.bookmarks_visible() && !app.commands_visible(),
        "Shift+1 из блока скрывает все колонки"
    );
    assert_eq!(app.focus(), ReaderFocus::Text, "фокус уходит к тексту");
}

#[test]
fn text_wraps_to_the_current_column_width() {
    let tmp = dir();
    let base = tmp.path().join("book.md");
    let long = "Очень длинный абзац про прогулку по вечернему городу, ".repeat(6);
    let body = (0..5).map(|i| format!("{long}{i}")).collect::<Vec<_>>().join("\n\n");
    write(&base, &body);

    let mut app = load(&base, None);
    app.set_size(60, 10);
    let shift = |c: char| KeyEvent::new(KeyCode::Char(c), KeyModifiers::SHIFT);
    // 60 − поле 1·2 − левая 24 − борта 2 − индент 2 − запас 5 = 24.
    assert_eq!(app.layout().width(), 24, "перенос по фактической ширине центра");

    app.handle_key(shift('4'));
    app.handle_key(shift('3'));
    assert_eq!(app.layout().width(), 24, "левая колонка ещё стоит");

    app.handle_key(shift('2'));
    assert_eq!(app.layout().width(), 49, "без колонок текст на 60−2−2−2−5");
}

#[test]
fn command_colon_opens_the_prompt_and_enter_executes_shelf() {
    let tmp = dir();
    let base = book_pair(tmp.path(), 4);
    let mut app = load(&base, None);
    app.handle_key(key(KeyCode::Char(':')));
    assert_eq!(app.typing_purpose(), Some(qbook::app::InputPurpose::Command));

    for c in "shelf".chars() {
        app.handle_key(key(KeyCode::Char(c)));
    }
    app.handle_key(key(KeyCode::Enter));
    assert_eq!(app.screen(), qbook::app::Screen::Shelf, ": shelf открывает полку");
}

#[test]
fn unknown_command_shows_a_toast() {
    let tmp = dir();
    let base = book_pair(tmp.path(), 4);
    let mut app = load(&base, None);
    app.handle_key(key(KeyCode::Char('5')));
    assert_eq!(app.typing_purpose(), Some(qbook::app::InputPurpose::Command), "5 открывает prompt");
    for c in "bogus".chars() {
        app.handle_key(key(KeyCode::Char(c)));
    }
    app.handle_key(key(KeyCode::Enter));
    assert_eq!(app.typing_purpose(), None, "prompt закрыт после команды");
    assert_eq!(app.notice(), Some(": нет команды «bogus»"), "тост об ошибке");
}

#[test]
fn command_lang_switches_by_name_or_cycles() {
    let tmp = dir();
    let base = book_pair(tmp.path(), 4);
    let mut app = load(&base, None);

    type_command(&mut app, "lang ru");
    assert_eq!(app.current_lang(), "ru", "lang ru переключает вариант");
    assert!(app.notice().is_some_and(|n| n.contains("ru")), "тост: {:?}", app.notice());

    type_command(&mut app, "lang t");
    assert_eq!(app.current_lang(), "en", "lang t — цикл обратно");
}

#[test]
fn command_goto_jumps_to_the_block() {
    let tmp = dir();
    let base = book_pair(tmp.path(), 26);
    let mut app = load(&base, None);

    type_command(&mut app, "goto 20");
    assert_eq!(app.anchor().block, 19, "goto 20 → блок 20 (индекс 19)");
    assert!(app.notice().is_some_and(|n| n.contains("20")), "тост: {:?}", app.notice());

    type_command(&mut app, "goto 99");
    assert!(app.notice().is_some_and(|n| n.contains("99")), "нет блока 99: {:?}", app.notice());
}

#[test]
fn command_open_loads_a_path() {
    let tmp = dir();
    let loaded = tmp.path().join("loaded.md");
    write(&loaded, "target");
    let base = book_pair(tmp.path(), 4);
    let mut app = load(&base, None);

    type_command(&mut app, &format!("open {}", loaded.display()));
    assert_eq!(app.title(), "loaded", "открыта другая книга");
    assert!(app.notice().is_some_and(|n| n.contains("loaded")), "тост: {:?}", app.notice());
}

fn type_command(app: &mut App, text: &str) {
    app.handle_key(key(KeyCode::Char(':')));
    assert_eq!(app.typing_purpose(), Some(InputPurpose::Command), "prompt команды");
    for c in text.chars() {
        app.handle_key(key(KeyCode::Char(c)));
    }
    app.handle_key(key(KeyCode::Enter));
}

#[test]
fn brackets_widen_and_narrow_the_column_keeping_the_block() {
    let tmp = dir();
    let base = tmp.path().join("book.md");
    let long = "Очень длинный абзац про прогулку по вечернему городу, ".repeat(6);
    let body = (0..12).map(|i| format!("{long}{i}")).collect::<Vec<_>>().join("\n\n");
    write(&base, &body);

    let mut app = load(&base, None);
    app.set_size(60, 10);
    app.set_scroll(15);
    let before = app.anchor();
    let width = app.layout().width();

    app.handle_key(key(KeyCode::Char(']')));
    assert!(app.layout().width() > width, "]: колонка шире");
    assert_eq!(app.anchor().block, before.block, "позиция не прыгнула");

    app.handle_key(key(KeyCode::Char('[')));
    assert_eq!(app.layout().width(), width, "[: обратно к автоширине");
    assert_eq!(app.anchor().block, before.block, "позиция не прыгнула");

    for _ in 0..40 {
        app.handle_key(key(KeyCode::Char('[')));
    }
    assert!(app.layout().width() >= 20, "узкая колонка не уже 20: {}", app.layout().width());
    assert_eq!(app.anchor().block, before.block, "позиция не прыгнула при клампе");
}

#[test]
fn t_cycles_languages_and_out_of_range_keys_are_ignored() {
    let tmp = dir();
    let base = book_pair(tmp.path(), 4);
    let mut app = load(&base, None);
    app.handle_key(key(KeyCode::Char('t')));
    assert_eq!(app.current_lang(), "ru");
    app.handle_key(key(KeyCode::Char('t')));
    assert_eq!(app.current_lang(), "en", "цикл по кругу");

    app.handle_key(key(KeyCode::Char('9')));
    assert_eq!(app.current_lang(), "en");
    assert!(!app.switch_lang(5));
    assert_eq!(app.current_lang(), "en");
}

#[test]
fn s_opens_shelf_and_old_l_keys_are_gone() {
    let tmp = dir();
    let base = book_pair(tmp.path(), 4);

    let mut app = load(&base, None);
    app.handle_key(key(KeyCode::Char('s')));
    assert_eq!(app.screen(), qbook::app::Screen::Shelf, "s открывает полку");
    assert!(!app.help_open(), "s не открывает справку");

    let mut app = load(&base, None);
    app.handle_key(key(KeyCode::Char('l')));
    assert_eq!(app.screen(), qbook::app::Screen::Reader, "l больше не полка");
    assert_eq!(app.current_lang(), "en");

    app.handle_key(key(KeyCode::Char('L')));
    assert_eq!(app.screen(), qbook::app::Screen::Reader, "L больше не язык");
    assert_eq!(app.current_lang(), "en", "L не переключает язык");
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

#[test]
fn shift3_toggles_only_bookmarks_shift4_only_commands() {
    let tmp = dir();
    let base = book_pair(tmp.path(), 4);
    let mut app = load(&base, None);
    app.set_size(60, 12);
    let shift = |c: char| KeyEvent::new(KeyCode::Char(c), KeyModifiers::SHIFT);

    assert!(app.bookmarks_visible());
    assert!(app.commands_visible());

    app.handle_key(shift('3'));
    assert!(!app.bookmarks_visible(), "Shift+3 скрывает заметки");
    assert!(app.commands_visible(), "команды остаются показаны");

    app.handle_key(shift('3'));
    assert!(app.bookmarks_visible(), "Shift+3 возвращает заметки");
    assert!(app.commands_visible(), "команды не тронуты");

    app.handle_key(shift('4'));
    assert!(app.bookmarks_visible(), "заметки не тронуты Shift+4");
    assert!(!app.commands_visible(), "Shift+4 скрывает команды");

    app.handle_key(shift('4'));
    assert!(app.commands_visible(), "Shift+4 возвращает команды");
}

#[test]
fn bracket_keys_narrow_and_widen_the_text() {
    let tmp = dir();
    let base = book_pair(tmp.path(), 4);
    let mut app = load(&base, None);
    app.set_size(60, 12);
    let auto = app.layout().width();

    app.handle_key(key(KeyCode::Char('[')));
    let narrowed = app.layout().width();
    assert!(narrowed < auto, "[ ужимает колонку: {narrowed} < {auto}");

    app.handle_key(key(KeyCode::Char(']')));
    assert_eq!(app.layout().width(), auto, "] возвращает автоширину");

    for _ in 0..20 {
        app.handle_key(key(KeyCode::Char('[')));
    }
    let min = app.layout().width();
    assert!(min < auto, "[ повторный ужимает дальше: {min} < {auto}");
    assert!(min > 0, "колонка не схлопывается в ноль");
}
