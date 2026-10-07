use qbook::app::App;
use qbook::ui::reader;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier};
use tempfile::TempDir;

use crossterm::event::{KeyCode, KeyModifiers};

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
    app.set_size(120, 10);
    app.set_scroll(25);
    let (lines, _) = screen(&mut app, 120, 10);
    let title = &lines[0];
    assert!(title.contains("book"), "заголовок: {title}");
    assert!(!title.contains('%'), "процент уехал из шапки в рамку текста: {title}");
    assert!(title.contains('/'), "позиция: {title}");
    assert!(title.contains("60"), "всего блоков: {title}");
    assert!(!title.contains('⚠'), "качество базы не нуждается в бейдже: {title}");
    let frame = &lines[2];
    assert!(frame.contains('1'), "цифра панели на рамке текста: {frame}");
    assert!(frame.contains('%'), "процент в заголовке текста: {frame}");
    assert!(frame.contains("en"), "язык в заголовке текста: {frame}");

    let bar = lines.last().expect("строки есть");
    assert!(bar.contains("1–4 — блоки"), "подсказка переключения в баре: {bar}");

    let (lines, _) = screen(&mut app, 100, 10);
    let bar = lines.last().expect("строки есть");
    assert!(bar.contains("выход"), "полный бар на широком терминале: {bar}");
    assert!(bar.contains("t язык"), "бар про язык: {bar}");
    assert!(bar.contains("s полка"), "бар про полку: {bar}");
    assert!(bar.contains("h справка"), "бар про справку: {bar}");
    assert!(bar.contains("b заметка"), "динамическая подсказка текста: {bar}");
    let toc = lines.iter().find(|line| line.contains(" Главы ")).expect("панель глав есть");
    assert!(toc.contains("╭2"), "цифра 2 на рамке глав: {toc}");
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
    app.set_size(40, 11);
    let (lines, buffer) = screen(&mut app, 40, 11);

    // Строка 0 — шапка книги, строка 1 — поле сверху, строка 2 — верхняя
    // граница рамки, текст в строках 3+.
    assert!(lines[2].contains('╭'), "рамка со скруглённым углом: {:?}", lines[2]);
    assert!(lines[3].contains("Глава"), "заголовок без решёток: {:?}", lines[3]);
    let heading_bold =
        (0..40u16).any(|x| buffer[(x, 3)].style().add_modifier.contains(Modifier::BOLD));
    assert!(heading_bold, "заголовок жирный");

    assert!(blank_inside_line(&lines[4]), "отступ после заголовка: {:?}", lines[4]);
    assert!(lines[5].contains("▌ первая строка цитаты"), "маркер цитаты: {:?}", lines[5]);
    assert!(lines[6].contains("вторая строка"), "отступ продолжения цитаты: {:?}", lines[6]);
    assert!(blank_inside_frame(&lines[9]), "слот над баром пуст: {:?}", lines[9]);
    assert!(lines[10].contains("1–4 — блоки"), "бар внизу: {:?}", lines[10]);
}

#[test]
fn paragraphs_are_separated_by_a_blank_line_on_screen() {
    let mut app = app_of("первый абзац\n\nвторой абзац");
    app.set_size(40, 10);
    let (lines, _) = screen(&mut app, 40, 10);
    assert_eq!(lines[3].trim_matches(|c: char| c == '│' || c == ' '), "первый абзац");
    assert!(blank_inside_line(&lines[4]), "между абзацами пустая строка: {:?}", lines[4]);
    assert_eq!(lines[5].trim_matches(|c: char| c == '│' || c == ' '), "второй абзац");
}

#[test]
fn long_documents_get_a_scrollbar() {
    let mut app = app_of(&paragraphs(60));
    app.set_size(40, 10);
    let (lines, _) = screen(&mut app, 40, 10);
    // Скроллбар прижат к правой границе колонки текста: рамка со смещённого на
    // поле края, текст индентен — колонка шириной 31 начинается в столбце 3.
    let column: String = lines.iter().map(|l| l.chars().nth(33).unwrap_or(' ')).collect();
    assert!(column.contains('█'), "скроллбар есть: {column:?}");
}

