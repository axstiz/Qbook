use qbook::app::App;
use qbook::ui::reader;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::style::Modifier;
use tempfile::TempDir;

fn dir() -> TempDir {
    tempfile::tempdir().expect("каталог")
}

fn paragraphs(count: usize) -> String {
    (0..count).map(|i| format!("Абзац номер {i}")).collect::<Vec<_>>().join("\n\n")
}

/// Книга живёт в памяти приложения, файлы удаляются вместе с каталогом.
fn app_of(content: &str) -> App {
    let tmp = dir();
    let path = tmp.path().join("book.md");
    std::fs::write(&path, content).expect("записать");
    App::load(&path, "en", &[], None).expect("загрузка")
}

fn screen(app: &mut App, width: u16, height: u16) -> (Vec<String>, Buffer) {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("терминал");
    terminal.draw(|frame| reader::render(app, frame)).expect("отрисовка");
    let buffer = terminal.backend().buffer().clone();
    let lines = (0..height)
        .map(|y| (0..width).map(|x| buffer[(x, y)].symbol()).collect::<String>())
        .collect();
    (lines, buffer)
}

#[test]
fn status_bar_shows_position_percent_and_hint() {
    let mut app = app_of(&paragraphs(60));
    app.set_size(40, 10);
    app.set_scroll(25);
    let (lines, _) = screen(&mut app, 40, 10);
    let status = lines.last().expect("строки есть");
    assert!(status.contains("book"), "заголовок: {status}");
    assert!(status.contains('%'), "процент: {status}");
    assert!(status.contains("блок"), "позиция: {status}");
    assert!(status.contains("60"), "всего блоков: {status}");
    assert!(status.contains("j/k"), "подсказка: {status}");
    assert!(!status.contains('⚠'), "качество базы не нуждается в бейдже: {status}");

    let (lines, _) = screen(&mut app, 80, 10);
    let status = lines.last().expect("строки есть");
    assert!(status.contains("выход"), "полная подсказка на широком терминале: {status}");
    assert!(status.contains("t язык"), "подсказка про язык: {status}");
    assert!(status.contains("h полка"), "подсказка про полку: {status}");
}

#[test]
fn headings_quotes_and_paragraphs_are_decorated() {
    let mut app = app_of("# Глава\n\n> первая строка цитаты\nвторая строка\n\nобычный абзац");
    app.set_size(40, 10);
    let (lines, buffer) = screen(&mut app, 40, 10);

    assert!(lines[0].starts_with("Глава"), "заголовок без решёток: {:?}", lines[0]);
    let heading_bold =
        (0..40).any(|x| buffer[(x, 0)].style().add_modifier.contains(Modifier::BOLD));
    assert!(heading_bold, "заголовок жирный");

    assert!(lines[1].contains("▌ первая строка цитаты"), "маркер цитаты: {:?}", lines[1]);
    assert!(lines[2].starts_with("  вторая строка"), "отступ продолжения цитаты: {:?}", lines[2]);
    assert!(lines[3].starts_with("обычный абзац"), "абзац без отступа: {:?}", lines[3]);
}

#[test]
fn long_documents_get_a_scrollbar() {
    let mut app = app_of(&paragraphs(60));
    app.set_size(40, 10);
    let (lines, _) = screen(&mut app, 40, 10);
    assert!(lines.iter().any(|line| line.ends_with('█')), "скроллбар есть: {lines:?}");
}

#[test]
fn short_documents_have_no_scrollbar() {
    let mut app = app_of(&paragraphs(4));
    app.set_size(40, 10);
    let (lines, _) = screen(&mut app, 40, 10);
    assert!(!lines.iter().any(|line| line.contains('█')), "скроллбара нет: {lines:?}");
}

