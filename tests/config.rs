//! Конфигурация на уровне интеграции: флаг `--theme`, мерж файла поверх
//! пресета и докатка темы/настроек до буфера рендера.

use clap::Parser;
use qbook::app::App;
use qbook::cli::Cli;
use qbook::config::{self, Config, Theme};
use qbook::ui::reader;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier};
use tempfile::TempDir;

fn dir() -> TempDir {
    tempfile::tempdir().expect("каталог")
}

fn paragraphs(count: usize) -> String {
    (0..count).map(|i| format!("Абзац номер {i}")).collect::<Vec<_>>().join("\n\n")
}

fn app_with(path_book: &std::path::Path, content: &str, config: Config) -> App {
    std::fs::write(path_book, content).expect("записать");
    App::load_with(path_book, "en", &[], None, config).expect("загрузка")
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
fn theme_flag_accepts_presets_and_rejects_the_rest() {
    let ok = Cli::try_parse_from(["qbook", "--theme", "mono", "book.md"]).expect("mono известен");
    assert_eq!(ok.theme.as_deref(), Some("mono"));
    let err =
        Cli::try_parse_from(["qbook", "--theme", "neon", "book.md"]).expect_err("нет пресета");
    let text = err.to_string();
    assert!(text.contains("neon"), "имя в ошибке: {text}");
    assert!(text.contains("mono"), "список пресетов в ошибке: {text}");
}

#[test]
fn file_patches_the_preset_before_render() {
    let dir = dir();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "[theme]\nkey = \"cyan\"\n[defaults]\nwheel_lines = 9\n")
        .expect("конфиг");
    let base = Config { theme: Theme::mono(), ..Config::default() };
    let config = config::load_from(&path, base, true).expect("конфиг читается");
    assert_eq!(config.theme.key, Color::Cyan, "файл поверх пресета");
    assert_eq!(config.theme.accent, Color::White, "остальное от mono");
    assert_eq!(config.defaults.wheel_lines, 9);
}

#[test]
fn overridden_key_color_reaches_the_bar() {
    let dir = dir();
    let config =
        Config { theme: Theme { key: Color::Cyan, ..Theme::default() }, ..Config::default() };
    let mut app = app_with(&dir.path().join("book.md"), &paragraphs(30), config);
    app.set_size(120, 10);
    app.set_scroll(0);
    let (lines, buffer) = screen(&mut app, 120, 10);
    let bar = lines.last().expect("бар есть");
    assert!(bar.contains('b'), "клавиша в баре: {bar}");
    let bar_y = 9;
    let cyan = (0..120).any(|x| buffer[(x, bar_y)].style().fg == Some(Color::Cyan));
    assert!(cyan, "цвет темы доехал до барь: {lines:?}");
    let yellow = (0..120).any(|x| buffer[(x, bar_y)].style().fg == Some(Color::Yellow));
    assert!(!yellow, "дефолтный жёлтый вытеснен: {bar}");
}

#[test]
fn fade_text_switch_controls_bottom_dimming() {
    let dir = dir();
    let faded = Config {
        defaults: qbook::config::Defaults { fade_text: true, ..Default::default() },
        ..Config::default()
    };
    let plain = Config {
        defaults: qbook::config::Defaults { fade_text: false, ..Default::default() },
        ..Config::default()
    };
    // Нижняя строка внутренней области текста (y = 5 при высоте 10):
    // центральная колонка, только буквенные ячейки — полоса прокрутушки
    // и боковые колонки сюда не попадают.
    let bottom_row_dim_flags = |config: Config| -> Vec<bool> {
        let mut app = app_with(&dir.path().join("book.md"), &paragraphs(30), config);
        app.set_size(120, 10);
        app.set_scroll(20);
        let (_, buffer) = screen(&mut app, 120, 10);
        (40..80)
            .filter(|&x| buffer[(x, 5)].symbol().chars().any(|c| c.is_alphanumeric()))
            .map(|x| buffer[(x, 5)].style().add_modifier.contains(Modifier::DIM))
            .collect()
    };
    let with_fade = bottom_row_dim_flags(faded);
    let without_fade = bottom_row_dim_flags(plain);
    assert!(!with_fade.is_empty(), "строка текста занята буквами");
    assert!(with_fade.iter().all(|dim| *dim), "со включённым fade нижняя строка гаснет");
    assert!(without_fade.iter().all(|dim| !*dim), "без fade текст не приглушается");
}

fn type_command(app: &mut App, text: &str) {
    app.handle_key(crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Char(':'),
        crossterm::event::KeyModifiers::NONE,
    ));
    for c in text.chars() {
        app.handle_key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char(c),
            crossterm::event::KeyModifiers::NONE,
        ));
    }
    app.handle_key(crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Enter,
        crossterm::event::KeyModifiers::NONE,
    ));
}

#[test]
fn theme_command_lists_sets_and_reports_errors() {
    let dir = dir();
    let mut app = app_with(&dir.path().join("book.md"), &paragraphs(6), Config::default());

    type_command(&mut app, "theme list");
    let list = app.notice().expect("тост списка").to_owned();
    for preset in config::PRESETS {
        assert!(list.contains(preset), "пресет {preset} в списке: {list}");
    }

    // Сравниваем с тем же вызовом load: env-файл пользователя, если есть,
    // применяется одинаково и к команде, и к тесту.
    let expected = config::load(Some("mono")).expect("mono");
    type_command(&mut app, "theme set mono");
    assert_eq!(app.config(), &expected, "theme set mono переключил конфиг");
    assert!(app.notice().is_some_and(|n| n.contains("mono")), "тост: {:?}", app.notice());

    type_command(&mut app, "theme set neon");
    assert!(
        app.notice().is_some_and(|n| n.contains("neon")),
        "тост ошибки пресета: {:?}",
        app.notice()
    );
    assert_eq!(app.config(), &expected, "ошибка не меняет тему");

    type_command(&mut app, "theme set");
    assert!(app.notice().is_some_and(|n| n.contains("set")), "помощь по set: {:?}", app.notice());

    type_command(&mut app, "theme");
    assert!(
        app.notice().is_some_and(|n| n.contains("list")),
        "помощь по theme: {:?}",
        app.notice()
    );
}

#[test]
fn save_round_trips_the_current_config_through_a_file() {
    let dir = dir();
    let path = dir.path().join("nested/config.toml");
    let saved = Config {
        theme: Theme { key: Color::Cyan, ..Theme::mono() },
        defaults: qbook::config::Defaults {
            wheel_lines: 7,
            fade_text: false,
            ..Default::default()
        },
    };
    config::save_to(&path, &saved).expect("запись");
    let loaded = config::load_from(&path, Config::default(), true).expect("чтение");
    assert_eq!(loaded, saved, "save → load без потерь");
}
