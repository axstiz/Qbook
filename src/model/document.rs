//! Документ как последовательность блоков и семантический якорь позиции.

/// Тип блока. Используется и при рендере, и как признак при выравнивании перевода.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BlockKind {
    Heading(u8),
    Paragraph,
    Quote,
    ListItem,
    Code,
    Verse,
    Rule,
    Blank,
}

impl BlockKind {
    pub fn is_heading(self) -> bool {
        matches!(self, Self::Heading(_))
    }

    /// Блок без текста: рендерится разделителем и не участвует в позиционировании.
    pub fn is_filler(self) -> bool {
        matches!(self, Self::Rule | Self::Blank)
    }
}

/// Абзац документа. `chars` кэширован, потому что символьные позиции нужны постоянно,
/// а `chars().count()` по всему документу на каждом кадре — лишний O(n).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub kind: BlockKind,
    pub text: String,
    chars: usize,
}

impl Block {
    /// Ведущие пробелы сохраняются: в `Verse` и `Code` отступ несёт смысл.
    /// Завершающие пробелы и переводы строк отбрасываются: на экране они не видны,
    /// а в символьных индексах только мешают.
    pub fn new(kind: BlockKind, text: impl Into<String>) -> Self {
        let text = text.into();
        let chars = text.trim_end().chars().count();
        Self { kind, text: text.trim_end().to_owned(), chars }
    }

    pub fn char_len(&self) -> usize {
        self.chars
    }

    pub fn is_filler(&self) -> bool {
        self.kind.is_filler()
    }
}

/// Позиция в документе, не зависящая от языка отображения.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Anchor {
    pub block: usize,
    /// Доля внутри блока, диапазон `[0.0, 1.0]`.
    pub frac: f32,
}

impl Anchor {
    pub const START: Self = Self { block: 0, frac: 0.0 };

    pub fn new(block: usize, frac: f32) -> Self {
        let frac = if frac.is_nan() { 0.0 } else { frac.clamp(0.0, 1.0) };
        Self { block, frac }
    }

    pub fn at_block(block: usize) -> Self {
        Self { block, frac: 0.0 }
    }
}

impl Default for Anchor {
    fn default() -> Self {
        Self::START
    }
}

/// Запись оглавления. Для EPUB берётся из `nav`/`ncx`, для текста выводится из заголовков.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TocItem {
    pub title: String,
    pub block: usize,
    pub level: u8,
}

#[derive(Debug, Clone)]
pub struct Document {
    lang: String,
    title: String,
    blocks: Vec<Block>,
    toc: Vec<TocItem>,
    /// Префиксные суммы: `prefix[i]` — число символов в блоках `0..i`, длина `blocks.len() + 1`.
    prefix: Vec<usize>,
}

impl Document {
    /// Оглавление выводится из блоков-заголовков.
    pub fn new(lang: impl Into<String>, title: impl Into<String>, blocks: Vec<Block>) -> Self {
        let toc = derive_toc(&blocks);
        Self::with_toc(lang, title, blocks, toc)
    }

    /// Оглавление из внешнего источника. Записи вне диапазона блоков отбрасываются,
    /// дубликаты по индексу блока схлопываются, порядок приводится к позициям блоков.
    pub fn with_toc(
        lang: impl Into<String>,
        title: impl Into<String>,
        blocks: Vec<Block>,
        toc: Vec<TocItem>,
    ) -> Self {
        Self {
            lang: lang.into(),
            title: title.into(),
            prefix: build_prefix(&blocks),
            toc: normalize_toc(toc, blocks.len()),
            blocks,
        }
    }

    pub fn blocks(&self) -> &[Block] {
        &self.blocks
    }

    pub fn block(&self, index: usize) -> Option<&Block> {
        self.blocks.get(index)
    }

