//! Плоский текст: абзацы по пустой строке.
//!
//! Markdown-разбор выпилен (спека v2): любые `#`/`>`/`---` — обычный текст.

use std::path::Path;

use crate::model::{Block, BlockKind, Document};

pub fn load_str(text: &str, path: &Path, lang: &str) -> Document {
    let blocks = parse(text);
    Document::new(lang, title_from_path(path), blocks)
}

/// Заголовок берём из имени файла: называется по исходнику книги.
fn title_from_path(path: &Path) -> String {
    path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Поток строк, свёрнутый в абзацы. Пустая строка — граница абзаца.
fn parse(text: &str) -> Vec<Block> {
    let mut blocks: Vec<Block> = Vec::new();
    let mut pending: Vec<String> = Vec::new();

    for line in text.lines() {
        if line.trim().is_empty() {
            flush(&mut pending, &mut blocks);
            continue;
        }
        pending.push(line.to_owned());
    }
    flush(&mut pending, &mut blocks);
    blocks
}

fn flush(pending: &mut Vec<String>, blocks: &mut Vec<Block>) {
    if pending.is_empty() {
        return;
    }
    blocks.push(Block::new(BlockKind::Paragraph, pending.join("\n")));
    pending.clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blocks(text: &str) -> Vec<(BlockKind, String)> {
        parse(text).into_iter().map(|b| (b.kind, b.text.clone())).collect()
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
    fn markdown_marks_are_plain_text() {
        let got = blocks("# One\n\n> quoted\n\n- item\n\n    code\n");
        assert!(got.iter().all(|(kind, _)| *kind == BlockKind::Paragraph));
        assert_eq!(got[0].1, "# One".to_owned(), "решётка — часть текста");
        assert_eq!(got[1].1, "> quoted".to_owned());
        assert_eq!(got[2].1, "- item".to_owned());
        assert_eq!(got[3].1, "    code".to_owned());
    }

    #[test]
    fn dashes_are_plain_text() {
        let got = blocks("--\n\n---\n\nb\n");
        assert!(got.iter().all(|(kind, _)| *kind == BlockKind::Paragraph));
    }

    #[test]
    fn indented_lines_are_plain_text() {
        let got = blocks("  Once upon a time\n\n    let x = 1;\n");
        assert!(got.iter().all(|(kind, _)| *kind == BlockKind::Paragraph));
    }

    #[test]
    fn empty_input_yields_no_blocks() {
        assert!(parse("").is_empty());
        assert!(parse("\n\n\n").is_empty());
    }

    #[test]
    fn parsed_document_is_valid() {
        let doc = load_str("plain\nbody\n\nother\n", Path::new("/books/Alice.txt"), "en");
        assert_eq!(doc.title(), "Alice");
        assert_eq!(doc.validate(), Ok(()));
        assert!(doc.toc().is_empty(), "без заголовков оглавление пустое");
    }
}
