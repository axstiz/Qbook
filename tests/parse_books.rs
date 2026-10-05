//! Разбор реальных книг: структура должна выживать на том, что люди читают,
//! а не только на синтетике из юнит-тестов.
//!
//! `tests/fixtures/sample-ru.epub` — реальная книга с `nav.xhtml`, `toc.ncx`,
//! вложенными списками, кодом, таблицами и картинками. Медиа вырезаны,
//! текстовые главы оставлены как есть: юнит-тесты собирают EPUB в памяти,
//! а этот файл держит разбор на настоящей разметке.

use std::path::Path;

use qbook::model::BlockKind;
use qbook::parse;

fn sample() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sample-ru.epub")
}

#[test]
fn real_epub_parses_and_validates() {
    let doc = parse::load(&sample(), "ru").expect("разобрать реальную книгу");
    assert_eq!(doc.validate(), Ok(()));
    assert!(doc.len() > 100, "подозрительно мало блоков: {}", doc.len());
    assert!(!doc.title().is_empty());
}

#[test]
fn real_epub_has_mixed_block_kinds() {
    let doc = parse::load(&sample(), "ru").expect("разобрать");
    let headings = doc.blocks().iter().filter(|b| b.kind.is_heading()).count();
    assert!(headings > 5, "заголовков всего {headings}");
    assert!(doc.toc().len() > 5, "оглавление пустое");
}

#[test]
fn real_epub_has_no_leftover_markup() {
    let doc = parse::load(&sample(), "ru").expect("разобрать");
    for block in doc.blocks() {
        let text = &block.text;
        // Теги XHTML не должны просочиться в текст, иначе выравнивание перевода
        // будет считать мусорные символы за содержание. Знаки сравнения в тексте
        // («поиск -> чтение») — часть содержания, поэтому ловим только настоящие
        // теги вида `<p` или `</p>`.
        let looks_like_tag = |needle: &str| {
            text.match_indices(needle).any(|(at, _)| {
                text[at + needle.len()..].starts_with(|c: char| c == '>' || c.is_whitespace())
            })
        };
        // В примерах кода HTML — часть содержания, а не остаток разметки.
        if !matches!(block.kind, BlockKind::Code | BlockKind::Verse) {
            for tag in ["<p", "</p", "<div", "<span", "<em", "<br", "</em", "<a"] {
                assert!(!looks_like_tag(tag), "тег {tag:?} в блоке {:?}: {text:?}", block.kind);
            }
        }
        assert!(!text.contains('\r'), "CR в блоке {text:?}");
        assert!(!text.contains("&amp;"), "неразобранный entity: {text:?}");
        assert!(!text.contains("&nbsp;"), "неразобранный entity: {text:?}");
        assert!(!text.contains("&#"), "неразобранный числовой entity: {text:?}");
        assert!(!text.contains("&lt;"), "неразобранный entity: {text:?}");
        assert!(!text.contains("&gt;"), "неразобранный entity: {text:?}");
        assert!(!text.contains('\t'), "табуляция в блоке {text:?}");
    }
}

#[test]
fn real_epub_headings_have_no_hash_prefixes() {
    let doc = parse::load(&sample(), "ru").expect("разобрать");
    for block in doc.blocks().iter().filter(|b| b.kind.is_heading()) {
        assert!(!block.text.starts_with('#'), "заголовок выглядит как markdown: {:?}", block.text);
    }
}

#[test]
fn real_epub_toc_points_at_real_blocks() {
    let doc = parse::load(&sample(), "ru").expect("разобрать");
    for item in doc.toc() {
        assert!(item.block < doc.len());
        let block = doc.block(item.block).expect("блок существует");
        assert!(
            block.kind.is_heading() || !block.text.is_empty(),
            "пустая цель оглавления: {item:?}"
        );
    }
}

#[test]
fn real_epub_blocks_have_no_repeated_whitespace_runs() {
    let doc = parse::load(&sample(), "ru").expect("разобрать");
    for block in doc.blocks() {
        if block.kind == BlockKind::Code {
            continue;
        }
        assert!(
            !block.text.contains("  "),
            "двойной пробел в блоке {:?}: {:?}",
            block.kind,
            block.text
        );
    }
}

#[test]
fn real_epub_toc_keeps_nesting() {
    let doc = parse::load(&sample(), "ru").expect("разобрать");
    // Оглавление из `nav.xhtml` вложенное: главы, разделы и подразделы.
    let levels: std::collections::BTreeSet<u8> = doc.toc().iter().map(|t| t.level).collect();
    assert!(levels.len() >= 3, "уровней оглавления всего {levels:?}");
    assert!(!doc.toc().iter().all(|t| t.level == 1), "оглавление плоское");
}

#[test]
fn real_epub_nav_page_is_not_book_text() {
    let doc = parse::load(&sample(), "ru").expect("разобрать");
    // `nav.xhtml` есть в spine почти у всех книг, но его пункты оглавления — не текст.
    // Заголовок книги в тексте при этом есть (титульный лист), поэтому сверяем списки:
    // именно в них попадают пункты `nav`.
    let list_items: Vec<&str> = doc
        .blocks()
        .iter()
        .filter(|b| b.kind == BlockKind::ListItem)
        .map(|b| b.text.as_str())
        .collect();
    for title in doc.toc().iter().map(|t| t.title.as_str()) {
        assert!(!list_items.contains(&title), "пункт оглавления попал в текст: {title:?}");
    }
}

#[test]
fn plain_text_file_parses() {
    let dir = tempfile::tempdir().expect("временный каталог");
    let path = dir.path().join("notes.md");
    std::fs::write(&path, "# Title\n\nText with **bold**.\n\n- a\n- b\n").expect("записать");

    let doc = parse::load(&path, "en").expect("разобрать текст");
    assert_eq!(doc.validate(), Ok(()));
    assert_eq!(doc.title(), "notes");
    assert_eq!(doc.blocks().len(), 4);
}

#[test]
fn unsupported_extension_is_rejected() {
    let dir = tempfile::tempdir().expect("временный каталог");
    let path = dir.path().join("book.xyz");
    std::fs::write(&path, "text").expect("записать");
    assert!(parse::load(&path, "en").is_err());
}

#[test]
fn missing_file_is_reported() {
    let err = parse::load(Path::new("/nonexistent/book.txt"), "en").expect_err("нет файла");
    assert!(err.to_string().contains("book.txt"), "{err}");
}
