//! Перевод целого EPUB: читаем контейнер, переводим контент-документы, пишем новый архив.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::Path;

use zip::{CompressionMethod, ZipArchive, ZipWriter};

use super::xhtml::DocKind;
use super::{Engine, TranslateError, translate_document};
use crate::parse::epub::content_documents;

/// Итог перевода книги.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Stats {
    /// Сколько контент-документов изменено.
    pub documents: usize,
}

/// Переводит `source` в `target`, меняя только текстовые узлы контент-документов.
pub fn translate_epub(
    source: &Path,
    target: &Path,
    from: &str,
    to: &str,
    engine: &dyn Engine,
) -> Result<Stats, TranslateError> {
    let shown = source.display().to_string();
    let file = std::fs::File::open(source)
        .map_err(|source| TranslateError::Io { path: shown.clone(), source })?;
    let mut zip = ZipArchive::new(file).map_err(|error| TranslateError::Epub {
        path: shown.clone(),
        problem: error.to_string(),
    })?;

    let paths = content_documents(&mut zip).map_err(|error| TranslateError::Epub {
        path: shown.clone(),
        problem: error.to_string(),
    })?;
    let mut kinds: HashMap<String, DocKind> = HashMap::new();
    for path in &paths.body {
        kinds.insert(path.clone(), DocKind::Body);
    }
    if let Some(nav) = &paths.nav {
        kinds.insert(nav.clone(), DocKind::Nav);
    }
    if let Some(ncx) = &paths.ncx {
        kinds.insert(ncx.clone(), DocKind::Ncx);
    }

    let mut entries = read_entries(&mut zip, &shown)?;
    let mut stats = Stats::default();
    for entry in &mut entries {
        let Some(&kind) = kinds.get(&entry.name) else { continue };
        let Ok(text) = std::str::from_utf8(&entry.data) else { continue };
        let translated = translate_document(text, kind, from, to, engine)?;
        if translated != text {
            stats.documents += 1;
        }
        entry.data = translated.into_bytes();
    }

    write_archive(&entries, target)?;
    Ok(stats)
}

struct Entry {
    name: String,
    data: Vec<u8>,
    method: CompressionMethod,
    is_dir: bool,
}

fn read_entries<R: Read + std::io::Seek>(
    zip: &mut ZipArchive<R>,
    path: &str,
) -> Result<Vec<Entry>, TranslateError> {
    let mut entries = Vec::with_capacity(zip.len());
    for index in 0..zip.len() {
        let mut file = zip.by_index(index).map_err(|error| TranslateError::Epub {
            path: path.to_owned(),
            problem: error.to_string(),
        })?;
        let name = file.name().to_owned();
        let method = file.compression();
        let is_dir = file.is_dir();
        let mut data = Vec::new();
        if !is_dir {
            file.read_to_end(&mut data)
                .map_err(|source| TranslateError::Io { path: name.clone(), source })?;
        }
        entries.push(Entry { name, data, method, is_dir });
    }
    Ok(entries)
}