#[test]
fn short_documents_have_no_scrollbar() {
    let mut app = app_of(&paragraphs(3));
    app.set_size(40, 10);
    let (lines, _) = screen(&mut app, 40, 10);
    let column: String = lines.iter().map(|l| l.chars().nth(33).unwrap_or(' ')).collect();
    assert!(!column.contains('█'), "скроллбара нет: {column:?}");
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
    assert!(lines[2].contains("ru"), "текущий язык в заголовке текста: {}", lines[2]);
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
    assert!(all.contains('▮'), "мини-прогресс: {all}");
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
    app.handle_key(key(KeyCode::Enter));
    app.set_scroll(app.max_scroll());
    app.handle_key(key(KeyCode::Char('b')));
    app.handle_key(key(KeyCode::Enter));
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
fn title_hides_the_bookmark_label_once_the_line_is_scrolled() {
    let (_tmp, mut app) = app_with_store_and_bookmarks();
    app.set_scroll(2);
    let (lines, _) = draw(&mut app, 120, 8);
    let title = &lines[0];
    assert!(
        !title.contains("Абзац номер 0"),
        "метка привязана к строке закладки, а не к блоку: {title}"
    );
}

#[test]
fn note_marker_tints_only_the_anchored_line_of_the_block() {
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
    app.handle_key(key(KeyCode::Enter));

    let (lines, buffer) = screen(&mut app, 40, 10);
    assert!(lines[3].contains("▎"), "маркер заметки на якорной строке: {:?}", lines[3]);
    assert_eq!(
        buffer[(3, 3)].style().fg,
        Some(qbook::ui::note_color(5)),
        "маркер цвета заметки: {:?}",
        buffer[(3, 3)].style()
    );
    let markers = lines.iter().map(|l| l.chars().filter(|&c| c == '▎').count()).sum::<usize>();
    assert_eq!(markers, 1, "маркер только на якорной строке: {lines:?}");
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
    app.handle_key(crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Enter,
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
fn panel_digits_highlight_the_active_block() {
    let mut app = app_of(&paragraphs(30));
    app.set_size(96, 10);
    let panel_digit = |lines: &[String], label: &str| {
        let row = lines.iter().position(|l| l.contains(&format!(" {label} "))).expect("панель");
        let col = lines[row].chars().position(|c| c.is_ascii_digit()).expect("цифра") as u16;
        (row as u16, col)
    };
    let fg = |buffer: &ratatui::buffer::Buffer, x: u16, y: u16| buffer[(x, y)].style().fg;
    let text_digit = |row: &str| {
        let byte = row.match_indices("╭1").next().expect("цифра 1 в заголовке текста").0;
        row[..byte].chars().count() as u16 + 1
    };

    let (lines, buffer) = screen(&mut app, 96, 10);
    let (panel_row, panel_col) = panel_digit(&lines, "Главы");
    assert_eq!(
        fg(&buffer, panel_col, panel_row),
        Some(ratatui::style::Color::Yellow),
        "неактивная панель оранжевая"
    );
    let text_col = text_digit(&lines[2]);
    assert_eq!(
        fg(&buffer, text_col, 2),
        Some(ratatui::style::Color::White),
        "активная цифра белая"
    );

    app.handle_key(crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Char('2'),
        crossterm::event::KeyModifiers::NONE,
    ));
    let (lines, buffer) = screen(&mut app, 96, 10);
    let (panel_row, panel_col) = panel_digit(&lines, "Главы");
    assert_eq!(
        fg(&buffer, panel_col, panel_row),
        Some(ratatui::style::Color::White),
        "активная панель белая"
    );
    let text_col = text_digit(&lines[2]);
    assert_eq!(
        fg(&buffer, text_col, 2),
        Some(ratatui::style::Color::Yellow),
        "неактивный текст оранжевый"
    );
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
fn inactive_headings_gray_out_when_the_toc_is_not_focused() {
    let mut app = app_of("# Раздел 1\n\nпервый абзац\n\n# Раздел 2\n\nвторой абзац");
    app.set_size(96, 12);
    let title_x = |line: &str| line.find('Р').expect("заголовок") as u16;

    let (lines, buffer) = screen(&mut app, 96, 12);
    let row_1 = lines.iter().position(|l| l.contains("Раздел 1")).expect("Раздел 1");
    let row_2 = lines.iter().position(|l| l.contains("Раздел 2")).expect("Раздел 2");
    assert_eq!(
        buffer[(title_x(&lines[row_1]), row_1 as u16)].style().fg,
        Some(Color::Cyan),
        "активная глава синяя: {}",
        lines[row_1]
    );
    assert_eq!(
        buffer[(title_x(&lines[row_2]), row_2 as u16)].style().fg,
        Some(Color::DarkGray),
        "неактивная глава серая: {}",
        lines[row_2]
    );

    app.handle_key(crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Char('2'),
        crossterm::event::KeyModifiers::NONE,
    ));
    let (lines, buffer) = screen(&mut app, 96, 12);
    let row_2 = lines.iter().position(|l| l.contains("Раздел 2")).expect("Раздел 2");
    assert_ne!(
        buffer[(title_x(&lines[row_2]), row_2 as u16)].style().fg,
        Some(Color::DarkGray),
        "при фокусе на главах неактивные белые: {}",
        lines[row_2]
    );
}

#[test]
fn inactive_bookmarks_gray_out_when_the_panel_is_not_focused() {
    let (_tmp, mut app) = app_with_store_and_bookmarks();
    app.handle_key(crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Char('1'),
        crossterm::event::KeyModifiers::NONE,
    ));
    app.handle_key(crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Char('4'),
        crossterm::event::KeyModifiers::SHIFT,
    ));
    let (lines, buffer) = draw(&mut app, 96, 14);
    let rows: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, l)| l.contains('●') && l.contains("Абзац"))
        .map(|(i, _)| i)
        .collect();
    assert!(rows.len() >= 2, "видно не меньше двух закладок: {rows:?}");
    let label_col = |line: &str| -> u16 {
        let dot = line.chars().position(|c| c == '●').expect("точка панели") as u16;
        dot + 2
    };
    let gray = rows
        .iter()
        .filter(|&&y| buffer[(label_col(&lines[y]), y as u16)].style().fg == Some(Color::DarkGray))
        .count();
    let white = rows.len() - gray;
    assert!(gray >= rows.len() - 1, "невыбранные закладки серые: {gray}");
    assert_eq!(white, 1, "выбранная закладка белая: {white}");
}

