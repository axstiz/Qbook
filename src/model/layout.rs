//! Раскладка документа в терминальные строки.
//!
//! Все индексы — в символах, не в байтах: иначе кириллица и эмодзи ломают
//! переходы между строками. Диапазоны строк блока идут подряд и покрывают весь
//! текст блока — на этом держится round-trip `scroll → Anchor → scroll`.

use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use super::{Block, Document};

/// Одна строка на экране: диапазон символов `[start_char, end_char)` внутри блока.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineInfo {
    pub block: usize,
    pub line_in_block: usize,
    pub start_char: usize,
    pub end_char: usize,
}

impl LineInfo {
    /// Текст строки. Конец включает хвостовые пробелы и перевод строки — их
    /// обрезает рендер, а диапазоны остаются сплошными и ничего не теряют.
    pub fn slice<'a>(&self, block: &'a Block) -> &'a str {
        fn byte_at(text: &str, chars: usize) -> usize {
            text.char_indices().nth(chars).map_or(text.len(), |(byte, _)| byte)
        }
        let start = byte_at(&block.text, self.start_char);
        let end = byte_at(&block.text, self.end_char);
        &block.text[start..end]
    }
}

/// Раскладка документа при заданной ширине. Кэшируется по «вариант + ширина».
#[derive(Debug, Clone)]
pub struct Layout {
    width: u16,
    lines: Vec<LineInfo>,
    /// Для каждого блока — диапазон строк, которые он занимает.
    block_range: Vec<Range<usize>>,
}

impl Layout {
    pub fn new(doc: &Document, width: u16) -> Self {
        // Нулевая ширина зациклила бы перенос: рисуем строку в один символ.
        let width = width.max(1);
        let cells = usize::from(width);
        let mut lines = Vec::new();
        let mut block_range = Vec::with_capacity(doc.len());
        for (block, item) in doc.blocks().iter().enumerate() {
            let start = lines.len();
            for (line_in_block, range) in wrap(&item.text, cells).into_iter().enumerate() {
                lines.push(LineInfo {
                    block,
                    line_in_block,
                    start_char: range.start,
                    end_char: range.end,
                });
            }
            block_range.push(start..lines.len());
        }
        Self { width, lines, block_range }
    }

    pub fn width(&self) -> u16 {
        self.width
    }

    pub fn lines(&self) -> &[LineInfo] {
        &self.lines
    }

    pub fn line(&self, index: usize) -> Option<&LineInfo> {
        self.lines.get(index)
    }

    /// Число блоков в раскладке: они идут в том же порядке, что и в документе.
    pub fn block_count(&self) -> usize {
        self.block_range.len()
    }

    pub fn len(&self) -> usize {
        self.lines.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// Строки блока. Для несуществующего блока возвращается пустой диапазон.
    pub fn block_range(&self, block: usize) -> Range<usize> {
        self.block_range.get(block).cloned().unwrap_or(0..0)
    }
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
fn wrap_segment(text: &str, width: usize) -> Vec<Range<usize>> {
    let mut lines = Vec::new();
    if text.is_empty() {
        lines.push(0..0);
        return lines;
    }
    let cells = cells(text);
    let total = text.chars().count();
    let mut line_start = 0;
    let mut used = 0;
    // Точка, с которой может начаться следующая строка: `(символ, ячейка)`.
    let mut last_break: Option<(usize, usize)> = None;
    let mut k = 0;
    while k < cells.len() {
        let cell = &cells[k];
        if used + cell.w > width {
            if cell.is_space {
                // Пробел сам не влезает: строка кончается перед ним, а хвостовые
                // пробелы добираются в её конец — визуально места они не занимают.
                let mut end = k;
                while end < cells.len() && cells[end].is_space {
                    end += 1;
                }
                lines.push(line_start..cells[end - 1].end);
                line_start = cells[end - 1].end;
                used = 0;
                last_break = None;
                k = end;
                continue;
            }
            if let Some((point, cell_at)) = last_break.filter(|&(point, _)| point > line_start) {
                // Строка кончается в последней точке переноса, ячейки после неё
                // переезжают в следующую строку.
                used = cells[cell_at..k].iter().map(|cell| cell.w).sum();
                lines.push(line_start..point);
                line_start = point;
                last_break = None;
                continue;
            }
            if cell.start == line_start {
                // Графема шире всей строки: ставим её и уходим дальше.
                lines.push(line_start..cell.end);
                line_start = cell.end;
                used = 0;
                k += 1;
                continue;
            }
            // Точки переноса не было — режем посреди слова.
            lines.push(line_start..cell.start);
            line_start = cell.start;
            used = 0;
            last_break = None;
            continue;
        }
        used += cell.w;
        if cell.break_after {
            last_break = Some((cell.end, k + 1));
        }
        k += 1;
    }
    if line_start < total {
        lines.push(line_start..total);
    }
    lines
}

/// Графема с её свойствами: единица, которую нельзя резать.
struct Cell {
    start: usize,
    end: usize,
    /// Ширина в клетках терминала.
    w: usize,
    is_space: bool,
    /// После графемы можно начинать новую строку.
    break_after: bool,
}

fn cells(text: &str) -> Vec<Cell> {
    let mut out = Vec::new();
    let mut at = 0;
    for grapheme in text.graphemes(true) {
        let len = grapheme.chars().count();
        // Табуляция у юникода нулевой ширины, а на экране она место занимает.
        let w = if grapheme == "\t" { 1 } else { UnicodeWidthStr::width(grapheme) };
        let is_space = grapheme.chars().all(char::is_whitespace)
            && !grapheme.contains('\u{00A0}')
            && !grapheme.contains('\u{202F}');
        let last = grapheme.chars().last().unwrap_or(' ');
        let break_after = is_space || matches!(last, '-' | '/') || is_cjk(last);
        out.push(Cell { start: at, end: at + len, w, is_space, break_after });
        at += len;
    }
    out
}

/// У китайского, японского и корейского нет пробелов, поэтому строку между ними
/// рвём посимвольно.
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
    use crate::model::BlockKind;

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
    fn trailing_spaces_do_not_take_width() {
        assert_eq!(rendered(&paragraph("aa  bb"), 3), ["aa", "bb"]);
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
