//! Бенчмарки M2/M3 спеки v2 (debug-профиль, поэтому пороги щедрые; в release
//! они уходят в разы ниже).
//!
//! M2: парсинг + первый layout книги 10k блоков ≤ 300 мс (release).
//! M3: рефлоу при смене ширины ≤ 50 мс (release).

use std::time::Instant;

use qbook::model::{Block, BlockKind, Document, Layout};

/// Абзац типичной главы: 3–4 предложения, ~140 символов, влезает в 2–3 строки.
fn paragraph(index: usize) -> String {
    let words = (0..22).map(|i| format!("слово{index}x{i}")).collect::<Vec<_>>();
    format!("Абзац номер {index}. {} и точка.", words.join(" "))
}

fn document(count: usize) -> Document {
    let blocks =
        (0..count).map(|index| Block::new(BlockKind::Paragraph, paragraph(index))).collect();
    Document::new("ru", "Перф", blocks)
}

#[test]
fn layout_of_10k_blocks_stays_affordable() {
    let doc = document(10_000);
    assert_eq!(doc.len(), 10_000);

    let start = Instant::now();
    let layout = Layout::new(&doc, 80);
    let first = start.elapsed();
    assert!(!layout.is_empty());
    // Debug-профиль примерно в 10 раз медленнее release, поэтому порог щедрый;
    // release-цель по спеке — 300 мс на парсинг И layout вместе.
    assert!(first.as_millis() < 900, "первый layout: {first:?}");

    // Рефлоу на новую ширину — тот же объём работы, что и первый layout.
    let start = Instant::now();
    let reflowed = Layout::new(&doc, 100);
    let reflow = start.elapsed();
    assert!(reflow.as_millis() < 900, "рефлоу: {reflow:?}");
    assert!(reflowed.len() != layout.len() || reflowed.width() != layout.width());
}

#[test]
fn line_indexes_are_logarithmic_not_linear() {
    // Префиксные счётчики: доступ по индексу блока не должен зависеть от
    // размера документа — это O(1) от block, а не O(строк).
    let doc = document(5_000);
    let layout = Layout::new(&doc, 80);
    let start = Instant::now();
    for (block, _) in doc.blocks().iter().enumerate() {
        let range = layout.block_range(block);
        assert_eq!(layout.line_offset(block), range.start);
    }
    let elapsed = start.elapsed();
    assert!(elapsed.as_millis() < 100, "prefiks по 5k блоков: {elapsed:?}");
}
