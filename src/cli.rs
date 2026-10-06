//! Разбор аргументов командной строки и поиск файлов перевода.

use std::path::{Path, PathBuf};

use clap::Parser;

#[derive(Debug, Parser)]
#[command(version, about, name = "qbook")]
pub struct Cli {
    /// Путь к книге: epub, txt или md. Без пути открывается полка.
    pub path: Option<PathBuf>,

    /// Язык исходного текста (ISO 639-1)
    #[arg(long, default_value = "en")]
    pub lang: String,

    /// Файл перевода, формат LANG=PATH; можно указывать несколько раз
    #[arg(long = "variant", value_name = "LANG=PATH", value_parser = parse_variant)]
    pub variants: Vec<(String, PathBuf)>,
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