#[test]
fn low_alignment_quality_is_badged() {
    let tmp = dir();
    let path = tmp.path().join("book.md");
    std::fs::write(&path, paragraphs(20)).expect("база");
    // Перевод с трёмя абзацами вместо двадцати: выравнивание почти ничего не находит.
    std::fs::write(
        tmp.path().join("book.ru.md"),
        "Очень длинный первый абзац перевода, растянутый вместо многих базовых.\n\n\
         Второй длинный абзац с совершенно иной структурой внутри.\n\n\
         Третий абзац завершает этот странный перевод.",
    )
    .expect("перевод");

    let mut app = App::load(&path, "en", &[], None).expect("загрузка");
    assert!(app.switch_lang(1));
    app.set_size(40, 10);
    let (lines, _) = screen(&mut app, 40, 10);
    let status = lines.last().expect("строки есть");
    assert!(status.contains('⚠'), "бейдж качества: {status}");
}

#[test]
fn good_alignment_has_no_badge() {
    let tmp = dir();
    let path = tmp.path().join("book.md");
    std::fs::write(&path, paragraphs(30)).expect("база");
    std::fs::write(tmp.path().join("book.ru.md"), paragraphs(30)).expect("перевод");

    let mut app = App::load(&path, "en", &[], None).expect("загрузка");
    assert!(app.switch_lang(1));
    app.set_size(40, 10);
    let (lines, _) = screen(&mut app, 40, 10);
    let status = lines.last().expect("строки есть");
    assert!(!status.contains('⚠'), "качество в норме: {status}");
    assert!(status.contains("ru"), "текущий язык: {status}");
}

#[test]
fn renders_in_a_single_line_terminal() {
    let mut app = app_of(&paragraphs(10));
    let (lines, _) = screen(&mut app, 24, 1);
    assert_eq!(lines.len(), 1);
    assert!(lines[0].contains('%'), "статус занимает всю строку: {:?}", lines[0]);
    assert!(!lines[0].contains("Абзац"), "места под текст не осталось: {:?}", lines[0]);
}

fn draw(app: &mut App, width: u16, height: u16) -> (Vec<String>, Buffer) {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("терминал");
    terminal.draw(|frame| qbook::ui::render(app, frame)).expect("отрисовка");
    let buffer = terminal.backend().buffer().clone();
    let lines = (0..height)
        .map(|y| (0..width).map(|x| buffer[(x, y)].symbol()).collect::<String>())
        .collect();
    (lines, buffer)
}

fn shelf_app_with_one_book() -> (TempDir, App) {
    let tmp = dir();
    let store = qbook::store::Store::open(tmp.path().join("qbook.db")).expect("хранилище");
    let id =
        store.add_book("/books/war.md", "Война и мир", "en", 1_700_000_000, 100).expect("книга");
    store.set_variant(id, "ru", "/books/war.ru.md", "md").expect("вариант");
    store.set_progress(id, qbook::model::Anchor::START, "en", 42.5).expect("прогресс");
    let app = App::shelf(Some(store), "en").expect("полка");
    (tmp, app)
}

#[test]
fn shelf_renders_title_langs_percent_and_date() {
    let (_tmp, mut app) = shelf_app_with_one_book();
    let (lines, _) = draw(&mut app, 80, 8);
    let all = lines.join("\n");
    assert!(all.contains("Полка"), "заголовок полки: {all}");
    assert!(all.contains("Война и мир"), "заголовок книги: {all}");
    assert!(all.contains("42.5"), "прогресс: {all}");
    assert!(all.contains("2023-11-14"), "дата добавления: {all}");
    assert!(all.contains("en"), "базовый язык: {all}");
    assert!(all.contains("ru"), "перевод: {all}");
    let cursor = lines.iter().find(|l| l.contains("Война и мир")).expect("строка книги");
    assert!(cursor.starts_with('>'), "курсор полки отмечен: {cursor}");
}

