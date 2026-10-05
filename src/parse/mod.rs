//! Разбор книг в `Document`.
//!
//! Книга — это всегда плоский список блоков: заголовки, абзацы, цитаты, списки.
//! Всё, что не является текстом (иллюстрации, сноски-обёртки), в v1 отбрасывается.

pub mod epub;
pub mod txt;

use std::path::Path;

use crate::model::Document;

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("не удалось прочитать {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("{path}: неподдерживаемый формат")]
    UnknownFormat { path: String },
    #[error("{path}: {problem}")]
    Malformed { path: String, problem: String },
    #[error("{path}: не удалось открыть архив: {problem}")]
    Zip { path: String, problem: String },
}

/// Что умеем открывать.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Epub,
    PlainText,
}

impl Format {
    /// Расширение важнее magic bytes: `.txt` может начинаться с `PK\x03\x04`,
    /// а `.epub` — нет. Расширение неизвестно, смотрим содержимое.
    pub fn detect(path: &Path) -> Option<Self> {
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        match ext.as_str() {
            "epub" => Some(Self::Epub),
            "txt" | "md" | "markdown" | "text" => Some(Self::PlainText),
            _ => match magic(path) {
                Some(magic) if magic.starts_with(b"PK\x03\x04") => Some(Self::Epub),
                _ => None,
            },
        }
    }
}

pub fn load(path: &Path, lang: &str) -> Result<Document, ParseError> {
    let format = Format::detect(path)
        .ok_or_else(|| ParseError::UnknownFormat { path: path.display().to_string() })?;
    let doc = match format {
        Format::Epub => epub::load(path),
        Format::PlainText => Ok(txt::load_str(&read_to_string(path)?, path, lang)),
    }?;
    doc.validate()
        .map_err(|problem| ParseError::Malformed { path: path.display().to_string(), problem })?;
    Ok(doc)
}

pub(crate) fn read_to_string(path: &Path) -> Result<String, ParseError> {
    let bytes = std::fs::read(path)
        .map_err(|source| ParseError::Io { path: path.display().to_string(), source })?;
    Ok(decode(&bytes))
}

/// Файлы часто приходят в windows- или mac-кодировке, поэтому сначала пробуем UTF-8
/// без BOM, потом с BOM, и только в конце откатываемся на lossy-разбор.
fn decode(bytes: &[u8]) -> String {
    if let Some(text) = strip_utf8_bom(bytes) {
        return text.to_owned();
    }
    let body = if bytes.starts_with(&[0xFF, 0xFE]) {
        decode_utf16(&bytes[2..], true).unwrap_or_else(|_| lossy(bytes))
    } else if bytes.starts_with(&[0xFE, 0xFF]) {
        decode_utf16_be(&bytes[2..])
    } else {
        lossy(bytes)
    };
    normalize_newlines(&body)
}

fn strip_utf8_bom(bytes: &[u8]) -> Option<&str> {
    let body = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF])?;
    std::str::from_utf8(body).ok()
}

fn decode_utf16(bytes: &[u8], little_endian: bool) -> Result<String, std::string::FromUtf16Error> {
    let units: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| if little_endian { u16::from_le_bytes(*c) } else { u16::from_be_bytes(*c) })
        .collect();
    String::from_utf16(&units)
}

fn decode_utf16_be(bytes: &[u8]) -> String {
    decode_utf16(bytes, false).unwrap_or_else(|_| lossy(bytes))
}

fn lossy(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// `\r\n` и одиночные `\r` в блоках ломают символьные индексы, поэтому приводим к `\n` сразу.
pub(crate) fn normalize_newlines(text: &str) -> String {
    if !text.contains('\r') {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\r' {
            out.push(c);
            continue;
        }
        if chars.peek() == Some(&'\n') {
            chars.next();
        }
        out.push('\n');
    }
    out
}

fn magic(path: &Path) -> Option<[u8; 4]> {
    use std::io::Read;
    let mut buf = [0u8; 4];
    std::fs::File::open(path).ok()?.read_exact(&mut buf).ok()?;
    Some(buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn temp_path(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("qbook-parse-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("создать каталог");
        dir
    }

    #[test]
    fn detects_format_by_extension() {
        assert_eq!(Format::detect(Path::new("a.epub")), Some(Format::Epub));
        assert_eq!(Format::detect(Path::new("a.TXT")), Some(Format::PlainText));
        assert_eq!(Format::detect(Path::new("a.md")), Some(Format::PlainText));
        assert_eq!(Format::detect(Path::new("a")), None);
    }

    #[test]
    fn detects_epub_by_magic_when_extension_is_unknown() {
        let path = temp_path("magic").join("book.bin");
        let mut f = std::fs::File::create(&path).expect("создать файл");
        f.write_all(b"PK\x03\x04rest").expect("записать");
        drop(f);
        assert_eq!(Format::detect(&path), Some(Format::Epub));
    }

    #[test]
    fn utf8_bom_is_stripped() {
        assert_eq!(decode("\u{feff}Привет".as_bytes()), "Привет");
    }

    #[test]
    fn utf16_le_is_decoded() {
        let bytes = [0xFF, 0xFE, 0x41, 0x00, 0x42, 0x00];
        assert_eq!(decode(&bytes), "AB");
    }

    #[test]
    fn utf16_be_is_decoded() {
        let bytes = [0xFE, 0xFF, 0x00, 0x41, 0x00, 0x42];
        assert_eq!(decode(&bytes), "AB");
    }

    #[test]
    fn invalid_utf8_does_not_panic() {
        assert_eq!(decode(&[0x41, 0xFF, 0x42]), "A\u{fffd}B");
    }

    #[test]
    fn newlines_are_normalized() {
        assert_eq!(normalize_newlines("a\r\nb\rc"), "a\nb\nc");
        assert_eq!(normalize_newlines("a\nb"), "a\nb");
    }
}
