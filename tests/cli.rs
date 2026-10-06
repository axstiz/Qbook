use std::path::{Path, PathBuf};

use clap::Parser;
use qbook::cli::{Cli, find_sidecars};

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("qbook-cli-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("создать каталог");
    dir
}

fn touch(path: &Path) {
    std::fs::write(path, "").expect("создать файл");
}

#[test]
fn parses_path_with_defaults() {
    let cli = Cli::try_parse_from(["qbook", "book.epub"]).expect("разбор");
    assert_eq!(cli.path, PathBuf::from("book.epub"));
    assert_eq!(cli.lang, "en");
    assert!(cli.variants.is_empty());
}

#[test]
fn parses_repeated_variants() {
    let cli = Cli::try_parse_from([
        "qbook",
        "b.md",
        "--variant",
        "ru=b.ru.md",
        "--variant",
        "fr=b.fr.md",
        "--lang",
        "de",
    ])
    .expect("разбор");
    assert_eq!(
        cli.variants,
        vec![
            ("ru".to_owned(), PathBuf::from("b.ru.md")),
            ("fr".to_owned(), PathBuf::from("b.fr.md")),
        ]
    );
    assert_eq!(cli.lang, "de");
}

#[test]
fn rejects_variant_without_path() {
    let error = Cli::try_parse_from(["qbook", "b.md", "--variant", "ru"])
        .expect_err("формат lang=path обязателен");
    assert!(!error.to_string().is_empty());
}

#[test]
fn rejects_missing_path() {
    Cli::try_parse_from(["qbook"]).expect_err("путь обязателен");
}

#[test]
fn sidecars_are_found_sorted_by_language() {
    let dir = temp_dir("sidecars");
    touch(&dir.join("book.md"));
    touch(&dir.join("book.ru.md"));
    touch(&dir.join("book.fr.txt"));
    touch(&dir.join("book.es.epub"));
    touch(&dir.join("readme.md"));
    touch(&dir.join("book..md"));
    touch(&dir.join("book.zip"));
    let found = find_sidecars(&dir.join("book.md"));
    assert_eq!(
        found,
        vec![
            ("es".to_owned(), dir.join("book.es.epub")),
            ("fr".to_owned(), dir.join("book.fr.txt")),
            ("ru".to_owned(), dir.join("book.ru.md")),
        ]
    );
}

#[test]
fn sidecars_require_the_same_stem() {
    let dir = temp_dir("stems");
    touch(&dir.join("book.md"));
    touch(&dir.join("books.ru.md"));
    touch(&dir.join("book-ru.md"));
    assert!(find_sidecars(&dir.join("book.md")).is_empty());
}