#[test]
fn toc_overlay_lists_headings_above_the_slot() {
    let mut app = app_of(
        "# Раздел 1\n\nпервый абзац\n\n## Раздел 2\n\nвторой абзац\n\n# Раздел 3\n\nтретий абзац",
    );
    app.set_size(60, 12);
    app.handle_key(crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Char('2'),
        crossterm::event::KeyModifiers::NONE,
    ));
    let (lines, _) = draw(&mut app, 60, 12);
    let all = lines.join("\n");
    assert!(all.contains("Главы"), "заголовок панели глав: {all}");
    assert!(all.contains("Раздел 1"), "первый пункт: {all}");
    assert!(all.contains("Раздел 2"), "второй пункт: {all}");
    assert!(all.contains("Раздел 3"), "третий пункт: {all}");
    assert!(lines[2].contains('%'), "метрики текста не перекрыты: {}", lines[2]);
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
    assert!(title.contains("book"), "шапка не перекрыта: {title}");
    let help_row = lines.iter().position(|l| l.contains(" Справка ")).expect("рамка справки");
    assert_eq!(help_row, 3, "справка по центру окна, а не у нижней границы");
}

#[test]
fn help_modal_dims_everything_but_itself() {
    let mut app = app_of("# Абзац номер 0\n\nпервый абзац\n\n# Абзац номер 1\n\nвторой абзац");
    app.set_size(100, 14);

    let (lines, buffer) = screen(&mut app, 100, 14);
    let text_row = lines.iter().position(|l| l.contains('А')).expect("текст");
    let a_col =
        (0..100u16).find(|&x| buffer[(x, text_row as u16)].symbol() == "А").expect("символ");
    assert!(
        !buffer[(a_col, text_row as u16)].style().add_modifier.contains(Modifier::DIM),
        "без справки текст не приглушён"
    );

    app.handle_key(crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Char('h'),
        crossterm::event::KeyModifiers::NONE,
    ));
    let (lines, buffer) = screen(&mut app, 100, 14);
    let text_row = lines.iter().position(|l| l.contains('А')).expect("текст");
    let a_col =
        (0..100u16).find(|&x| buffer[(x, text_row as u16)].symbol() == "А").expect("символ");
    assert!(a_col < 37, "это колонка «Главы», справка её не накрывает: x={a_col}");
    let text_style = buffer[(a_col, text_row as u16)].style();
    assert_eq!(text_style.fg, Some(Color::DarkGray), "текст под справкой тёмно-серый");
    assert!(text_style.add_modifier.contains(Modifier::DIM), "текст под справкой приглушён");
    let left_fg = buffer[(1, 3)].style().fg;
    assert_eq!(left_fg, Some(Color::DarkGray), "левая колонка тоже темнеет: {left_fg:?}");

    let help_row = lines.iter().position(|l| l.contains(" Справка ")).expect("справка");
    let gum_col =
        (0..100u16).find(|&x| buffer[(x, help_row as u16)].symbol() == "С").expect("титул справки");
    assert!(gum_col >= 37, "титул внутри центральной колонки: x={gum_col}");
    let help_style = buffer[(gum_col, help_row as u16)].style();
    assert_ne!(help_style.fg, Some(Color::DarkGray), "рамка справки остаётся яркой");
    assert!(!help_style.add_modifier.contains(Modifier::DIM), "справка не приглушена");

    let help_left = gum_col.saturating_sub(2);
    let bleed = (help_row..help_row + 9.min(14 - help_row))
        .any(|y| (help_left..help_left + 28).any(|x| buffer[(x, y as u16)].symbol() == "А"));
    assert!(!bleed, "внутри справки не просвечивает «А» из текста");
}

