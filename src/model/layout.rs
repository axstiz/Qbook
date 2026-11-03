//! Раскладка документа в терминальные строки.
//!
//! Все индексы — в символах, не в байтах: иначе кириллица и эмодзи ломают
//! переходы между строками. Диапазоны строк блока идут подряд и покрывают весь
//! текст блока — на этом держится round-trip `scroll → Anchor → scroll`.
//! После непустого блока (кроме последнего) идёт отступ — blank-строка с
//! пустым диапазоном, она принадлежит своему блоку.
//!
//! v2: перенос — однопроходный, без промежуточных ячеек; каждая строка кроме
//! диапазона несёт стили спанов, пересекающих её границы, и раскладка хранит
//! префиксные счётчики строк блоков (`anchor↔scroll` — O(log n)).

use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::{Block, Document, InlineStyle};

/// Одна строка на экране: диапазон символов `[start_char, end_char)` внутри блока.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineInfo {
    pub block: usize,
    pub line_in_block: usize,
    pub start_char: usize,
    pub end_char: usize,
    /// Стили спанов, чей текст пересекает строку: (символьный диапазон в
    /// тексте блока, стиль). Клампится к строке, пустых диапазонов нет.
    pub styles: Vec<(Range<usize>, InlineStyle)>,
}

impl LineInfo {
    /// Текст строки. Конец включает хвостовые пробелы и перевод строки — их
    /// обрезает рендер, а диапазоны остаются сплошными и ничего не теряют.
    pub fn slice<'a>(&self, block: &'a Block) -> &'a str {
        slice_range(&block.text, self.start_char, self.end_char)
    }
}

fn slice_range(text: &str, start_char: usize, end_char: usize) -> &str {
    fn byte_at(text: &str, chars: usize) -> usize {
        match text.char_indices().nth(chars) {
            Some((byte, _)) => byte,
            None => text.len(),
        }
    }
    &text[byte_at(text, start_char)..byte_at(text, end_char)]
}

/// Раскладка документа при заданной ширине: линии, счётчики строк блоков и их
/// префиксные суммы. Строится целиком при создании (перенос однопроходный), а
/// оконный вариант приходит вместе с рендером v2.
#[derive(Debug, Clone)]
pub struct Layout {
    width: u16,
    lines: Vec<LineInfo>,
    /// Префиксные суммы строк блоков: `line_offsets[i]` — строковый индекс начала блока `i`.
    line_offsets: Vec<usize>,
    block_ranges: Vec<Range<usize>>,
}

impl Layout {
    pub fn new(doc: &Document, width: u16) -> Self {
        // Нулевая ширина зациклила бы перенос: рисуем строку в один символ.
        let width = width.max(1);
        let cells = usize::from(width);
        let mut lines: Vec<LineInfo> = Vec::new();
        let mut counts = Vec::with_capacity(doc.len() + 1);
        let mut block_ranges: Vec<Range<usize>> = Vec::with_capacity(doc.len());
        counts.push(0);
        let last = doc.len();
        for (block, item) in doc.blocks().iter().enumerate() {
            let start = lines.len();
            let shared = style_ranges(item);
            let wrapped = wrap(&item.text, cells);
            let mut gap = false;
            for (line_in_block, range) in wrapped.into_iter().enumerate() {
                lines.push(LineInfo {
                    block,
                    line_in_block,
                    start_char: range.start,
                    end_char: range.end,
                    styles: intersect_styles(&shared, range.clone()),
                });
                gap = range.start < range.end;
            }
            // Отступ между абзацами: пустая строка после блока, чья последняя
            // строка непуста. Пустые блоки и блоки с концевым \n её не получают —
            // иначе она неотличима от их собственных строк и round-trip ломается.
            let is_gap = gap && block + 1 < last;
            if is_gap {
                let char_len = item.char_len();
                lines.push(LineInfo {
                    block,
                    line_in_block: lines.len() - start,
                    start_char: char_len,
                    end_char: char_len,
                    styles: Vec::new(),
                });
            }
            block_ranges.push(start..lines.len());
            let next = *counts.last().expect("префикс не пуст") + lines.len() - start;
            counts.push(next);
        }

        Self { width, lines, line_offsets: counts, block_ranges }
    }

