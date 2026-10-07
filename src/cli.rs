//! Разбор аргументов командной строки и поиск файлов перевода.

use std::path::{Path, PathBuf};

use clap::Parser;

#[derive(Debug, Parser)]
#[command(version, about, name = "qbook")]
pub struct Cli {
    /// Путь к книге: epub, txt или md. Без пути открывается полка.
    pub path: Option<PathBuf>,

    /// Язык исходного текста (ISO 639-1). Без флага определяется
    /// автоматически: по суффиксу имени книги, метаданным EPUB или тексту.
    #[arg(long)]
    pub lang: Option<String>,

    /// Файл перевода, формат LANG=PATH; можно указывать несколько раз
    #[arg(long = "variant", value_name = "LANG=PATH", value_parser = parse_variant)]
    pub variants: Vec<(String, PathBuf)>,

    /// Пресет темы поверх встроенного дефолта (файл конфига перекрывает его)
    #[arg(long, value_name = "NAME", value_parser = parse_preset)]
    pub theme: Option<String>,
}

fn parse_preset(raw: &str) -> Result<String, String> {
    if crate::config::PRESETS.contains(&raw) {
        Ok(raw.to_owned())
    } else {
        Err(format!("неизвестный пресет «{raw}», доступны: {}", crate::config::PRESETS.join(", ")))
    }
}

fn parse_variant(raw: &str) -> Result<(String, PathBuf), String> {
    match raw.split_once('=') {
        Some((lang, path)) if !lang.is_empty() && !path.is_empty() => {
            Ok((lang.to_owned(), PathBuf::from(path)))
        }
        _ => Err(format!("ожидается LANG=PATH (например ru=book.ru.md), получено: {raw}")),
    }
}

/// Сайдкары перевода рядом с книгой: `книга.<lang>.{epub,txt,md,...}`.
/// Возвращаются отсортированными по языку; сама книга не попадает в список.
pub fn find_sidecars(path: &Path) -> Vec<(String, PathBuf)> {
    let (Some(dir), Some(stem)) = (path.parent(), path.file_stem().and_then(|s| s.to_str())) else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let prefix = format!("{stem}.");
    let mut found: Vec<(String, PathBuf)> = entries
        .flatten()
        .filter_map(|entry| {
            let file = entry.path();
            let name = entry.file_name().into_string().ok()?;
            let rest = name.strip_prefix(&prefix)?;
            let lang = rest.split_once('.').map(|(lang, _)| lang)?;
            if lang.is_empty() || crate::parse::Format::detect(&file).is_none() {
                return None;
            }
            Some((lang.to_owned(), file))
        })
        .collect();
    found.sort_by(|a, b| a.0.cmp(&b.0));
    found
}

/// Коды, которые умеем распознавать автоматически: пока только `en` и `ru`.
const KNOWN_LANGS: &[&str] = &["en", "ru"];

/// Код языка из суффикса имени книги (`книга.ru.md`, `chapter.en.txt`).
/// Совпадает с паттерном сайдкара, но ограничен распознаваемыми языками,
/// чтобы не принимать за язык любого слова вроде `том.1`.
pub fn lang_from_filename(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?;
    let (base, lang) = stem.rsplit_once('.')?;
    if !base.is_empty() && KNOWN_LANGS.contains(&lang) { Some(lang.to_owned()) } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suffix_recognizes_known_language_names() {
        assert_eq!(lang_from_filename(Path::new("book.ru.md")).as_deref(), Some("ru"));
        assert_eq!(lang_from_filename(Path::new("chapter.en.txt")).as_deref(), Some("en"));
        assert_eq!(lang_from_filename(Path::new("book.md")), None);
        assert_eq!(lang_from_filename(Path::new("том.3.md")), None, "не имя языка");
        assert_eq!(lang_from_filename(Path::new("works")), None);
    }
}