#[test]
fn text_title_grows_the_progress_bar_to_three_times() {
    let mut app = app_of(&paragraphs(60));
    app.set_size(120, 10);
    let (lines, _) = screen(&mut app, 120, 10);
    let frame = &lines[2];
    let gauge = |l: &str| l.chars().filter(|&c| c == '▯' || c == '▮').count();
    assert!(gauge(frame) >= 24, "бар занимает 24 ячейки: {}", gauge(frame));
}

#[test]
fn bar_hint_follows_the_focused_block() {
    let mut app = app_of(&paragraphs(10));
    app.set_size(100, 10);
    let bar = |app: &mut App| {
        let (lines, _) = screen(app, 100, 10);
        lines.last().expect("бар").clone()
    };

    let text_bar = bar(&mut app);
    assert!(text_bar.contains("b заметка"), "подсказка текста: {text_bar}");

    app.handle_key(crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Char('2'),
        crossterm::event::KeyModifiers::NONE,
    ));
    let toc_bar = bar(&mut app);
    assert!(toc_bar.contains("к разделу"), "подсказка глав: {toc_bar}");

    app.handle_key(crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Char('3'),
        crossterm::event::KeyModifiers::NONE,
    ));
    let notes_bar = bar(&mut app);
    assert!(notes_bar.contains("к заметке"), "подсказка заметок: {notes_bar}");

    app.handle_key(crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Char('4'),
        crossterm::event::KeyModifiers::NONE,
    ));
    let cmds_bar = bar(&mut app);
    assert!(cmds_bar.contains("подставить"), "подсказка команд: {cmds_bar}");
}

#[test]
fn commands_block_hides_itself_but_bookmarks_stay() {
    let mut app = app_of(&paragraphs(5));
    // Высота 7: правой колонке 5 строк — командам тесно, заметки остаются.
    app.set_size(96, 7);
    let (lines, _) = screen(&mut app, 96, 7);
    let all = lines.join("\n");
    assert!(all.contains("Заметки"), "заметки остаются: {all}");
    assert!(!all.contains(":open"), "команды спрятались сами: {all}");

    // Окно повыше — команды вернулись.
    app.set_size(96, 12);
    let (lines, _) = screen(&mut app, 96, 12);
    let all = lines.join("\n");
    assert!(all.contains(":open"), "команды вернулись: {all}");
    assert!(all.contains("Заметки"), "заметки на месте: {all}");
}

#[test]
fn focused_commands_do_not_eat_the_bookmarks() {
    let mut app = app_of(&paragraphs(5));
    app.set_size(96, 11);
    app.handle_key(crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Char('4'),
        crossterm::event::KeyModifiers::NONE,
    ));
    let (lines, _) = screen(&mut app, 96, 11);
    let all = lines.join("\n");
    assert!(all.contains(":open"), "команды в фокусе видны: {all}");
    assert!(all.contains("Заметки"), "заметки остались: {all}");
    assert!(all.contains("— пусто"), "список заметок не съеден: {all}");
}