fn app_with_store_and_bookmarks() -> (TempDir, App) {
    let tmp = dir();
    std::fs::write(tmp.path().join("book.md"), paragraphs(60)).expect("файл");
    let store = qbook::store::Store::open(tmp.path().join("qbook.db")).expect("хранилище");
    let mut app = App::load(&tmp.path().join("book.md"), "en", &[], Some(store)).expect("загрузка");
    app.set_size(50, 12);
    app.set_scroll(0);
    use crossterm::event::{KeyCode, KeyModifiers};
    app.handle_key(crossterm::event::KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE));
    app.set_scroll(app.max_scroll());
    app.handle_key(crossterm::event::KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE));
    app.handle_key(crossterm::event::KeyEvent::new(KeyCode::Char('B'), KeyModifiers::NONE));
    (tmp, app)
}

#[test]
fn bookmarks_panel_shows_labels_with_the_selection_bold() {
    let (_tmp, mut app) = app_with_store_and_bookmarks();
    let (lines, buffer) = draw(&mut app, 50, 12);
    let all = lines.join("\n");
    assert!(all.contains("Закладки:"), "заголовок панели: {all}");
    assert!(all.contains("Абзац номер 0"), "метка в панели: {all}");

    let selected_row = lines
        .iter()
        .position(|l| l.starts_with('>') && l.contains("Абзац"))
        .expect("выделенная строка панели");
    let bold = (0..50u16)
        .any(|x| buffer[(x, selected_row as u16)].style().add_modifier.contains(Modifier::BOLD));
    assert!(bold, "выбранная закладка подчёркнута жирным");
}

#[test]
fn prompt_line_renders_typed_path() {
    let tmp = dir();
    let store = qbook::store::Store::open(tmp.path().join("qbook.db")).expect("хранилище");
    let mut app = App::shelf(Some(store), "en").expect("полка");
    use crossterm::event::{KeyCode, KeyModifiers};
    let key = |c: char| crossterm::event::KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE);
    app.handle_key(key('a'));
    for c in "/books/war.md".chars() {
        app.handle_key(key(c));
    }
    let (lines, _) = draw(&mut app, 60, 6);
    let all = lines.join("\n");
    assert!(all.contains("Путь:"), "подпись prompt: {all}");
    assert!(all.contains("/books/war.md"), "введённый путь: {all}");
}

#[test]
fn status_bar_highlights_the_bookmark_label() {
    let (_tmp, mut app) = app_with_store_and_bookmarks();
    app.set_scroll(0);
    let (lines, _) = draw(&mut app, 60, 8);
    let status = lines.last().expect("статус");
    assert!(status.contains("Абзац номер 0"), "метка закладки в статусе: {status}");
}

#[test]
fn toc_overlay_lists_headings_above_the_status_line() {
    let mut app = app_of(
        "# Раздел 1\n\nпервый абзац\n\n## Раздел 2\n\nвторой абзац\n\n# Раздел 3\n\nтретий абзац",
    );
    app.set_size(50, 12);
    app.handle_key(crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Char('o'),
        crossterm::event::KeyModifiers::NONE,
    ));
    let (lines, _) = draw(&mut app, 50, 12);
    let all = lines.join("\n");
    assert!(all.contains("Оглавление"), "заголовок шторки: {all}");
    assert!(all.contains("Раздел 1"), "первый пункт: {all}");
    assert!(all.contains("Раздел 2"), "второй пункт: {all}");
    assert!(all.contains("Раздел 3"), "третий пункт: {all}");
    let status = lines.last().expect("статус");
    assert!(status.contains('%'), "статус не перекрыт: {status}");
}

#[test]
fn help_overlay_shows_bindings_and_keeps_status_visible() {
    let mut app = app_of(&paragraphs(10));
    app.set_size(50, 14);
    app.handle_key(crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Char('?'),
        crossterm::event::KeyModifiers::NONE,
    ));
    let (lines, _) = draw(&mut app, 50, 14);
    let all = lines.join("\n");
    assert!(all.contains("Справка"), "заголовок справки: {all}");
    assert!(all.contains("оглавление"), "строка об оглавлении: {all}");
    let status = lines.last().expect("статус");
    assert!(status.contains('%'), "статус не перекрыт: {status}");
}
