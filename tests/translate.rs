//! Интеграционные тесты перевода: конвейер с журналом, resume после сбоя,
//! проверка окружения `--check`.

#![cfg(feature = "translate")]

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::thread;

use qbook::store::{Store, TranslateJobSpec, UNIT_DONE};
use qbook::translate::Engine;
use qbook::translate::TranslateError;
use qbook::translate::journal::server_ok;
use qbook::translate::task::{estimate, translate_journaled};
use zip::write::ZipWriter;

const CONTAINER: &str = r#"<?xml version="1.0"?>
<container xmlns="urn:oasis:names:tc:opendocument:xmlns:container" version="1.0">
<rootfiles><rootfile full-path="OEBPS/content.opf"/></rootfiles></container>"#;

const OPF: &str = r#"<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="uid">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>The Book</dc:title><dc:language>en</dc:language>
  </metadata>
  <manifest>
    <item id="nav" href="nav.xhtml" properties="nav"/>
    <item id="c1" href="c1.xhtml"/>
    <item id="c2" href="c2.xhtml"/>
    <item id="c3" href="c3.xhtml"/>
  </manifest>
  <spine><itemref idref="c1"/><itemref idref="c2"/><itemref idref="c3"/></spine>
</package>"#;

const NAV: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><body><nav epub:type="toc"><ol>
<li><a href="c1.xhtml">One</a></li><li><a href="c2.xhtml">Two</a></li>
</ol></nav></body></html>"#;

fn chapter(number: u8) -> String {
    format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><head><title>x</title></head>
<body><h1>Para {number} one</h1><p>Para {number} two.</p></body></html>"#
    )
}

fn write_epub(path: &Path) {
    let file = std::fs::File::create(path).unwrap();
    let mut zip = ZipWriter::new(file);
    let options = zip::write::FileOptions::<()>::default();
    for (name, body) in [
        ("mimetype", "application/epub+zip".to_owned()),
        ("META-INF/container.xml", CONTAINER.to_owned()),
        ("OEBPS/content.opf", OPF.to_owned()),
        ("OEBPS/nav.xhtml", NAV.to_owned()),
        ("OEBPS/c1.xhtml", chapter(1)),
        ("OEBPS/c2.xhtml", chapter(2)),
        ("OEBPS/c3.xhtml", chapter(3)),
    ] {
        zip.start_file(name, options).unwrap();
        zip.write_all(body.as_bytes()).unwrap();
    }
    zip.finish().unwrap();
}

fn read_zip(path: &Path) -> Vec<(String, String)> {
    let mut buffer = Vec::new();
    std::fs::File::open(path).unwrap().read_to_end(&mut buffer).unwrap();
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(buffer)).unwrap();
    let mut out = Vec::new();
    for index in 0..zip.len() {
        let mut file = zip.by_index(index).unwrap();
        if file.is_dir() {
            continue;
        }
        let name = file.name().to_owned();
        let mut text = String::new();
        file.read_to_string(&mut text).expect("читаемый entry");
        out.push((name, text));
    }
    out
}

/// Движок-заглушка: оборачивает текст в скобки и копит запросы.
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

/// Движок с обрывом: первые `limit` запросов успешны, дальше — ошибка.
struct EngineFailingAfter {
    calls: std::sync::atomic::AtomicUsize,
    limit: usize,
    seen: Mutex<Vec<String>>,
}

impl Engine for EngineFailingAfter {
    fn translate(
        &self,
        texts: &[String],
        _from: &str,
        _to: &str,
    ) -> Result<Vec<String>, TranslateError> {
        use std::sync::atomic::Ordering;
        self.seen.lock().unwrap().extend(texts.iter().cloned());
        let calls = self.calls.fetch_add(1, Ordering::Relaxed) + 1;
        if calls > self.limit {
            return Err(TranslateError::Engine("сервер выключился".to_owned()));
        }
        Ok(texts.iter().map(|text| format!("[{text}]")).collect())
    }
}

struct Harness {
    dir: tempfile::TempDir,
    src: PathBuf,
    store: Store,
    spec: TranslateJobSpec,
}

impl Harness {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        write_epub(&dir.path().join("book.epub"));
        let src = dir.path().join("book.epub");
        let store = Store::open_in(dir.path().join("journal")).unwrap();
        let spec = TranslateJobSpec {
            src_path: src.display().to_string(),
            src_hash: "h1".to_owned(),
            dst_path: dir.path().join("out.epub").display().to_string(),
            from_lang: "en".to_owned(),
            to_lang: "ru".to_owned(),
            engine: "libretranslate".to_owned(),
            base_url: "http://localhost:5000".to_owned(),
        };
        Self { dir, src, store, spec }
    }

    fn target(&self) -> PathBuf {
        self.dir.path().join("out.epub")
    }
}

const fn noop_progress(_done: usize, _total: usize) {}