#[test]
fn bookmark_marker_marks_only_the_selected_row() {
    let (_tmp, mut app) = app_with_store_and_bookmarks();
    let (lines, _) = draw(&mut app, 96, 12);
    let all = lines.join("\n");
    // ► только у выбранной закладки: в панели команд (":open") свой маркер, поэтому
    // считаем маркеры только на строках-закладках — со знаком цвета «●».
    let bookmark_markers = lines
        .iter()
        .filter(|line| line.contains('●'))
        .map(|line| line.chars().filter(|&c| c == '►').count())
        .sum::<usize>();
    assert_eq!(bookmark_markers, 1, "► только у выбранной закладки: {all}");
}

#[test]
fn many_bookmarks_scroll_inside_the_panel() {
    let tmp = dir();
    std::fs::write(tmp.path().join("book.md"), paragraphs(60)).expect("файл");
    let store = qbook::store::Store::open(tmp.path().join("qbook.db")).expect("хранилище");
    let mut app = App::load(&tmp.path().join("book.md"), "en", &[], Some(store)).expect("загрузка");
    app.set_size(50, 12);
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let key = |code: KeyCode| KeyEvent::new(code, KeyModifiers::NONE);
    for i in (0..40).rev() {
        app.set_scroll(i * 2);
        app.handle_key(key(KeyCode::Char('b')));
        app.handle_key(key(KeyCode::Enter));
        app.handle_key(key(KeyCode::Enter));
    }
    app.set_scroll(0);
    app.handle_key(key(KeyCode::Char('B')));
    for _ in 0..50 {
        app.handle_key(key(KeyCode::Char('j')));
    }
    let last_label = app.bookmarks().last().map(|b| b.label.clone()).expect("закладки есть");
    let (lines, _) = draw(&mut app, 96, 12);
    let all = lines.join("\n");
    assert!(
        all.contains(&last_label),
        "последняя закладка ({last_label}) достижима листанием: {all}"
    );
}

#[test]
fn many_headings_scroll_inside_the_toc_panel() {
    let mut content = String::from("# Глава 0");
    for i in 1..30 {
        content.push_str(&format!("\n\n## Глава {i}\n\nАбзац"));
    }
    let mut app = app_of(&content);
    app.set_size(50, 12);
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let key = |code: KeyCode| KeyEvent::new(code, KeyModifiers::NONE);
    app.handle_key(key(KeyCode::Char('o')));
    for _ in 0..40 {
        app.handle_key(key(KeyCode::Char('j')));
    }
    let (lines, _) = draw(&mut app, 96, 12);
    let all = lines.join("\n");
    assert!(all.contains("Глава 29"), "последняя глава достижима листанием: {all}");
}

#[test]
fn hidden_bookmarks_leave_commands_alone_in_right_column() {
    let mut app = app_of(&paragraphs(5));
    app.set_size(84, 14);
    let (lines, _) = screen(&mut app, 96, 14);
    let joined = lines.join("\n");
    assert!(joined.contains(" Заметки "), "заметки видимы: {joined}");
    assert!(joined.contains(" Команды "), "команды видимы: {joined}");

    let shift3 = crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Char('3'),
        crossterm::event::KeyModifiers::SHIFT,
    );
    app.handle_key(shift3);
    let (lines, _) = screen(&mut app, 96, 14);
    let joined = lines.join("\n");
    assert!(!joined.contains("3 Заметки "), "заметки скрыты: {joined}");
    assert!(joined.contains(" Команды "), "команды остались: {joined}");
    assert!(joined.contains(":open"), "команды занимают колонку: {joined}");
}

#[test]
fn text_fades_towards_the_bottom_edge() {
    let mut app = app_of(&paragraphs(10));
    app.set_size(40, 10);
    let (_lines, buffer) = screen(&mut app, 40, 10);
    // Нижняя внутренняя строка рамки текста: слот (1) + бар (1) + поле (1) +
    // нижний борт (1), сама рамка сдвинута на поле сверху.
    let bottom_text_row = 10u16 - 5;
    let dim = (0..40u16)
        .any(|x| buffer[(x, bottom_text_row)].style().add_modifier.contains(Modifier::DIM));
    assert!(dim, "нижняя строка текста приглушена");
    let mid = (0..40u16)
        .any(|x| buffer[(x, bottom_text_row - 3)].style().add_modifier.contains(Modifier::DIM));
    assert!(!mid, "выше зоны градиента текст яркий");
}

