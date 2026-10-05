//! Текст и Markdown.
//!
//! Ровно та же схема блоков, что и в EPUB, — это важно: выравнивание перевода
//! опирается на сопоставимость последовательностей блоков, и разница между
//! форматами должна быть только в источнике тегов.

use std::path::Path;

use crate::model::{Block, BlockKind, Document};

pub fn load_str(text: &str, path: &Path, lang: &str) -> Document {
    let blocks = parse(text);
    Document::new(lang, title_from_path(path), blocks)
}

/// Заголовок берём из имени файла: `# Заголовок` в .txt встречается далеко не всегда.
fn title_from_path(path: &Path) -> String {
    path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Поток строк, свёрнутый в блоки. Пустая строка завершает текущий блок.
fn parse(text: &str) -> Vec<Block> {
    let mut blocks: Vec<Block> = Vec::new();
    let mut current = Pending::default();

    for line in text.lines() {
        if let Some(kind) = rule(line) {
            current.flush(&mut blocks);
            blocks.push(Block::new(kind, ""));
            continue;
        }
        if line.trim().is_empty() {
            current.flush(&mut blocks);
            continue;
        }
        if current.starts_block(line) {
            current.flush(&mut blocks);
        }
        current.push_line(line);
    }
    current.flush(&mut blocks);

    blocks
}

/// Блок в процессе набора. Тип выводится по первой содержательной строке,
/// но отступ переопределяет его — так отступ превращается в Verse или Code.
#[derive(Default)]
struct Pending {
    kind: Option<BlockKind>,
    lines: Vec<String>,
}

impl Pending {
    /// `true`, если строка открывает новый блок, а не продолжает текущий.
    fn starts_block(&self, line: &str) -> bool {
        if self.lines.is_empty() {
            return false;
        }
        // Пунк списка всегда отдельный блок: иначе весь список схлопнется в один абзац
        // и при выравнивании перевода не будет на что опереться.
        matches!(self.kind, Some(BlockKind::ListItem)) || list_marker(line).is_some()
    }

    fn push_line(&mut self, line: &str) {
        if self.lines.is_empty() {
            self.kind = Some(classify(line));
        }
        self.lines.push(line.to_owned());
    }

    fn flush(&mut self, blocks: &mut Vec<Block>) {
        let Some(kind) = self.kind.take() else { return };
        if self.lines.is_empty() {
            return;
        }
        blocks.push(Block::new(kind, self.lines.join("\n")));
        self.lines.clear();
    }
}

fn rule(line: &str) -> Option<BlockKind> {
    let t = line.trim();
    let is_rule = t.len() >= 3
        && t.chars().all(|c| matches!(c, '-' | '_' | '*'))
        && t.chars().all(|c| c == '-' || c == '_' || c == '*');
    is_rule.then_some(BlockKind::Rule)
}

fn classify(line: &str) -> BlockKind {
    let indent = line.len() - line.trim_start().len();

    if let Some(rest) = line.trim_start().strip_prefix('>') {
        // Цитата внутри цитаты не отличается от обычной.
        let _ = rest;
        return BlockKind::Quote;
    }
    if indent >= 4 {
        return BlockKind::Code;
    }
    if let Some((level, _)) = heading(line) {
        return BlockKind::Heading(level);
    }
    if list_marker(line).is_some() {
        return BlockKind::ListItem;
    }
    if indent > 0 {
        return BlockKind::Verse;
    }
    BlockKind::Paragraph
}

/// `#`..`######` в начале непустой строки, максимум 6 уровней.
fn heading(line: &str) -> Option<(u8, &str)> {
    let hashes = line.trim_start().bytes().take_while(|&b| b == b'#').count();
    let rest = &line.trim_start()[hashes..];
    let heading = (1..=6).contains(&hashes) && rest.starts_with(' ') && !rest.trim().is_empty();
    heading.then(|| (hashes as u8, rest.trim()))
}

/// Маркер списка: `- `, `* `, `+ `, `1. `, `10) `.
fn list_marker(line: &str) -> Option<&str> {
    let t = line.trim_start();
    let bullet = t.strip_prefix(['-', '*', '+']).filter(|r| r.starts_with(' ')).map(|r| &r[1..]);
    if bullet.is_some() {
        return Some(bullet.unwrap_or_default());
    }
    let digits = t.bytes().take_while(u8::is_ascii_digit).count();
    if digits > 0 {
        let rest = &t[digits..];
        let marker = rest.strip_prefix(['.', ')']).filter(|r| r.starts_with(' '));
        if marker.is_some() {
            return marker;
        }
    }
    None
}

/// `## Заголовок` -> `Заголовок`. Валидация уже прошла в `Document::validate`.
pub fn strip_heading(text: &str) -> &str {
    let hashes = text.bytes().take_while(|&b| b == b'#').count();
    text.get(hashes..).map_or(text, str::trim)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blocks(text: &str) -> Vec<(BlockKind, String)> {
        parse(text).into_iter().map(|b| (b.kind, b.text)).collect()
    }

    #[test]
    fn splits_paragraphs_on_blank_lines() {
        let got = blocks("first para\nsecond line\n\nnext para\n");
        assert_eq!(
            got,
            [
                (BlockKind::Paragraph, "first para\nsecond line".to_owned()),
                (BlockKind::Paragraph, "next para".to_owned()),
            ]
        );
    }

    #[test]
    fn collapses_multiple_blank_lines() {
        let got = blocks("a\n\n\n\n\nb\n");
        assert_eq!(got.len(), 2);
    }

    #[test]
    fn reads_markdown_headings_with_level() {
        let got = blocks("# One\n\n### Three\n");
        assert_eq!(
            got,
            [
                (BlockKind::Heading(1), "# One".to_owned()),
                (BlockKind::Heading(3), "### Three".to_owned()),
            ]
        );
    }

    #[test]
    fn heading_text_is_stripped_of_hashes() {
        assert_eq!(strip_heading("## Chapter 1"), "Chapter 1");
        assert_eq!(strip_heading("Chapter 1"), "Chapter 1");
        assert_eq!(strip_heading("###"), "");
    }

    #[test]
    fn seven_hashes_is_not_a_heading() {
        let got = blocks("####### not a heading\n");
        assert_eq!(got, [(BlockKind::Paragraph, "####### not a heading".to_owned())]);
    }

    #[test]
    fn hash_without_space_is_not_a_heading() {
        let got = blocks("#tag\n");
        assert_eq!(got[0].0, BlockKind::Paragraph);
    }

    #[test]
    fn quotes_are_recognized() {
        let got = blocks("> quoted text\n> more\n");
        assert_eq!(got, [(BlockKind::Quote, "> quoted text\n> more".to_owned())]);
    }

    #[test]
    fn list_items_are_recognized() {
        let got = blocks("- one\n* two\n1. three\n2) four\n");
        assert_eq!(got.iter().map(|(k, _)| *k).collect::<Vec<_>>(), vec![BlockKind::ListItem; 4]);
    }

    #[test]
    fn four_space_indent_is_code() {
        let got = blocks("    let x = 1;\n    let y = 2;\n");
        assert_eq!(got, [(BlockKind::Code, "    let x = 1;\n    let y = 2;".to_owned())]);
    }

    #[test]
    fn small_indent_is_verse() {
        let got = blocks("  Once upon a time\n  there was a test\n");
        assert_eq!(got, [(BlockKind::Verse, "  Once upon a time\n  there was a test".to_owned())]);
    }

    #[test]
    fn verse_and_code_do_not_split_across_lines() {
        let got = blocks("  a\n\n  b\n\n    c\n\n    d\n");
        assert_eq!(
            got,
            [
                (BlockKind::Verse, "  a".to_owned()),
                (BlockKind::Verse, "  b".to_owned()),
                (BlockKind::Code, "    c".to_owned()),
                (BlockKind::Code, "    d".to_owned()),
            ]
        );
    }

    #[test]
    fn horizontal_rules() {
        let got = blocks("a\n\n---\n\nb\n");
        assert_eq!(got[1].0, BlockKind::Rule);
    }

    #[test]
    fn double_dash_is_not_a_rule() {
        let got = blocks("--\n");
        assert_eq!(got, [(BlockKind::Paragraph, "--".to_owned())]);
    }

    #[test]
    fn crlf_and_bom_are_handled() {
        let text = "\u{feff}# Title\r\n\r\nbody\r\n";
        let got = blocks(&crate::parse::normalize_newlines(text.strip_prefix('\u{feff}').unwrap()));
        assert_eq!(got[0].0, BlockKind::Heading(1));
        assert_eq!(got[1], (BlockKind::Paragraph, "body".to_owned()));
    }

    #[test]
    fn paragraph_kind_follows_first_line() {
        let got = blocks("> quote\ncontinued plain\n");
        assert_eq!(got[0].0, BlockKind::Quote);
    }

    #[test]
    fn empty_input_yields_no_blocks() {
        assert!(parse("").is_empty());
        assert!(parse("\n\n\n").is_empty());
    }

    #[test]
    fn parsed_document_is_valid() {
        let doc = load_str(
            "# Title\n\nbody\n\n> quote\n\n- item\n",
            Path::new("/books/Alice.epub"),
            "en",
        );
        assert_eq!(doc.title(), "Alice");
        assert_eq!(doc.validate(), Ok(()));
        assert_eq!(doc.toc().len(), 1);
    }
}
