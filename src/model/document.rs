//! Документ как последовательность rich-блоков и семантический якорь позиции.

/// Вариант, который рендерится inline: жирный, курсив и остальное.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct InlineStyle {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub sup: bool,
    pub sub: bool,
    /// Индекс в `Document.hrefs`; `None` — не ссылка.
    pub link: Option<usize>,
}

impl InlineStyle {
    /// Стиль без выделений и ссылок — обычный текст.
    pub const PLAIN: Self = Self {
        bold: false,
        italic: false,
        underline: false,
        strike: false,
        sup: false,
        sub: false,
        link: None,
    };

    pub fn is_plain(self) -> bool {
        self == Self::PLAIN
    }

    pub fn bold(mut self) -> Self {
        self.bold = true;
        self
    }

    pub fn italic(mut self) -> Self {
        self.italic = true;
        self
    }

    pub fn underline(mut self) -> Self {
        self.underline = true;
        self
    }

    pub fn strike(mut self) -> Self {
        self.strike = true;
        self
    }

    pub fn sup(mut self) -> Self {
        self.sup = true;
        self
    }

    pub fn sub(mut self) -> Self {
        self.sub = true;
        self
    }

    pub fn link(mut self, href: usize) -> Self {
        self.link = Some(href);
        self
    }
}

/// Диапазон текста с единым стилем. `\n` внутри такого спана — жёсткий перенос,
/// поставленный парсером (`<br>`); обычный спан-текст его не содержит.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub text: String,
    pub style: InlineStyle,
}

impl Span {
    pub fn new(text: impl Into<String>) -> Self {
        Self { text: text.into(), style: InlineStyle::PLAIN }
    }

    pub fn styled(text: impl Into<String>, style: InlineStyle) -> Self {
        Self { text: text.into(), style }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn style(&self) -> InlineStyle {
        self.style
    }
}

/// Тип блока. Используется и при рендере, и как признак при выравнивании перевода.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BlockKind {
    Paragraph,
    Heading(u8),
    Quote,
    ListItem,
    Code,
    Verse,
    Rule,
    Blank,
    Image,
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

/// Ссылка на ресурс картинки внутри книги.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageRef {
    /// Путь ресурса внутри контейнера (например `OEBPS/img/x.png`), нормализованный.
    pub resource: String,
    /// Текст-замена, показывается если картинка не может быть отрисована.
    pub alt: String,
    /// Пиксельный размер (ширина, высота) — из заголовка файла; рендер
    /// переводит его в высоту терминальных ячеек для раскладки.
    pub size: Option<(u32, u32)>,
}

/// Абзац документа: набор стилизованных спанов плюс оформление блока.
///
/// `text` — конкатенация текстов спанов. Это кэш совместимости: пока весь
/// остальной код (перенос, поиск, выравнивание) работает по плоскому тексту,
/// он живёт здесь; в блоке 3 перенос переедет на спаны и кэш уйдёт.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub kind: BlockKind,
    /// Конкатенация текстов спанов. Кэш совместимости — весь остальной код
    /// (перенос, поиск, выравнивание) живёт по плоскому тексту; в блоке 3
    /// перенос переедет на спаны и кэш уйдёт.
    pub text: String,
    /// Стилизованные диапазоны блока.
    pub spans: Vec<Span>,
    chars: usize,
    /// Отступ слева в колонках: вложенные списки, цитаты, стихи.
    pub indent: u8,
    /// Префикс первой строки: буллет `•`, номер `1.`, метка `›`.
    pub marker: Option<String>,
    /// Ресурс изображения, заполнен только у `BlockKind::Image`.
    pub image: Option<ImageRef>,
}

impl Block {
    /// Ведущие пробелы сохраняются: в `Verse` и `Code` отступ несёт смысл.
    /// Завершающие пробелы и переводы строк отбрасываются: на экране они не видны,
    /// а в символьных индексах только мешают.
    pub fn new(kind: BlockKind, text: impl Into<String>) -> Self {
        let text = text.into();
        let text = text.trim_end().to_owned();
        let chars = text.chars().count();
        let spans = if text.is_empty() { Vec::new() } else { vec![Span::new(text.clone())] };
        Self { kind, text, chars, spans, indent: 0, marker: None, image: None }
    }

    /// Блок из готовых стилизованных спанов: текст спанов склеивается без потерь.
    /// Концевые пробелы сохраняются — их нарезает парсер.
    pub fn styled(kind: BlockKind, spans: Vec<Span>) -> Self {
        let chars: usize = spans.iter().map(|s| s.text.chars().count()).sum();
        let text = spans.iter().map(|s| s.text.as_str()).collect();
        Self { kind, text, chars, spans, indent: 0, marker: None, image: None }
    }

    /// Отступ слева в колонках (вложенные списки, цитаты, стихи).
    pub fn with_indent(mut self, indent: u8) -> Self {
        self.indent = indent;
        self
    }