    pub fn lang(&self) -> &str {
        &self.lang
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn toc(&self) -> &[TocItem] {
        &self.toc
    }

    pub fn len(&self) -> usize {
        self.blocks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    pub fn total_chars(&self) -> usize {
        self.prefix[self.blocks.len()]
    }

    /// Символьное смещение начала блока; блоки за пределами документа дают `total_chars`.
    pub fn block_start(&self, block: usize) -> usize {
        self.prefix[block.min(self.blocks.len())]
    }

    /// Абсолютное смещение в символах для семантической позиции.
    pub fn char_offset(&self, anchor: Anchor) -> usize {
        let block = anchor.block.min(self.blocks.len());
        let chars = self.blocks.get(block).map_or(0, Block::char_len);
        self.block_start(block) + (chars as f32 * anchor.frac).round() as usize
    }

    /// Обратное преобразование. При нескольких подходящих блоках выбирается последний
    /// непустой — так пустые абзацы и разделители не «притягивают» позицию на себя.
    pub fn anchor_at_offset(&self, offset: usize) -> Anchor {
        if self.blocks.is_empty() {
            return Anchor::START;
        }
        let offset = offset.min(self.total_chars());
        let block = self.prefix.partition_point(|&p| p <= offset).saturating_sub(1);
        let block = block.min(self.blocks.len() - 1);
        let chars = self.blocks[block].char_len();
        let frac = match chars {
            0 => 0.0,
            chars => (offset - self.prefix[block]) as f32 / chars as f32,
        };
        Anchor::new(block, frac)
    }

    /// Доля прочитанного в диапазоне `[0.0, 1.0]`.
    pub fn progress(&self, anchor: Anchor) -> f32 {
        let total = self.total_chars();
        if total == 0 {
            return 0.0;
        }
        self.char_offset(anchor) as f32 / total as f32
    }

    /// Ближайший заголовок выше `block` — для подзаголовка в статус-баре.
    pub fn heading_at_or_before(&self, block: usize) -> Option<&TocItem> {
        self.toc.iter().rev().find(|item| item.block <= block)
    }

    /// Проверка инвариантов после разбора файла.
    pub fn validate(&self) -> Result<(), String> {
        if self.prefix.len() != self.blocks.len() + 1 {
            return Err(format!(
                "prefix имеет длину {}, а блоков {}",
                self.prefix.len(),
                self.blocks.len()
            ));
        }
        for (i, block) in self.blocks.iter().enumerate() {
            if block.chars != block.text.chars().count() {
                return Err(format!("блок {i}: кэш длины {} неверен", block.chars));
            }
            if block.text.contains('\r') {
                return Err(format!("блок {i}: остался символ CR"));
            }
            if !block.is_filler() && block.text.is_empty() {
                return Err(format!("блок {i}: пустой блок типа {:?}", block.kind));
            }
            if self.prefix[i + 1] - self.prefix[i] != block.chars {
                return Err(format!("блок {i}: префиксная сумма не сходится"));
            }
        }
        for item in &self.toc {
            if item.block >= self.blocks.len() {
                return Err(format!("оглавление: блок {} вне документа", item.block));
            }
        }
        Ok(())
    }
}

fn build_prefix(blocks: &[Block]) -> Vec<usize> {
    let mut prefix = Vec::with_capacity(blocks.len() + 1);
    prefix.push(0);
    for block in blocks {
        let last = *prefix.last().expect("префикс не пуст");
        prefix.push(last + block.char_len());
    }
    prefix
}

fn derive_toc(blocks: &[Block]) -> Vec<TocItem> {
    blocks
        .iter()
        .enumerate()
        .filter_map(|(i, block)| {
            let BlockKind::Heading(level) = block.kind else { return None };
            Some(TocItem { title: block.text.clone(), block: i, level })
        })
        .collect()
}

fn normalize_toc(toc: Vec<TocItem>, block_count: usize) -> Vec<TocItem> {
    let mut toc: Vec<TocItem> = toc.into_iter().filter(|i| i.block < block_count).collect();
    toc.sort_by_key(|i| i.block);
    toc.dedup_by_key(|i| i.block);
    toc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_counts_chars_not_bytes() {
        let block = Block::new(BlockKind::Paragraph, "привет");
        assert_eq!(block.char_len(), 6);
        assert_eq!(block.text.len(), 12);
    }

    #[test]
    fn block_trims_trailing_whitespace_only() {
        let block = Block::new(BlockKind::Verse, "  строфа\n\n  \n");
        assert_eq!(block.text, "  строфа");
        assert_eq!(block.char_len(), 8);
    }

    #[test]
    fn anchor_clamps_fraction() {
        assert_eq!(Anchor::new(3, -0.5).frac, 0.0);
        assert_eq!(Anchor::new(3, 1.5).frac, 1.0);
        assert_eq!(Anchor::new(3, 0.25).frac, 0.25);
        assert_eq!(Anchor::new(3, f32::NAN).frac, 0.0);
    }

    #[test]
    fn document_prefix_and_offsets() {
        let doc = Document::new(
            "en",
            "T",
            vec![
                Block::new(BlockKind::Heading(1), "Title"),
                Block::new(BlockKind::Paragraph, "hello"),
                Block::new(BlockKind::Paragraph, "world"),
            ],
        );
        assert_eq!(doc.total_chars(), 15);
        assert_eq!(doc.block_start(0), 0);
        assert_eq!(doc.block_start(2), 10);
        assert_eq!(doc.char_offset(Anchor::new(2, 0.4)), 12);
        assert_eq!(doc.char_offset(Anchor::new(2, 0.5)), 13); // round(2.5) = 3
        assert_eq!(doc.progress(Anchor::new(3, 0.0)), 1.0);
    }

    #[test]
    fn offset_to_anchor_round_trip() {
        let doc = Document::new(
            "en",
            "T",
            vec![
                Block::new(BlockKind::Paragraph, "hello"),
                Block::new(BlockKind::Paragraph, "мир"),
            ],
        );
        for offset in 0..=doc.total_chars() {
            let anchor = doc.anchor_at_offset(offset);
            assert_eq!(doc.char_offset(anchor), offset, "offset {offset}");
        }
    }

    #[test]
    fn anchor_at_offset_skips_leading_blank() {
        let doc = Document::new(
            "en",
            "T",
            vec![Block::new(BlockKind::Blank, ""), Block::new(BlockKind::Paragraph, "abc")],
        );
        assert_eq!(doc.anchor_at_offset(0), Anchor::new(1, 0.0));
    }

    #[test]
    fn empty_document_is_safe() {
        let doc = Document::new("en", "T", vec![]);
        assert!(doc.is_empty());
        assert_eq!(doc.total_chars(), 0);
        assert_eq!(doc.char_offset(Anchor::new(99, 0.5)), 0);
        assert_eq!(doc.anchor_at_offset(42), Anchor::START);
        assert_eq!(doc.progress(Anchor::START), 0.0);
        assert!(doc.validate().is_ok());
    }

    #[test]
    fn out_of_range_block_clamps_to_end() {
        let doc = Document::new("en", "T", vec![Block::new(BlockKind::Paragraph, "abc")]);
        assert_eq!(doc.block_start(50), 3);
        assert_eq!(doc.char_offset(Anchor::new(50, 1.0)), 3);
    }

    #[test]
    fn toc_is_derived_from_headings() {
        let doc = Document::new(
            "en",
            "T",
            vec![
                Block::new(BlockKind::Paragraph, "intro"),
                Block::new(BlockKind::Heading(2), "First"),
                Block::new(BlockKind::Paragraph, "text"),
                Block::new(BlockKind::Heading(1), "Second"),
            ],
        );
        assert_eq!(
            doc.toc(),
            [
                TocItem { title: "First".into(), block: 1, level: 2 },
                TocItem { title: "Second".into(), block: 3, level: 1 },
            ]
        );
        assert_eq!(doc.heading_at_or_before(2).map(|i| i.block), Some(1));
        assert_eq!(doc.heading_at_or_before(0), None);
    }

    #[test]
    fn external_toc_is_sorted_and_deduplicated() {
        let doc = Document::new(
            "en",
            "T",
            vec![Block::new(BlockKind::Paragraph, "a"), Block::new(BlockKind::Paragraph, "b")],
        );
        let toc = vec![
            TocItem { title: "вне".into(), block: 9, level: 1 },
            TocItem { title: "second".into(), block: 1, level: 1 },
            TocItem { title: "first".into(), block: 0, level: 1 },
            TocItem { title: "dup".into(), block: 0, level: 1 },
        ];
        let doc = Document::with_toc("en", "T", doc.blocks().to_vec(), toc);
        let titles: Vec<&str> = doc.toc().iter().map(|i| i.title.as_str()).collect();
        assert_eq!(titles, ["first", "second"]);
    }

    #[test]
    fn validate_accepts_well_formed_document() {
        let doc = Document::new(
            "en",
            "T",
            vec![
                Block::new(BlockKind::Heading(1), "Title"),
                Block::new(BlockKind::Paragraph, "hello"),
                Block::new(BlockKind::Blank, ""),
                Block::new(BlockKind::Rule, ""),
            ],
        );
        assert_eq!(doc.validate(), Ok(()));
    }

    #[test]
    fn validate_rejects_empty_textual_block() {
        let doc = Document::new("en", "T", vec![Block::new(BlockKind::Paragraph, "   ")]);
        assert!(doc.validate().is_err());
    }
}