/// Правый край рамки (столбцы 68..72, строки 3..9) пуст — от сужения колонки.
fn right_margin_empty(buffer: &Buffer) -> bool {
    (68..=72).all(|x| (3..9).all(|y| buffer[(x, y)].symbol() == " "))
}

#[test]
fn manual_narrowing_moves_the_text_and_leaves_space_on_the_right() {
    let long = "По вечернему городу гуляет очень длинный абзац, ".repeat(6);
    let body = (0..30).map(|i| format!("{long} {i}")).collect::<Vec<_>>().join("\n\n");
    let mut app = app_of(&body);
    app.set_size(80, 14);
    let (_, buffer) = screen(&mut app, 80, 14);
    assert!(!right_margin_empty(&buffer), "по автоширине текст занимает правую часть");

    let key = |c: char| crossterm::event::KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE);
    app.handle_key(key('['));
    app.handle_key(key('['));
    let (_, buffer) = screen(&mut app, 80, 14);
    assert!(right_margin_empty(&buffer), "после [ справа от текста — пустое место");
}

#[test]
fn column_limit_shows_a_hint_in_the_bar() {
    let mut app = app_of(&paragraphs(30));
    app.set_size(120, 14);
    let key = |c: char| crossterm::event::KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE);
    for _ in 0..50 {
        app.handle_key(key(']'));
    }
    let (lines, _) = screen(&mut app, 120, 14);
    let bar = lines.last().expect("бар есть");
    assert!(bar.contains("шире некуда"), "подсказка предела в баре: {bar}");

    app.handle_key(key('['));
    let (lines, _) = screen(&mut app, 120, 14);
    let bar = lines.last().expect("бар есть");
    assert!(!bar.contains("предел"), "успешное сужение снимает подсказку: {bar}");
}

#[test]
fn bold_and_italic_markdown_are_styled_in_the_text() {
    let mut app = app_of("Обычный **жирный** и *курсивный* текст");
    app.set_size(80, 14);
    app.set_scroll(0);
    let (_lines, buffer) = screen(&mut app, 80, 14);
    // Заголовка нет, цитат нет — первая строка текста в третьей строке экрана,
    // текст начинается с внутреннего поля рамки (столбец 40).
    let y = 3u16;
    let first_row: String = (40..64).map(|x| buffer[(x, y)].symbol()).collect();
    for x in 40..64 {
        let symbol = buffer[(x, y)].symbol();
        assert_ne!(symbol, "*", "звёздочка-маркер не рендерится в {x}: {first_row:?}");
        assert_ne!(symbol, "_", "подчёркивание-маркер не рендерится в {x}: {first_row:?}");
    }
    // `**жирный**` начинается после 8 символов.
    let bold = buffer[(48, y)].style();
    assert!(
        bold.add_modifier.contains(Modifier::BOLD),
        "жирный выделен жирным: {bold:?} строка {first_row:?}"
    );
    assert!(!bold.add_modifier.contains(Modifier::ITALIC), "жирный не курсив");
    // `*курсивный*` начинается после «жирный и ».
    let italic = buffer[(57, y)].style();
    assert!(
        italic.add_modifier.contains(Modifier::ITALIC),
        "курсивный выделен курсивом: {italic:?} строка {first_row:?}"
    );
    assert!(!italic.add_modifier.contains(Modifier::BOLD), "курсив не жирный");
}

#[test]
fn rule_renders_as_a_full_width_dividing_line() {
    let mut app = app_of("До разделителя\n\n---\n\nПосле разделителя");
    app.set_size(80, 14);
    let (_, buffer) = screen(&mut app, 80, 14);
    let rule_row = (3..9).find(|&y| buffer[(40, y)].symbol() == "─").expect("линия разделителя");
    assert_eq!(buffer[(76, rule_row)].symbol(), "─", "линия тянется до края рамки");
    assert_eq!(buffer[(40, rule_row - 1)].symbol(), " ", "перед линией — отступ абзаца");
    let below = buffer[(40, rule_row + 1)].symbol();
    assert_ne!(below, "─", "под линией начинается текст, а не вторая линия");
    assert_ne!(below, " ", "под линией нет пустой строки");
}