    /// Префикс первой строки блока: буллет `•`, номер `1.`, метка `›`.
    pub fn with_marker(mut self, marker: impl Into<String>) -> Self {
        self.marker = Some(marker.into());
        self
    }

    /// Ресурс изображения. Блок обязан быть `BlockKind::Image`.
    pub fn with_image(mut self, image: ImageRef) -> Self {
        debug_assert_eq!(self.kind, BlockKind::Image, "картинка на Image-блоке");
        self.image = Some(image);
        self
    }

    /// Конкатенация текстов спанов — плоский текст блока.
    pub fn flat_text(&self) -> String {
        self.spans.iter().map(|s| s.text.as_str()).collect()
    }

    pub fn char_len(&self) -> usize {
        self.chars
    }

    pub fn is_filler(&self) -> bool {
        self.kind.is_filler()
    }

    /// Присваивает готовый текст и пересчитывает кэш длины.
    /// Нужен там, где текст дописывается по частям в обход конструктора.
    /// Обновляет кэш символьной длины после дописывания текста на месте.
    pub fn set_char_len(&mut self, chars: usize) {
        self.chars = chars;
    }

    pub fn assign(&mut self, text: String) {
        let text = text.trim_end();
        self.chars = text.chars().count();
        self.text = text.to_owned();
        self.spans =
            if self.text.is_empty() { Vec::new() } else { vec![Span::new(self.text.clone())] };
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
    /// href'ы всех ссылок документа; `InlineStyle.link` — индекс сюда.
    hrefs: Vec<String>,
    toc: Vec<TocItem>,
    /// Префиксные суммы: `prefix[i]` — число символов в блоках `0..i`, длина `blocks.len() + 1`.
    prefix: Vec<usize>,
}

impl Document {
    /// Оглавление выводится из блоков-заголовков.
    pub fn new(lang: impl Into<String>, title: impl Into<String>, blocks: Vec<Block>) -> Self {
        let toc = derive_toc(&blocks);
        Self::build(lang, title, blocks, toc, Vec::new())
    }

    /// Оглавление из внешнего источника. Записи вне диапазона блоков отбрасываются,
    /// дубликаты по индексу блока схлопываются, порядок приводится к позициям блоков.
    pub fn with_toc(
        lang: impl Into<String>,
        title: impl Into<String>,
        blocks: Vec<Block>,
        toc: Vec<TocItem>,
    ) -> Self {
        Self::build(lang, title, blocks, toc, Vec::new())
    }

    /// Документ с накопленным списком ссылок.
    pub fn with_hrefs(
        lang: impl Into<String>,
        title: impl Into<String>,
        blocks: Vec<Block>,
        hrefs: Vec<String>,
    ) -> Self {
        let toc = derive_toc(&blocks);
        Self::build(lang, title, blocks, toc, hrefs)
    }

    /// Документ со внешним оглавлением и накопленным списком ссылок — путь EPUB v2.
    pub fn with_hrefs_toc(
        lang: impl Into<String>,
        title: impl Into<String>,
        blocks: Vec<Block>,
        hrefs: Vec<String>,
        toc: Vec<TocItem>,
    ) -> Self {
        Self::build(lang, title, blocks, toc, hrefs)
    }

    fn build(
        lang: impl Into<String>,
        title: impl Into<String>,
        blocks: Vec<Block>,
        toc: Vec<TocItem>,
        hrefs: Vec<String>,
    ) -> Self {
        Self {
            lang: lang.into(),
            title: title.into(),
            prefix: build_prefix(&blocks),
            toc: normalize_toc(toc, blocks.len()),
            blocks,
            hrefs,
        }
    }

    pub fn blocks(&self) -> &[Block] {
        &self.blocks
    }

    pub fn block(&self, index: usize) -> Option<&Block> {
        self.blocks.get(index)
    }

