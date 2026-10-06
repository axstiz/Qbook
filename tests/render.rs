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