    /// Строки блока от начала документа. Бинарный поиск вместо итерации.
    pub fn line(&self, index: usize) -> Option<&LineInfo> {
        self.lines.get(index)
    }

    pub fn width(&self) -> u16 {
        self.width
    }

    pub fn lines(&self) -> &[LineInfo] {
        &self.lines
    }

    /// Число блоков в раскладке: они идут в том же порядке, что и в документе.
    pub fn block_count(&self) -> usize {
        self.block_ranges.len()
    }

    pub fn len(&self) -> usize {
        *self.line_offsets.last().unwrap_or(&0)
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// Строки блока. Для несуществующего блока возвращается пустой диапазон.
    pub fn block_range(&self, block: usize) -> Range<usize> {
        self.block_ranges.get(block).cloned().unwrap_or(0..0)
    }

    /// Префиксные позиции: смещение строки блока в общей раскладке.
    pub fn line_offset(&self, block: usize) -> usize {
        self.line_offsets[block.min(self.line_offsets.len() - 1)]
    }

    /// Строковый индекс блока, содержащего строку `index`, бинарным поиском.
    pub fn block_at_line(&self, index: usize) -> usize {
        self.block_ranges.partition_point(|range| range.end <= index).saturating_sub(1)
    }
}

/// Символьные диапазоны стилей блока: каждый спан даёт свой непрерывный кусок.
pub fn style_ranges(block: &Block) -> Vec<(Range<usize>, InlineStyle)> {
    let mut out = Vec::with_capacity(block.spans.len());
    let mut offset = 0;
    for span in &block.spans {
        let len = span.text.chars().count();
        if len > 0 {
            out.push((offset..offset + len, span.style));
        }
        offset += len;
    }
    out
}

/// Пересечение стилей блока с диапазоном строки (границы клампятся).
fn intersect_styles(
    shared: &[(Range<usize>, InlineStyle)],
    line: Range<usize>,
) -> Vec<(Range<usize>, InlineStyle)> {
    if shared.is_empty() {
        return Vec::new();
    }
    shared
        .iter()
        .filter_map(|(range, style)| {
            let start = line.start.max(range.start);
            let end = line.end.min(range.end);
            (start < end).then_some((start..end, *style))
        })
        .collect()
}

/// Превращает текст блока в диапазоны символов: `[start, end)` каждой строки.
///
/// `\n` — жёсткая граница, он сам входит в предыдущую строку, чтобы диапазоны
/// покрывали весь текст без разрывов.
fn wrap(text: &str, width: usize) -> Vec<Range<usize>> {
    let mut lines = Vec::new();
    let segments: Vec<&str> = text.split('\n').collect();
    let mut base = 0;
    for (i, segment) in segments.iter().enumerate() {
        let has_newline = i + 1 < segments.len();
        let segment_end = base + segment.chars().count() + usize::from(has_newline);
        let wrapped = wrap_segment(segment, width);
        let last = wrapped.len() - 1;
        for (n, mut range) in wrapped.into_iter().enumerate() {
            range.start += base;
            range.end += base;
            if has_newline && n == last {
                range.end = segment_end;
            }
            lines.push(range);
        }
        base = segment_end;
    }
    lines
}

/// Перенос одного сегмента без переводов строк. Индексы относительно сегмента.
///
/// Однопроходный скан: строка заканчивается на последней точке переноса
/// (пробел, дефис, слэш, CJK); слово длиннее строки режется посреди; серии
/// хвостовых пробелов остаются в своей строке — так сохраняется покрытие
/// диапазонов без разрывов.
fn wrap_segment(text: &str, width: usize) -> Vec<Range<usize>> {
    let total = text.chars().count();
    if text_has_composites(text) {
        let steps = text.graphemes(true).map(|grapheme| {
            let len = grapheme.chars().count();
            let w = match len {
                1 => {
                    let ch = grapheme.chars().next().expect("графема не пуста");
                    if matches!(ch, ' ' | '\t') {
                        1
                    } else {
                        UnicodeWidthChar::width(ch).unwrap_or(1)
                    }
                }
                _ => UnicodeWidthStr::width(grapheme),
            };
            let is_space = grapheme.chars().all(char::is_whitespace)
                && !grapheme.contains('\u{00A0}')
                && !grapheme.contains('\u{202F}');
            let breakable = is_space
                || matches!(grapheme.chars().next_back(), Some('-') | Some('/'))
                || grapheme.chars().next().is_some_and(is_cjk);
            Step { len, w, is_space, breakable }
        });
        return wrap_steps(total, width, steps);
    }
    let steps = text.chars().map(|ch| {
        let w = if matches!(ch, ' ' | '\t') { 1 } else { UnicodeWidthChar::width(ch).unwrap_or(1) };
        Step { len: 1, w, is_space: char_is_space(ch), breakable: char_breakable(ch) }
    });
    wrap_steps(total, width, steps)
}

/// Плейсхолдер атрибутов одного шага переноса: графема или одиночный символ.
struct Step {
    len: usize,
    w: usize,
    is_space: bool,
    breakable: bool,
}

/// Серии шагов → строковые диапазоны; общая для fast- и графем-путей.
fn wrap_steps(total: usize, width: usize, steps: impl Iterator<Item = Step>) -> Vec<Range<usize>> {
    let mut lines = Vec::new();
    if total == 0 {
        lines.push(0..0);
        return lines;
    }
    let mut line_start = 0usize;
    let mut used = 0usize;
    // Точка переноса: `(конец_шага, ширина_этой_части)`.
    let mut last_break: Option<(usize, usize)> = None;
    let mut at = 0usize;
    for step in steps {
        let Step { len, w, is_space, breakable } = step;
        if is_space && used + w > width && used > 0 {
            // Пробельная серия не влезает: линия включает весь её хвост, новая
            // начинается ровно после него (хвост обеих тримит рендер, диапазоны
            // остаются сплошными).
            lines.push(line_start..at + len);
            line_start = at + len;
            used = 0;
            last_break = None;
            at += len;
            continue;
        }
        if !is_space && used + w > width {
            if let Some((point, used_at_break)) =
                last_break.filter(|&(point, _)| point > line_start)
            {
                lines.push(line_start..point);
                line_start = point;
                // `point..at` — начало слова, уехавшее в следующую строку.
                used = used - used_at_break + w;
                last_break = None;
                at += len;
                continue;
            }
            if at == line_start {
                // Шаг шире строки: ставим его и идём дальше.
                lines.push(line_start..at + len);
                line_start = at + len;
                used = 0;
                at += len;
                continue;
            }
            // Точек переноса не было — режем посреди слова.
            lines.push(line_start..at);
            line_start = at;
            used = w;
            last_break = None;
            at += len;
            continue;
        }
        used += w;
        let end = at + len;
        if breakable {
            last_break = Some((end, used));
        }
        at = end;
    }
    if line_start < total {
        lines.push(line_start..total);
    }
    lines
}

/// Признаки составных графем: сканирование эквивалентно полному пути, только
/// медленнее. Основные комбинации — скан-детект: любые они уводят текст в
/// графемный путь.
fn text_has_composites(text: &str) -> bool {
    text.chars().any(|ch| {
        matches!(ch,
            '\u{200C}' | '\u{200D}'                  // ZWNJ, ZWJ (emoji-композиты)
            | '\u{FE00}'..='\u{FE0F}'                // variation selectors
            | '\u{0300}'..='\u{036F}'                // диакритики
            | '\u{0483}'..='\u{0489}'                // кириллические йоговые
            | '\u{0591}'..='\u{05C7}'                // еврейские огласовки
            | '\u{0610}'..='\u{061A}' | '\u{064B}'..='\u{065F}'
            | '\u{093A}'..='\u{094F}' | '\u{0951}'..='\u{0957}' | '\u{0962}'..='\u{0963}'
            | '\u{0E00}'..='\u{0E7F}'                // тайские знаки
            | '\u{1100}'..='\u{11FF}'                // хангыль-джамо
            | '\u{1AB0}'..='\u{1AFF}' | '\u{1DC0}'..='\u{1DFF}'
            | '\u{20D0}'..='\u{20F0}'                 // combining marks, keycap-плац
            | '\u{1F3FB}'..='\u{1F3FF}'              // тоны кожи
        )
    })
}

fn char_is_space(ch: char) -> bool {
    matches!(ch, ' ' | '\t' | '\n' | '\r' | '\u{000B}' | '\u{000C}')
        || (ch.is_whitespace() && ch != '\u{00A0}' && ch != '\u{202F}')
}

fn char_breakable(ch: char) -> bool {
    char_is_space(ch) || matches!(ch, '-' | '/') || is_cjk(ch)
}

/// Китайский, японский, корейский и прочие заслонные плоскости.
fn is_cjk(c: char) -> bool {
    matches!(
        c,
        '\u{2E80}'..='\u{9FFF}'
            | '\u{A960}'..='\u{A97F}'
            | '\u{AC00}'..='\u{D7FF}'
            | '\u{F900}'..='\u{FAFF}'
            | '\u{FE30}'..='\u{FE4F}'
            | '\u{FF00}'..='\u{FFEF}'
            | '\u{20000}'..='\u{3FFFD}'
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Block, BlockKind, Document, Span};

    fn paragraph(text: &str) -> Document {
        Document::new("en", "T", vec![Block::new(BlockKind::Paragraph, text)])
    }

    /// Строки так, как их видит читатель: без хвостовых пробелов и переводов.
    fn rendered(doc: &Document, width: u16) -> Vec<String> {
        let layout = Layout::new(doc, width);
        layout
            .lines()
            .iter()
            .map(|line| line.slice(doc.block(line.block).expect("блок")).trim_end().to_owned())
            .collect()
    }

    #[test]
    fn wraps_at_spaces_without_breaking_words() {
        let doc = paragraph("hello world");
        assert_eq!(rendered(&doc, 7), ["hello", "world"]);
    }

    #[test]
    fn breaks_after_hyphen_and_slash() {
        assert_eq!(rendered(&paragraph("foo-bar"), 5), ["foo-", "bar"]);
        assert_eq!(rendered(&paragraph("a/b/c"), 2), ["a/", "b/", "c"]);
    }

    #[test]
    fn cjk_wraps_character_by_character() {
        let doc = paragraph("漢字漢字漢");
        assert_eq!(rendered(&doc, 3), ["漢", "字", "漢", "字", "漢"]);
    }

    #[test]
    fn indices_count_chars_not_bytes() {
        let doc = paragraph("привет мир");
        let layout = Layout::new(&doc, 5);
        let block = doc.block(0).expect("блок");
        let texts: Vec<&str> = layout.lines().iter().map(|line| line.slice(block)).collect();
        // Индексы в символах: у «привет» шесть символов, а байт в два раза больше.
        assert_eq!(texts, ["приве", "т мир"]);
        assert_eq!(layout.lines()[1].start_char, 5);
        assert_eq!(layout.lines()[1].end_char, 10);
    }

    #[test]
    fn word_longer_than_width_is_split() {
        let doc = paragraph("supercalifragilistic");
        assert_eq!(rendered(&doc, 6), ["superc", "alifra", "gilist", "ic"]);
    }

    #[test]
    fn emoji_stays_whole() {
        // Эмодзи — одна графема из нескольких кодпоинтов: резать её нельзя.
        assert_eq!(rendered(&paragraph("😀😀"), 3), ["😀", "😀"]);
    }

    #[test]
    fn wide_grapheme_over_width_is_kept() {
        let doc = paragraph("🗻x");
        assert_eq!(rendered(&doc, 1), ["🗻", "x"]);
    }

    #[test]
    fn trailing_spaces_do_not_take_width() {
        assert_eq!(rendered(&paragraph("aa  bb"), 3), ["aa", "bb"]);
    }

    fn styled_block(_text: &str, bold_at: Range<usize>) -> Document {
        // «Раз» + [bold] «жир» + «ный» — жирный по диапазону символов.
        let _ = bold_at;
        Document::new(
            "en",
            "T",
            vec![Block::styled(
                BlockKind::Paragraph,
                vec![Span::new("Раз "), Span::styled("жирный", InlineStyle::PLAIN.bold())],
            )],
        )
    }

    #[test]
    fn styled_spans_land_on_lines() {
        let doc = styled_block("Раз жирный", 4..10);
        let layout = Layout::new(&doc, 10);
        let line0 = &layout.lines()[0];
        assert_eq!(line0.end_char, 10, "вся строка на одной линии");
        assert_eq!(line0.styles.len(), 2, "plain у «Раз » и bold у «жирный»");
        assert_eq!(line0.styles[0].0, 0..4);
        assert!(!line0.styles[0].1.bold);
        assert_eq!(line0.styles[1].0, 4..10);
        assert!(line0.styles[1].1.bold);
    }

    #[test]
    fn styles_are_clamped_to_line_fallback() {
        let doc = Document::new(
            "en",
            "T",
            vec![Block::styled(
                BlockKind::Paragraph,
                vec![
                    Span::styled("fatty", InlineStyle::PLAIN.bold()),
                    Span::styled("plain", InlineStyle::PLAIN),
                ],
            )],
        );
        let layout = Layout::new(&doc, 3);
        let line0 = &layout.lines()[0];
        assert_eq!(line0.start_char, 0);
        assert_eq!(line0.end_char, 3);
        assert_eq!(line0.styles.len(), 1, "жирный клампится к 0..3");
        assert_eq!(line0.styles[0].0, 0..3);
        let line1 = &layout.lines()[1];
        assert_eq!(line1.styles.len(), 2, "хвост жирного и plain на второй строке");
        assert!(line1.styles[0].1.bold);
        assert!(!line1.styles[1].1.bold);
    }

    #[test]
    fn line_offsets_are_prefix_sums() {
        let doc = Document::new(
            "en",
            "T",
            vec![
                Block::new(BlockKind::Paragraph, "первый"),
                Block::new(BlockKind::Paragraph, "второй абзац"),
            ],
        );
        let layout = Layout::new(&doc, 40);
        assert_eq!(layout.line_offset(0), 0);
        assert_eq!(layout.line_offset(1), 2, "первый блок: строка + отступ");
        assert_eq!(layout.line_offset(2), 3, "последний блок без хвостового отступа");
        assert_eq!(layout.block_range(1).start, layout.line_offset(1));
    }

    #[test]
    fn block_at_line_finds_own_block() {
        let doc = paragraph("один\nдва");
        let layout = Layout::new(&doc, 40);
        assert_eq!(layout.block_at_line(0), 0);
        assert_eq!(layout.block_at_line(2), 0, "отступ принадлежит своему блоку");
    }

    #[test]
    fn filler_blocks_take_one_line() {
        let doc = Document::new(
            "en",
            "T",
            vec![
                Block::new(BlockKind::Rule, ""),
                Block::new(BlockKind::Blank, ""),
                Block::new(BlockKind::Paragraph, "hi"),
            ],
        );
        let layout = Layout::new(&doc, 10);
        assert_eq!(layout.block_range(0), 0..1);
        assert_eq!(layout.block_range(1), 1..2);
        assert_eq!(layout.block_range(2), 2..3);
        let filler = layout.lines()[0].clone();
        assert_eq!((filler.start_char, filler.end_char), (0, 0));
    }

    #[test]
    fn newline_ends_the_line() {
        let doc = Document::new("en", "T", vec![Block::new(BlockKind::Code, "ab\ncd")]);
        let layout = Layout::new(&doc, 10);
        assert_eq!(layout.len(), 2);
        // Перевод строки входит в предыдущую строку: диапазоны не разрывятся.
        assert_eq!((layout.lines()[0].start_char, layout.lines()[0].end_char), (0, 3));
        assert_eq!((layout.lines()[1].start_char, layout.lines()[1].end_char), (3, 5));
        assert_eq!(layout.lines()[1].line_in_block, 1);
    }

    #[test]
    fn line_ranges_cover_blocks_without_gaps() {
        let doc = Document::new(
            "en",
            "T",
            vec![
                Block::new(BlockKind::Heading(1), "Заголовок"),
                Block::new(BlockKind::Paragraph, "Один два три четыре пять"),
                Block::new(BlockKind::Rule, ""),
            ],
        );
        let layout = Layout::new(&doc, 8);
        for (i, block) in doc.blocks().iter().enumerate() {
            let range = layout.block_range(i);
            assert!(!range.is_empty(), "блок {i} без строк");
            let mut at = 0;
            for line in &layout.lines()[range.clone()] {
                assert_eq!(line.block, i);
                assert_eq!(line.start_char, at, "блок {i}: разрыв после символа {at}");
                at = line.end_char;
            }
            assert_eq!(at, block.char_len(), "блок {i}: диапазоны не покрывают текст");
        }
        assert_eq!(layout.block_range(99), 0..0);
    }

    #[test]
    fn paragraphs_are_separated_by_a_blank_line() {
        let doc = Document::new(
            "en",
            "T",
            vec![
                Block::new(BlockKind::Paragraph, "первый"),
                Block::new(BlockKind::Paragraph, "второй абзац"),
                Block::new(BlockKind::Paragraph, "третий"),
            ],
        );
        let layout = Layout::new(&doc, 40);
        assert_eq!(layout.block_range(0), 0..2, "текст + отступ");
        assert_eq!(layout.block_range(1), 2..4);
        assert_eq!(layout.block_range(2), 4..5, "после последнего блока отступа нет");
        let blank = layout.line(1).expect("отступ после первого блока");
        assert_eq!(blank.block, 0, "отступ принадлежит своему блоку");
        assert_eq!(blank.start_char, blank.end_char, "отступ без символов");
        assert_eq!(blank.slice(doc.block(0).expect("блок")), "");
    }

    #[test]
    fn filler_and_newline_ending_blocks_get_no_blank_line() {
        let doc = Document::new(
            "en",
            "T",
            vec![
                Block::new(BlockKind::Paragraph, "ab\n"),
                Block::new(BlockKind::Rule, ""),
                Block::new(BlockKind::Paragraph, "hi"),
            ],
        );
        let layout = Layout::new(&doc, 40);
        assert_eq!(layout.block_range(0), 0..2, "концевой \\n не пускает отступ");
        assert_eq!(layout.block_range(1), 2..3, "пустой блок без отступа");
        assert_eq!(layout.block_range(2), 3..4);
    }

    #[test]
    fn zero_width_does_not_panic() {
        let layout = Layout::new(&paragraph("мир"), 0);
        assert!(!layout.is_empty());
        assert_eq!(layout.width(), 1);
    }

    #[test]
    fn empty_document_has_no_lines() {
        let doc = Document::new("en", "T", vec![]);
        let layout = Layout::new(&doc, 40);
        assert!(layout.is_empty());
        assert_eq!(layout.line(0), None);
        assert_eq!(layout.block_range(0), 0..0);
        assert_eq!(layout.block_count(), 0);
    }
}
