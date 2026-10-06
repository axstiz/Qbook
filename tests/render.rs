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
fn title_line_shows_position_percent_and_quality() {
    let mut app = app_of(&paragraphs(60));
    app.set_size(40, 10);
    app.set_scroll(25);
    let (lines, _) = screen(&mut app, 40, 10);
    println!("TITLE-TEST={lines:?}");
    let title = &lines[0];
    assert!(title.contains("book"), "заголовок: {title}");
    assert!(title.contains('%'), "процент: {title}");
    assert!(title.contains('/'), "позиция: {title}");
    assert!(title.contains("60"), "всего блоков: {title}");
    assert!(!title.contains('⚠'), "качество базы не нуждается в бейдже: {title}");

    let bar = lines.last().expect("строки есть");
    assert!(bar.contains("1:Текст"), "цифровая панель в баре: {bar}");
    assert!(bar.contains("Главы"), "панели глав в баре: {bar}");

    let (lines, _) = screen(&mut app, 80, 10);
    let bar = lines.last().expect("строки есть");
    assert!(bar.contains("выход"), "полный бар на широком терминале: {bar}");
    assert!(bar.contains("t язык"), "бар про язык: {bar}");
    assert!(bar.contains("h полка"), "бар про полку: {bar}");
}

fn blank_inside_frame(line: &str) -> bool {
    line.chars().all(|c| matches!(c, '│' | '░' | '╮' | ' '))
}

fn blank_inside_line(line: &str) -> bool {
    line.chars().all(|c| matches!(c, '│' | ' '))
}

#[test]
fn headings_quotes_and_paragraphs_are_decorated() {
    let mut app = app_of("# Глава\n\n> первая строка цитаты\nвторая строка\n\nобычный абзац");
    app.set_size(40, 10);
    let (lines, buffer) = screen(&mut app, 40, 10);

    // Строка 0 — верхняя граница рамки; текст начинается в строке 1.
    assert!(lines[0].contains('╭'), "рамка со скруглённым углом: {:?}", lines[0]);
    assert!(lines[1].starts_with("│Глава"), "заголовок без решёток: {:?}", lines[1]);
    let heading_bold =
        (0..40u16).any(|x| buffer[(x, 1)].style().add_modifier.contains(Modifier::BOLD));
    assert!(heading_bold, "заголовок жирный");

    assert!(blank_inside_line(&lines[2]), "отступ после заголовка: {:?}", lines[2]);
    assert!(lines[3].contains("▌ первая строка цитаты"), "маркер цитаты: {:?}", lines[3]);
    assert!(lines[4].starts_with("│  вторая строка"), "отступ продолжения цитаты: {:?}", lines[4]);
    assert!(blank_inside_line(&lines[5]), "отступ после цитаты: {:?}", lines[5]);
    assert!(lines[6].starts_with("│обычный абзац"), "абзац без отступа: {:?}", lines[6]);
    assert!(blank_inside_frame(&lines[8]), "слот над баром пуст: {:?}", lines[8]);
    assert!(lines[9].contains("1:Текст"), "бар внизу: {:?}", lines[9]);
}

#[test]
fn paragraphs_are_separated_by_a_blank_line_on_screen() {
    let mut app = app_of("первый абзац\n\nвторой абзац");
    app.set_size(40, 8);
    let (lines, _) = screen(&mut app, 40, 8);
    assert_eq!(lines[1].trim_matches(|c: char| c == '│' || c == ' '), "первый абзац");
    assert!(blank_inside_line(&lines[2]), "между абзацами пустая строка: {:?}", lines[2]);
    assert_eq!(lines[3].trim_matches(|c: char| c == '│' || c == ' '), "второй абзац");
}

#[test]
fn long_documents_get_a_scrollbar() {
    let mut app = app_of(&paragraphs(60));
    app.set_size(40, 10);
    let (lines, _) = screen(&mut app, 40, 10);
    assert!(lines.iter().any(|line| line.contains('█')), "скроллбар есть: {lines:?}");
}

