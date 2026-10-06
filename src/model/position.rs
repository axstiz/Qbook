//! Переходы «строка экрана ↔ семантический якорь».

use super::{Anchor, Block, Document, Layout, LineInfo};

/// Строка на экране → семантический якорь. Якорь описывает **начало** строки:
/// при смене языка ровно эта строка остаётся на прежней высоте экрана.
///
/// Выход за границы раскладки прижимается к ближайшему краю: пустой документ
/// отдаёт `Anchor::START`, а не паникует.
pub fn scroll_to_anchor(layout: &Layout, doc: &Document, scroll: usize) -> Anchor {
    match layout.line(scroll).or_else(|| layout.lines().last()) {
        Some(line) => anchor_of(doc, line),
        None => Anchor::START,
    }
}

/// Семантический якорь → строка экрана: первая строка блока, на которой лежит
/// символ якоря. Доля `1.0` (и пустые блоки) ведёт на последнюю строку блока.
pub fn anchor_to_scroll(layout: &Layout, doc: &Document, anchor: Anchor) -> usize {
    if layout.is_empty() {
        return 0;
    }
    let block = anchor.block.min(layout.block_count().saturating_sub(1));
    let range = layout.block_range(block);
    if range.is_empty() {
        return 0;
    }
    let chars = doc.block(block).map_or(0, Block::char_len);
    let offset = match chars {
        0 => 0,
        chars => (chars as f32 * anchor.frac).round() as usize,
    };
    (range.start..range.end)
        .find(|&index| offset < layout.lines()[index].end_char)
        .unwrap_or(range.end - 1)
}

fn anchor_of(doc: &Document, line: &LineInfo) -> Anchor {
    let chars = doc.block(line.block).map_or(0, Block::char_len);
    let frac = match chars {
        0 => 0.0,
        chars => line.start_char as f32 / chars as f32,
    };
    Anchor::new(line.block, frac)
}