fn write_archive(entries: &[Entry], target: &Path) -> Result<(), TranslateError> {
    let shown = target.display().to_string();
    let file = std::fs::File::create(target)
        .map_err(|source| TranslateError::Io { path: shown, source })?;
    let mut writer = ZipWriter::new(file);

    for entry in entries {
        // `mimetype` обязан лежать первым и без сжатия — иначе EPUB не откроется.
        let method =
            if entry.name == "mimetype" { CompressionMethod::Stored } else { entry.method };
        let options = zip::write::FileOptions::<()>::default().compression_method(method);
        let result = if entry.is_dir {
            writer.add_directory(&entry.name, options)
        } else {
            writer.start_file(&entry.name, options)
        };
        result.map_err(|error| TranslateError::Epub {
            path: entry.name.clone(),
            problem: error.to_string(),
        })?;
        if !entry.is_dir {
            writer
                .write_all(&entry.data)
                .map_err(|source| TranslateError::Io { path: entry.name.clone(), source })?;
        }
    }

    writer.finish().map_err(|error| TranslateError::Epub {
        path: target.display().to_string(),
        problem: error.to_string(),
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Mutex;

    use super::*;
    use crate::translate::Engine;

    /// Движок-заглушка: переводит, оборачивая текст в угловые маркеры.
    #[derive(Default)]
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

    fn write_zip(path: &Path, parts: Vec<(&str, &str)>) {
        let file = std::fs::File::create(path).expect("создать");
        let mut zip = ZipWriter::new(file);
        let options = zip::write::FileOptions::<()>::default();
        for (name, body) in parts {
            zip.start_file(name, options).expect("запись");
            zip.write_all(body.as_bytes()).expect("данные");
        }
        zip.finish().expect("finish");
    }

    fn read_zip(path: &Path) -> Vec<(String, String)> {
        let file = std::fs::File::open(path).expect("открыть");
        let mut zip = ZipArchive::new(file).expect("архив");
        let mut out = Vec::new();
        for index in 0..zip.len() {
            let mut file = zip.by_index(index).expect("запись");
            if file.is_dir() {
                continue;
            }
            let name = file.name().to_owned();
            let mut body = String::new();
            file.read_to_string(&mut body).expect("текст");
            out.push((name, body));
        }
        out
    }

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
    <item id="css" href="style.css"/>
  </manifest>
  <spine><itemref idref="c1"/></spine>
</package>"#;

    const NAV: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><body><nav epub:type="toc"><ol>
<li><a href="c1.xhtml">Intro</a></li></ol></nav></body></html>"#;

    const CH1: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><head><title>x</title></head>
<body><h1>Intro</h1><p>Hello <em>world</em>.</p></body></html>"#;

    const CSS: &str = "body { color: red }";

    fn book(dir: &Path) -> PathBuf {
        let path = dir.join("book.epub");
        write_zip(
            &path,
            vec![
                ("mimetype", "application/epub+zip"),
                ("META-INF/container.xml", CONTAINER),
                ("OEBPS/content.opf", OPF),
                ("OEBPS/nav.xhtml", NAV),
                ("OEBPS/c1.xhtml", CH1),
                ("OEBPS/style.css", CSS),
            ],
        );
        path
    }

    #[test]
    fn translates_body_and_nav_but_not_css() {
        let dir = tempfile::tempdir().unwrap();
        let src = book(dir.path());
        let dst = dir.path().join("out.epub");

        let engine = Mark::default();
        let stats = translate_epub(&src, &dst, "en", "ru", &engine).expect("перевести");
        assert_eq!(stats.documents, 2);

        let files: HashMap<String, String> = read_zip(&dst).into_iter().collect();
        assert_eq!(files["OEBPS/style.css"], CSS);
        let chapter = &files["OEBPS/c1.xhtml"];
        assert!(chapter.contains("[Hello <em>world</em>.]"), "{chapter}");
        // Разметка и атрибуты на месте.
        assert!(chapter.contains("<em>world</em>"), "{chapter}");
        assert!(chapter.contains(r#"<html xmlns="http://www.w3.org/1999/xhtml">"#), "{chapter}");
    }

    #[test]
    fn mimetype_stays_first_and_uncompressed() {
        let dir = tempfile::tempdir().unwrap();
        let src = book(dir.path());
        let dst = dir.path().join("out.epub");
        translate_epub(&src, &dst, "en", "ru", &Mark::default()).expect("перевести");

        let file = std::fs::File::open(&dst).unwrap();
        let mut zip = ZipArchive::new(file).unwrap();
        assert_eq!(zip.by_index(0).unwrap().name(), "mimetype");
        assert_eq!(zip.by_index(0).unwrap().compression(), CompressionMethod::Stored);
    }

    #[test]
    fn all_entries_survive() {
        let dir = tempfile::tempdir().unwrap();
        let src = book(dir.path());
        let dst = dir.path().join("out.epub");
        translate_epub(&src, &dst, "en", "ru", &Mark::default()).expect("перевести");

        let mut before: Vec<String> = read_zip(&src).into_iter().map(|(name, _)| name).collect();
        let mut after: Vec<String> = read_zip(&dst).into_iter().map(|(name, _)| name).collect();
        before.sort();
        after.sort();
        assert_eq!(before, after);
    }

    #[test]
    fn missing_source_is_io_error() {
        let dir = tempfile::tempdir().unwrap();
        let dst = dir.path().join("out.epub");
        let err = translate_epub(&dir.path().join("nope.epub"), &dst, "en", "ru", &Mark::default());
        assert!(matches!(err, Err(TranslateError::Io { .. })));
    }

    /// Движок со словарём: структура книги не меняется, только слова.
    struct Swap;

    impl Engine for Swap {
        fn translate(
            &self,
            texts: &[String],
            _from: &str,
            _to: &str,
        ) -> Result<Vec<String>, TranslateError> {
            Ok(texts
                .iter()
                .map(|text| {
                    text.replace("Hello", "Привет")
                        .replace("world", "мир")
                        .replace("Intro", "Введение")
                })
                .collect())
        }
    }

    #[test]
    fn translated_book_parses_and_aligns_one_to_one() {
        let dir = tempfile::tempdir().unwrap();
        let src = book(dir.path());
        let dst = dir.path().join("out.epub");
        translate_epub(&src, &dst, "en", "ru", &Swap).expect("перевести");

        let base = crate::parse::load(&src, "en").expect("прочитать исходник");
        let translated = crate::parse::load(&dst, "ru").expect("прочитать перевод");
        assert_eq!(translated.validate(), Ok(()));
        assert_eq!(base.len(), translated.len());

        let alignment = crate::align::align(&base, &translated);
        assert_eq!(alignment.coverage(), 1.0);
    }
}
