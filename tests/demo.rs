//! Демо-набор (п. 25 плана): пары должны парситься, а выравнивание — почти идеальным.

use std::path::{Path, PathBuf};

use qbook::align::align;
use qbook::model::Document;
use qbook::parse;

fn demo(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("demo").join(name)
}

fn assert_pair_aligns(base: &Document, variant: &Document) {
    assert_eq!(base.blocks().len(), variant.blocks().len(), "одинаковая структура абзацев");
    let alignment = align(base, variant);
    assert!(alignment.coverage() > 0.95, "покрытие выравнивания: {:.2}", alignment.coverage());
}

#[test]
fn demo_markdown_pair_is_translatable_in_place() {
    let base = parse::load(&demo("moon.md"), "en").expect("база");
    let variant = parse::load(&demo("moon.ru.md"), "ru").expect("перевод");
    assert_eq!(base.title(), "moon");
    assert_pair_aligns(&base, &variant);
}

#[test]
fn demo_epub_pair_is_translatable_in_place() {
    let base = parse::load(&demo("moon.epub"), "en").expect("база");
    let variant = parse::load(&demo("moon.ru.epub"), "ru").expect("перевод");
    assert_eq!(base.title(), "The Moon");
    assert_eq!(variant.title(), "Луна");
    assert_pair_aligns(&base, &variant);
}