#[test]
fn short_documents_have_no_scrollbar() {
    let mut app = app_of(&paragraphs(3));
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
    let title = &lines[0];
    assert!(title.contains('⚠'), "бейдж качества в заголовке: {title}");
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
    let title = &lines[0];
    assert!(!title.contains('⚠'), "качество в норме: {title}");
    assert!(title.contains("ru"), "текущий язык в заголовке: {title}");
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
    assert!(all.contains('▓'), "мини-прогресс: {all}");
    let cursor = lines.iter().find(|l| l.contains("Война и мир")).expect("строка книги");
    assert!(cursor.contains('►'), "курсор полки отмечен: {cursor}");
}

fn app_with_store_and_bookmarks() -> (TempDir, App) {
    let tmp = dir();
    std::fs::write(tmp.path().join("book.md"), paragraphs(60)).expect("файл");
    let store = qbook::store::Store::open(tmp.path().join("qbook.db")).expect("хранилище");
    let mut app = App::load(&tmp.path().join("book.md"), "en", &[], Some(store)).expect("загрузка");
    app.set_size(50, 12);
    app.set_scroll(0);
    use crossterm::event::{KeyCode, KeyModifiers};
    let key = |code: KeyCode| crossterm::event::KeyEvent::new(code, KeyModifiers::NONE);
    app.handle_key(key(KeyCode::Char('b')));
    app.handle_key(key(KeyCode::Enter));
    app.set_scroll(app.max_scroll());
    app.handle_key(key(KeyCode::Char('b')));
    app.handle_key(key(KeyCode::Enter));
    app.handle_key(key(KeyCode::Char('B')));
    (tmp, app)
}

#[test]
fn bookmarks_panel_shows_labels_with_the_selection_bold() {
    let (_tmp, mut app) = app_with_store_and_bookmarks();
    // Скрываем команды (Shift+4), чтобы список занял всю правую колонку.
    app.handle_key(crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Char('4'),
        crossterm::event::KeyModifiers::SHIFT,
    ));
    let (lines, buffer) = draw(&mut app, 96, 12);
    let all = lines.join("\n");
    assert!(all.contains("Заметки"), "заголовок панели: {all}");
    assert!(all.contains("Абзац номер 0"), "метка в панели: {all}");
    assert!(all.contains('●'), "цветная точка заметки в панели: {all}");

    let selected_row = lines
        .iter()
        .position(|l| l.contains('►') && l.contains("Абзац"))
        .expect("выделенная строка панели");
    let bold = (0..96u16)
        .any(|x| buffer[(x, selected_row as u16)].style().add_modifier.contains(Modifier::BOLD));
    assert!(bold, "выбранная закладка жирная");
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
fn title_highlights_the_bookmark_label() {
    let (_tmp, mut app) = app_with_store_and_bookmarks();
    app.set_scroll(0);
    let (lines, _) = draw(&mut app, 120, 8);
    let title = &lines[0];
    assert!(title.contains("Абзац номер 0"), "метка закладки в заголовке: {title}");
}

#[test]
fn note_marker_tints_the_first_column_of_the_bookmarked_block() {
    let tmp = dir();
    std::fs::write(tmp.path().join("book.md"), paragraphs(30)).expect("файл");
    let store = qbook::store::Store::open(tmp.path().join("qbook.db")).expect("хранилище");
    let mut app = App::load(&tmp.path().join("book.md"), "en", &[], Some(store)).expect("загрузка");
    app.set_size(40, 10);
    app.set_scroll(0);
    use crossterm::event::{KeyCode, KeyModifiers};
    let key = |code: KeyCode| crossterm::event::KeyEvent::new(code, KeyModifiers::NONE);
    app.handle_key(key(KeyCode::Char('b')));
    app.handle_key(key(KeyCode::Enter));

    let (lines, buffer) = screen(&mut app, 40, 10);
    assert!(lines[1].starts_with("│▎"), "маркер заметки на блоке: {:?}", lines[1]);
    assert!(
        buffer[(1, 1)].style().fg == Some(qbook::ui::note_color(5)),
        "маркер цвета заметки: {:?}",
        buffer[(1, 1)].style()
    );
    let blank = lines.iter().position(|l| l.starts_with("│▎ "));
    assert_eq!(blank, Some(2), "маркер на отступе виден: {lines:?}");
}