/// Пуск конвейера: немного сокращает перечисление аргументов заданий.
fn run_task(
    harness: &Harness,
    engine: &(impl Engine + Sync),
    jobs: usize,
) -> Result<qbook::translate::Stats, TranslateError> {
    let mut noop = noop_progress;
    let mut task = qbook::translate::task::JobTask {
        source: &harness.src,
        target: &harness.target(),
        from: "en",
        to: "ru",
        spec: &harness.spec,
        jobs,
        progress: &mut noop,
    };
    translate_journaled(&mut task, engine, &harness.store)
}

#[test]
fn journaled_translation_writes_translated_epub() {
    let harness = Harness::new();
    let engine = Mark { seen: Mutex::new(Vec::new()) };
    let stats = run_task(&harness, &engine, 1).expect("перевод");
    // 3 главы + nav: все пять документов имеют непустые юниты.
    assert_eq!(stats.documents, 4, "статистика");
    let files: std::collections::HashMap<String, String> =
        read_zip(&harness.target()).into_iter().collect();
    let one = &files["OEBPS/c1.xhtml"];
    assert!(one.contains("[Para 1 one]"), "{one}");
    assert!(one.contains("[Para 1 two.]"), "{one}");
}

#[test]
fn estimate_counts_units_before_any_server() {
    let harness = Harness::new();
    let plan = estimate(&harness.src, 32).expect("план");
    assert!(plan.units > 0);
    assert!(plan.documents > 0);
    assert_eq!(plan.batches, plan.units.div_ceil(32));
}

#[test]
fn resume_continues_after_interruption() {
    let harness = Harness::new();
    {
        let flaky = EngineFailingAfter {
            calls: std::sync::atomic::AtomicUsize::new(0),
            limit: 1,
            seen: Mutex::new(Vec::new()),
        };
        let result = run_task(&harness, &flaky, 1);
        assert!(result.is_err(), "первый запуск сбоит");
    }
    let total = harness.store.total_translate_units(1).unwrap_or(0);
    let done = harness.store.done_translate_units(1).unwrap_or(0);
    assert!(total > 0, "юниты зарегистрированы");
    assert!(done > 0 && done < total, "прервано на середине: {done}/{total}");

    // Вторая попытка переводит только недоделки, готовые юниты не трогает.
    let engine = Mark { seen: Mutex::new(Vec::new()) };
    let stats = run_task(&harness, &engine, 1).expect("resume");
    assert_eq!(stats.documents, 4);
    let seen = engine.seen.lock().unwrap().len();
    assert!(seen < total, "resume переводит только недоделки: {seen} из {total}");

    let files: std::collections::HashMap<String, String> =
        read_zip(&harness.target()).into_iter().collect();
    assert!(files["OEBPS/c3.xhtml"].contains("[Para 3 two.]"));
    assert!(files["OEBPS/c2.xhtml"].contains("[Para 2 two.]"));
    assert!(files["OEBPS/nav.xhtml"].contains("["));
    let _ = UNIT_DONE;
}

#[test]
fn parallel_jobs_survive_multiple_documents() {
    let harness = Harness::new();
    let engine = Mark { seen: Mutex::new(Vec::new()) };
    let stats = run_task(&harness, &engine, 3).expect("перевод с 3 воркерами");
    assert_eq!(stats.documents, 4);
    let files = read_zip(&harness.target());
    for (name, text) in files {
        if name.ends_with(".xhtml") {
            assert!(text.contains("["), "{name}: нет переведённых юнитов");
        }
    }
}

fn fake_server() -> (String, thread::JoinHandle<()>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buffer = String::new();
        let mut buf = [0u8; 1024];
        loop {
            let n = stream.read(&mut buf).unwrap();
            buffer.push_str(&String::from_utf8_lossy(&buf[..n]));
            if buffer.contains("\r\n\r\n") {
                break;
            }
        }
        let body = b"{\"translatedText\":[]}";
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        stream.write_all(response.as_bytes()).unwrap();
        stream.write_all(body).unwrap();
    });
    (url, handle)
}

#[test]
fn server_ok_hits_a_local_endpoint() {
    let (url, server) = fake_server();
    server_ok(&url).expect("локальный сервер отвечает");
    server.join().unwrap();
}

#[test]
fn server_ok_reports_a_dead_endpoint() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let dead = listener.local_addr().unwrap();
    drop(listener);
    let url = format!("http://{dead}");
    server_ok(&url).expect_err("мёртвый сервер");
}

#[test]
fn error_run_leaves_journal_consistent() {
    let harness = Harness::new();
    let flaky = EngineFailingAfter {
        calls: std::sync::atomic::AtomicUsize::new(0),
        limit: 1,
        seen: Mutex::new(Vec::new()),
    };
    let result = run_task(&harness, &flaky, 1);
    assert!(result.is_err());
    let total = harness.store.total_translate_units(1).unwrap_or(0);
    let done = harness.store.done_translate_units(1).unwrap_or(0);
    let pending = harness.store.pending_translate_units(1).unwrap().len();
    assert!(total > 0);
    assert_eq!(done + pending, total, "каждый юнит либо готов, либо ждёт");
}
