//! Позиция: строка экрана ↔ семантический якорь.
//!
//! Метрика M4: round-trip `scroll → Anchor → scroll` возвращает ту же строку
//! (порог метрики — расхождение ≤ 1 строки, здесь проверяется строгое равенство).

use qbook::model::{
    Anchor, Block, BlockKind, Document, Layout, anchor_to_scroll, scroll_to_anchor,
};

fn sample_doc() -> Document {
    Document::new(
        "ru",
        "Тест",
        vec![
            Block::new(BlockKind::Heading(1), "Заголовок главы"),
            Block::new(
                BlockKind::Paragraph,
                "Один два три четыре пять шесть семь восемь девять десять.",
            ),
            Block::new(BlockKind::Blank, ""),
            Block::new(BlockKind::Quote, "Цитата, которая переносится на несколько строк экрана."),
            Block::new(BlockKind::Paragraph, "Короткий."),
            Block::new(BlockKind::Rule, ""),
            Block::new(
                BlockKind::Paragraph,
                "Длинный абзац с кириллицей и словами длиннее строки, чтобы переносы были жёсткими.",
            ),
        ],
    )
}

#[test]
fn round_trip_returns_the_same_line() {
    let doc = sample_doc();
    for width in [1, 3, 7, 12, 24, 40, 200] {
        let layout = Layout::new(&doc, width);
        for scroll in 0..layout.len() {
            let anchor = scroll_to_anchor(&layout, &doc, scroll);
            assert_eq!(anchor.block, layout.line(scroll).expect("строка").block, "width {width}");
            let back = anchor_to_scroll(&layout, &doc, anchor);
            assert_eq!(back, scroll, "width {width}: scroll {scroll} -> {anchor:?} -> {back}");
        }
    }
}

#[test]
fn round_trip_is_stable_for_empty_and_single_line_documents() {
    for doc in [
        Document::new("en", "T", vec![]),
        Document::new("en", "T", vec![Block::new(BlockKind::Paragraph, "одна строка")]),
        sample_doc(),
    ] {
        let layout = Layout::new(&doc, 10);
        for scroll in 0..layout.len() {
            let anchor = scroll_to_anchor(&layout, &doc, scroll);
            assert_eq!(anchor_to_scroll(&layout, &doc, anchor), scroll);
        }
    }
}

#[test]
fn frac_maps_to_the_line_that_contains_it() {
    let doc = Document::new(
        "en",
        "T",
        vec![Block::new(BlockKind::Paragraph, "один два три четыре пять")],
    );
    let layout = Layout::new(&doc, 10);
    let chars = doc.block(0).expect("блок").char_len();
    assert!(layout.block_range(0).len() > 1, "тесту нужен многострочный блок");
    for (index, line) in layout.lines().iter().enumerate() {
        let middle = (line.start_char + line.end_char) / 2;
        let anchor = Anchor::new(0, middle as f32 / chars as f32);
        assert_eq!(
            anchor_to_scroll(&layout, &doc, anchor),
            index,
            "середина строки {line:?} должна вести в неё же"
        );
    }
}

#[test]
fn single_line_block_keeps_one_line_for_any_frac() {
    let doc = Document::new(
        "en",
        "T",
        vec![
            Block::new(BlockKind::Paragraph, "короткий абзац"),
            Block::new(BlockKind::Paragraph, "второй абзац"),
        ],
    );
    let layout = Layout::new(&doc, 40);
    let expected = layout.block_range(0).start;
    for i in 0..20 {
        let frac = i as f32 / 20.0;
        assert_eq!(anchor_to_scroll(&layout, &doc, Anchor::new(0, frac)), expected, "frac {frac}");
    }
    // Фракция 1.0 — конец блока: с отступом между абзацами это blank-строка.
    assert_eq!(
        anchor_to_scroll(&layout, &doc, Anchor::new(0, 1.0)),
        layout.block_range(0).end - 1,
        "конец блока ведёт на его отступ"
    );
    assert_eq!(anchor_to_scroll(&layout, &doc, Anchor::new(1, 0.5)), layout.block_range(1).start);
}

#[test]
fn out_of_range_values_are_clamped() {
    let doc = sample_doc();
    let layout = Layout::new(&doc, 20);
    let last = layout.len() - 1;
    let anchor = scroll_to_anchor(&layout, &doc, usize::MAX);
    assert_eq!(anchor.block, doc.len() - 1);
    assert_eq!(anchor_to_scroll(&layout, &doc, anchor), last);
    // Якорь за последним блоком прижимается к концу, а не паникует.
    assert_eq!(anchor_to_scroll(&layout, &doc, Anchor::new(999, 1.0)), last);

    let empty = Document::new("en", "T", vec![]);
    let layout = Layout::new(&empty, 40);
    assert_eq!(scroll_to_anchor(&layout, &empty, 5), Anchor::START);
    assert_eq!(anchor_to_scroll(&layout, &empty, Anchor::new(3, 0.5)), 0);
}

#[test]
fn anchor_of_a_filler_block_stays_on_its_line() {
    let doc = Document::new(
        "en",
        "T",
        vec![
            Block::new(BlockKind::Paragraph, "текст"),
            Block::new(BlockKind::Rule, ""),
            Block::new(BlockKind::Paragraph, "ещё текст"),
        ],
    );
    let layout = Layout::new(&doc, 40);
    let rule_line = layout.block_range(1).start;
    let anchor = scroll_to_anchor(&layout, &doc, rule_line);
    assert_eq!((anchor.block, anchor.frac), (1, 0.0));
    assert_eq!(anchor_to_scroll(&layout, &doc, anchor), rule_line);
}
