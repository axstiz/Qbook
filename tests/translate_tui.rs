//! `:translate` в TUI: фоновой запуск, панель прогресса, тост, rescan сайдкара.

#![cfg(feature = "translate")]

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use qbook::app::App;
use qbook::store::Store;
use qbook::translate::Engine;
use qbook::translate::TranslateError;

mod common;
use common::write_mini_epub;

struct Mark {
    seen: Mutex<Vec<String>>,
}

impl Engine for Mark {
    fn translate(
        &self,
        texts: &[String],
        _from: &str,
        _to: &str,
    ) -> Result<Vec<String>, TranslateError> {
        self.seen.lock().unwrap().extend(texts.iter().cloned());
        Ok(texts.iter().map(|text| format!("[{text}]")).collect())
    }
}

struct Fail;

impl Engine for Fail {
    fn translate(
        &self,
        _texts: &[String],
        _from: &str,
        _to: &str,
    ) -> Result<Vec<String>, TranslateError> {
        Err(TranslateError::Engine("сервер выключился".to_owned()))
    }
}

/// Медленный движок: конвейер висит в работе для рендера панели.
struct Slow {
    pause: Duration,
}

impl Engine for Slow {
    fn translate(
        &self,
        texts: &[String],
        _from: &str,
        _to: &str,
    ) -> Result<Vec<String>, TranslateError> {
        std::thread::sleep(self.pause);
        Ok(texts.iter().map(|text| format!("[{text}]")).collect())
    }
}

const CHAPTER: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><body><p>One.</p><p>Two.</p></body></html>"#;

fn setup() -> (tempfile::TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    let book = dir.path().join("book.epub");
    write_mini_epub(&book, &[CHAPTER.to_owned()]);
    let store = Store::open_in(dir.path().join("data")).unwrap();
    let app = App::load(&book, "en", &[], Some(store)).expect("открытие книги");
    (dir, app)
}

/// Ждём завершения фоновой задачи тиками. true — завершилась за предел.
fn wait_job(app: &mut App) -> bool {
    let deadline = Instant::now() + Duration::from_secs(5);
    while app.translate_progress().is_some() {
        app.tick().expect("тик");
        if Instant::now() > deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    true
}

#[test]
fn translate_job_finishes_and_rescans_variant() {
    let (_dir, mut app) = setup();
    app.start_translate_with("ru", Arc::new(Mark { seen: Mutex::new(Vec::new()) })).expect("старт");
    assert!(app.translate_progress().is_some(), "задача в работе");
    assert!(wait_job(&mut app), "задача завершилась");
    let notice = app.notice().unwrap_or_default();
    assert!(notice.contains("перевод завершён"), "{notice}");

    // Сайдкар book.ru.epub стал вариантом: переключение обязано сработать.
    let switched = app.run_command(":lang ru");
    assert_eq!(switched.as_deref(), Some(": язык · ru"));
    assert_eq!(app.current_lang(), "ru");
}

#[test]
fn failed_translate_shows_toast_without_variant() {
    let (_dir, mut app) = setup();
    app.start_translate_with("ru", Arc::new(Fail)).expect("старт");
    assert!(wait_job(&mut app), "задача завершилась");
    let notice = app.notice().unwrap_or_default();
    assert!(notice.contains("перевод не удался"), "{notice}");
    assert!(notice.contains("сервер выключился"), "{notice}");
    // Языка ru в списке вариантов нет.
    assert_eq!(app.run_command(":lang ru").as_deref(), Some(": нет языка «ru»"));
}

#[test]
fn refuse_double_start_base_lang_and_non_epub() {
    let (dir, mut app) = setup();
    let reply = app.run_command(":translate ru").expect("старт");
    assert!(reply.contains("перевод → ru"), "{reply}");
    let second = app.run_command(":translate de").expect("проверка");
    assert!(second.contains("перевод уже идёт"), "{second}");
    assert!(wait_job(&mut app), "задача завершилась");

    let third = app.run_command(":translate en").expect("проверка");
    assert!(third.contains("исходный язык"), "{third}");

    let empty = app.run_command(":translate");
    assert_eq!(empty.as_deref(), Some(": нужен язык: :translate ru"));

    // TXT не переводится.
    let txt = dir.path().join("book.ru.txt");
    std::fs::write(&txt, "привет").unwrap();
    let txt_store = Store::open_in(dir.path().join("data2")).unwrap();
    let mut txt_app = App::load(&txt, "ru", &[], Some(txt_store)).expect("txt открыт");
    let reply = txt_app.run_command(":translate de").expect("проверка");
    assert!(reply.contains("только EPUB"), "{reply}");
}

#[test]
fn progress_panel_renders_while_job_runs() {
    let dir = tempfile::tempdir().unwrap();
    let book = dir.path().join("book.epub");
    write_mini_epub(&book, &[CHAPTER.to_owned()]);
    let app_store = Store::open_in(dir.path().join("data")).unwrap();
    let mut app = App::load(&book, "en", &[], Some(app_store)).expect("открытие книги");
    app.set_size(80, 10);
    app.start_translate_with("ru", Arc::new(Slow { pause: Duration::from_millis(300) }))
        .expect("старт");
    let mut terminal =
        ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 10)).expect("терминал");
    terminal.draw(|frame| qbook::ui::reader::render(&mut app, frame)).expect("отрисовка");
    let buffer = terminal.backend().buffer().clone();
    let slot_y = 8;
    let line: String = (0..80).map(|x| buffer[(x, slot_y)].symbol()).collect();
    assert!(line.contains("перевод → ru"), "панель прогресса: {line}");
    assert!(line.contains("юнитов"), "счётчик юнитов: {line}");
    std::thread::sleep(Duration::from_millis(400));
}