#[test]
fn note_prompt_renders_its_label() {
    let tmp = dir();
    std::fs::write(tmp.path().join("book.md"), paragraphs(20)).expect("файл");
    let store = qbook::store::Store::open(tmp.path().join("qbook.db")).expect("хранилище");
    let mut app = App::load(&tmp.path().join("book.md"), "en", &[], Some(store)).expect("загрузка");
    app.handle_key(crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Char('b'),
        crossterm::event::KeyModifiers::NONE,
    ));
    let (lines, _) = draw(&mut app, 60, 10);
    let all = lines.join("\n");
    assert!(all.contains("Заметка:"), "подпись prompt для заметки: {all}");
    app.handle_key(crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Esc,
        crossterm::event::KeyModifiers::NONE,
    ));
}

#[test]
fn bar_highlights_the_digit_of_the_active_block() {
    let mut app = app_of(&paragraphs(30));
    app.set_size(80, 10);
    let bar_row = 9u16;
    let digit_at = |lines: &[String], digit: char| {
        lines[bar_row as usize].chars().position(|c| c == digit).expect("цифра в баре") as u16
    };
    let fg = |buffer: &ratatui::buffer::Buffer, x: u16| buffer[(x, bar_row)].style().fg;

    let (lines, buffer) = screen(&mut app, 80, 10);
    let idx = digit_at(&lines, '2');
    assert_eq!(fg(&buffer, idx), Some(ratatui::style::Color::DarkGray), "неактивная цифра серая");

    app.handle_key(crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Char('2'),
        crossterm::event::KeyModifiers::NONE,
    ));
    let (lines, buffer) = screen(&mut app, 80, 10);
    let idx = digit_at(&lines, '2');
    assert_eq!(fg(&buffer, idx), Some(ratatui::style::Color::White), "активная цифра белая");
    let text_idx = digit_at(&lines, '1');
    assert_eq!(fg(&buffer, text_idx), Some(ratatui::style::Color::DarkGray), "текст стал серым");
}

#[test]
fn wide_window_shows_columns_and_highlights_the_active_heading() {
    let mut app = app_of(
        "# Раздел 1\n\nпервый абзац\n\n## Раздел 2\n\nвторой абзац\n\n# Раздел 3\n\nтретий абзац",
    );
    app.set_size(120, 16);
    let (lines, buffer) = screen(&mut app, 120, 16);
    let all = lines.join("\n");
    for title in ["Главы", "Заметки", "Команды"] {
        assert!(all.contains(title), "колонка «{title}» на экране: {all}");
    }

    let cyan = (0..24u16).flat_map(|x| (0..16u16).map(move |y| (x, y))).any(|(x, y)| {
        buffer[(x, y)].symbol() == "Р"
            && buffer[(x, y)].style().fg == Some(ratatui::style::Color::Cyan)
    });
    assert!(cyan, "активная глава подсвечена голубым в левой колонке");
}

#[test]
fn toc_overlay_lists_headings_above_the_slot() {
    let mut app = app_of(
        "# Раздел 1\n\nпервый абзац\n\n## Раздел 2\n\nвторой абзац\n\n# Раздел 3\n\nтретий абзац",
    );
    app.set_size(50, 12);
    app.handle_key(crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Char('2'),
        crossterm::event::KeyModifiers::NONE,
    ));
    let (lines, _) = draw(&mut app, 50, 12);
    let all = lines.join("\n");
    assert!(all.contains("Главы"), "заголовок панели глав: {all}");
    assert!(all.contains("Раздел 1"), "первый пункт: {all}");
    assert!(all.contains("Раздел 2"), "второй пункт: {all}");
    assert!(all.contains("Раздел 3"), "третий пункт: {all}");
    let title = &lines[0];
    assert!(title.contains('%'), "метрики не перекрыты: {title}");
}

#[test]
fn help_overlay_shows_bindings_and_keeps_status_visible() {
    let mut app = app_of(&paragraphs(10));
    app.set_size(40, 14);
    app.handle_key(crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Char('?'),
        crossterm::event::KeyModifiers::NONE,
    ));
    let (lines, _) = draw(&mut app, 40, 14);
    let all = lines.join("\n");
    assert!(all.contains("Справка"), "заголовок справки: {all}");
    assert!(all.contains("оглавление"), "строка об оглавлении: {all}");
    let title = &lines[0];
    assert!(title.contains('%'), "метрики не перекрыты: {title}");
}
