//! Оффлайн-перевод EPUB: правим текстовые узлы, сохраняя разметку.

pub mod epub;
pub mod libretranslate;
pub mod xhtml;

use std::ops::Range;
use std::path::{Path, PathBuf};

use xhtml::{DocKind, Unit, find_units};

pub use epub::Stats;
use libretranslate::LibreTranslate;

/// Ошибка перевода.
#[derive(Debug, thiserror::Error)]
pub enum TranslateError {
    /// Движок перевода вернул ошибку или некорректный ответ.
    #[error("движок перевода: {0}")]
    Engine(String),
    /// Ошибка чтения/записи файла EPUB.
    #[error("файл {path}: {source}")]
    Io {
        /// Путь к файлу.
        path: String,
        /// Исходная ошибка ввода-вывода.
        source: std::io::Error,
    },
    /// Структура EPUB некорректна.
    #[error("epub {path}: {problem}")]
    Epub {
        /// Путь к файлу.
        path: String,
        /// Описание проблемы.
        problem: String,
    },
}

/// Движок перевода партии текстов.
pub trait Engine {
    /// Переводит `texts` с языка `from` на язык `to`, сохраняя порядок.
    fn translate(
        &self,
        texts: &[String],
        from: &str,
        to: &str,
    ) -> Result<Vec<String>, TranslateError>;
}

/// Переводит один XHTML/XML-документ. При потере плейсхолдеров — фолбэк по узлам.
pub fn translate_document(
    document: &str,
    kind: DocKind,
    from: &str,
    to: &str,
    engine: &dyn Engine,
) -> Result<String, TranslateError> {
    let units = find_units(document, kind);

    let mut targets: Vec<&Unit> = Vec::new();
    let mut templates: Vec<String> = Vec::new();
    for unit in &units {
        if unit.is_blank() {
            continue;
        }
        targets.push(unit);
        templates.push(unit.template());
    }
    if targets.is_empty() {
        return Ok(document.to_owned());
    }

    let translated = engine.translate(&templates, from, to)?;
    if translated.len() != templates.len() {
        return Err(TranslateError::Engine(format!(
            "ожидалось {} переводов, получено {}",
            templates.len(),
            translated.len()
        )));
    }

    let mut replacements: Vec<(Range<usize>, String)> = Vec::with_capacity(targets.len());
    for (unit, result) in targets.iter().zip(translated) {
        let rebuilt = match unit.rebuild(&result) {
            Some(inner) => inner,
            None => {
                let parts: Vec<String> = unit.text_parts().into_iter().map(str::to_owned).collect();
                let done = engine.translate(&parts, from, to)?;
                unit.rebuild_parts(&done)
            }
        };
        replacements.push((unit.range.clone(), rebuilt));
    }

    Ok(splice(document, replacements))
}

/// Собирает документ, заменяя диапазоны на новые фрагменты.
fn splice(source: &str, mut replacements: Vec<(Range<usize>, String)>) -> String {
    replacements.sort_by_key(|(range, _)| range.start);
    let mut out = String::with_capacity(source.len());
    let mut cursor = 0usize;
    for (range, text) in replacements {
        out.push_str(&source[cursor..range.start]);
        out.push_str(&text);
        cursor = range.end;
    }
    out.push_str(&source[cursor..]);
    out
}

/// Выполняет `qbook translate`: переводит книгу через локальный LibreTranslate.
pub fn run(args: &crate::cli::TranslateArgs) -> anyhow::Result<Stats> {
    let source = &args.path;
    if crate::parse::Format::detect(source) != Some(crate::parse::Format::Epub) {
        anyhow::bail!("{}: перевод поддерживает только EPUB", source.display());
    }
    let from = match &args.from {
        Some(from) => from.clone(),
        None => crate::parse::epub::detect_lang(source)
            .ok_or_else(|| anyhow::anyhow!("не удалось определить язык книги; укажите --from"))?,
    };
    let target = args.output.clone().unwrap_or_else(|| sidecar(source, &args.to));

    let engine = LibreTranslate::new(args.server.clone());
    let stats = epub::translate_epub(source, &target, &from, &args.to, &engine)?;
    println!(
        "{} → {} (документов переведено: {})",
        source.display(),
        target.display(),
        stats.documents
    );
    Ok(stats)
}

/// `book.epub` + `ru` -> `book.ru.epub` рядом с исходником.
fn sidecar(source: &Path, lang: &str) -> PathBuf {
    let stem = source.file_stem().and_then(|s| s.to_str()).unwrap_or("book");
    source.with_file_name(format!("{stem}.{lang}.epub"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sidecar_keeps_stem_and_directory() {
        assert_eq!(sidecar(Path::new("/tmp/book.epub"), "ru"), Path::new("/tmp/book.ru.epub"));
        assert_eq!(sidecar(Path::new("book.epub"), "ru"), Path::new("book.ru.epub"));
    }
}