    pub fn hrefs(&self) -> &[String] {
        &self.hrefs
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
            let flat = block.spans.iter().map(|s| s.text.as_str()).collect::<String>();
            if block.spans.iter().map(|s| s.text.chars().count()).sum::<usize>() != block.chars {
                return Err(format!(
                    "блок {i}: сумма спанов {} ≠ кэшу длины {}",
                    block.spans.iter().map(|s| s.text.chars().count()).sum::<usize>(),
                    block.chars
                ));
            }
            if flat != block.text {
                return Err(format!("блок {i}: кэш текста не совпадает со спанами"));
            }
            if flat.chars().count() != block.text.chars().count() {
                return Err(format!("блок {i}: кэш длины {} неверен", block.chars));
            }
            if block.text.contains('\r') {
                return Err(format!("блок {i}: остался символ CR"));
            }
            if !block.is_filler() && block.kind != BlockKind::Image && block.text.is_empty() {
                return Err(format!("блок {i}: пустой блок типа {:?}", block.kind));
            }
            if (block.kind == BlockKind::Image) != block.image.is_some() {
                return Err(format!("блок {i}: несоответствие kind и ресурса картинки"));
            }
            for span in &block.spans {
                let Some(link) = span.style.link else { continue };
                if link >= self.hrefs.len() {
                    return Err(format!("блок {i}: ссылка {link} вне списка hrefs"));
                }
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
    fn styled_block_concatenates_spans_and_counts_chars() {
        let block = Block::styled(
            BlockKind::Paragraph,
            vec![
                Span::new("Обычный "),
                Span::styled("жирный", InlineStyle::PLAIN.bold()),
                Span::styled(" и ", InlineStyle::PLAIN),
                Span::styled("курсив", InlineStyle::PLAIN.italic()),
            ],
        );
        assert_eq!(block.text, "Обычный жирный и курсив");
        assert_eq!(block.char_len(), 23);
        assert_eq!(block.spans.len(), 4);
        assert!(block.spans[1].style().bold);
        assert!(block.spans[3].style().italic);
    }

    #[test]
    fn plain_block_has_single_plain_span() {
        let block = Block::new(BlockKind::Paragraph, "текст");
        assert_eq!(block.spans.len(), 1);
        assert!(block.spans[0].style().is_plain());
        assert!(block.marker.is_none());
        assert_eq!(block.indent, 0);
        assert!(block.image.is_none());
    }

    #[test]
    fn empty_block_has_no_spans() {
        let block = Block::new(BlockKind::Blank, "");
        assert!(block.spans.is_empty());
        assert_eq!(block.char_len(), 0);
    }

    #[test]
    fn assign_rebuilds_spans() {
        let mut block = Block::styled(
            BlockKind::Paragraph,
            vec![Span::styled("жирный", InlineStyle::PLAIN.bold())],
        );
        block.assign("новый текст".to_owned());
        assert_eq!(block.text, "новый текст");
        assert_eq!(block.spans.len(), 1);
        assert!(block.spans[0].style().is_plain(), "assign даёт плоские спаны");
    }

    #[test]
    fn image_block_carries_resource_and_alt() {
        let block = Block::new(BlockKind::Image, "").with_image(ImageRef {
            resource: "img/x.png".into(),
            alt: "подпись".into(),
            size: Some((8, 4)),
        });
        assert_eq!(block.kind, BlockKind::Image);
        let image = block.image.as_ref().expect("ресурс");
        assert_eq!(image.resource, "img/x.png");
        assert_eq!(image.alt, "подпись");
    }

    #[test]
    fn block_with_marker_and_indent() {
        let block = Block::new(BlockKind::ListItem, "элемент").with_marker("•").with_indent(2);
        assert_eq!(block.marker.as_deref(), Some("•"));
        assert_eq!(block.indent, 2);
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
        assert!(doc.hrefs().is_empty());
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
    fn document_stores_hrefs() {
        let doc = Document::with_hrefs(
            "en",
            "T",
            vec![Block::styled(
                BlockKind::Paragraph,
                vec![Span::styled("ссылка", InlineStyle::PLAIN.link(0))],
            )],
            vec!["https://example.org".into()],
        );
        assert_eq!(doc.hrefs(), ["https://example.org"]);
        assert!(doc.validate().is_ok());
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

    #[test]
    fn validate_accepts_image_block_without_alt() {
        let doc = Document::new(
            "en",
            "T",
            vec![Block::new(BlockKind::Image, "").with_image(ImageRef {
                resource: "img/a.png".into(),
                alt: String::new(),
                size: None,
            })],
        );
        assert_eq!(doc.validate(), Ok(()));
    }

    #[test]
    fn validate_rejects_image_without_resource() {
        let mut block = Block::new(BlockKind::Image, "");
        block.image = None;
        let doc = Document::new("en", "T", vec![block]);
        assert!(doc.validate().is_err());
    }

    #[test]
    fn validate_rejects_text_block_with_resource() {
        let mut block = Block::new(BlockKind::Paragraph, "hello");
        block.image =
            Some(ImageRef { resource: "img/a.png".into(), alt: String::new(), size: None });
        let doc = Document::new("en", "T", vec![block]);
        assert!(doc.validate().is_err());
    }

    #[test]
    fn validate_rejects_broken_span_cache() {
        let mut block = Block::new(BlockKind::Paragraph, "три символа");
        block.set_char_len(4);
        let doc = Document::new("en", "T", vec![block]);
        assert!(doc.validate().is_err(), "кэш длины не совпал со спанами");
    }

    #[test]
    fn validate_rejects_link_outside_hrefs() {
        let doc = Document::with_hrefs(
            "en",
            "T",
            vec![Block::styled(
                BlockKind::Paragraph,
                vec![Span::styled("ссылка", InlineStyle::PLAIN.link(0))],
            )],
            Vec::new(),
        );
        assert!(doc.validate().is_err(), "ссылка вне списка hrefs");
    }
}
