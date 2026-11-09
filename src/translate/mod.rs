//! Оффлайн-перевод EPUB: правим текстовые узлы, сохраняя разметку.

pub mod epub;
pub mod journal;
pub mod libretranslate;
pub mod task;
pub mod xhtml;

use std::io::Write;
use std::ops::Range;
use std::path::{Path, PathBuf};

use xhtml::{DocKind, Unit, find_units};

pub use epub::Stats;
pub use journal::Journal;
use libretranslate::LibreTranslate;

use crate::store::{Store, TranslateJobSpec};

impl From<journal::JournalError> for TranslateError {
    fn from(problem: journal::JournalError) -> Self {
        TranslateError::Engine(problem.to_string())
    }
}

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
pub(crate) fn splice(source: &str, mut replacements: Vec<(Range<usize>, String)>) -> String {
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

/// Выполняет `qbook translate`: проверку, план или перевод с журналом.
pub fn run(args: &crate::cli::TranslateArgs) -> anyhow::Result<Stats> {
    let source = args.path.clone();
    if crate::parse::Format::detect(&source) != Some(crate::parse::Format::Epub) {
        anyhow::bail!("{}: перевод поддерживает только EPUB", source.display());
    }
    let from = resolve_source_language(&source, &args.from)?;

    if args.check {
        check(args)?;
        return Ok(Stats::default());
    }
    if args.dry_run {
        let plan =
            task::estimate(&source, args.batch).map_err(|problem| anyhow::anyhow!("{problem}"))?;
        println!(
            "план перевода: {} юнитов в {} документах ({} обращений при партии {})",
            plan.units, plan.documents, plan.batches, args.batch
        );
        return Ok(Stats::default());
    }

    let target = args.output.clone().unwrap_or_else(|| sidecar(&source, &args.to));
    let store = Store::open_default().map_err(|problem| anyhow::anyhow!("{problem}"))?;
    let spec = TranslateJobSpec {
        src_path: source.display().to_string(),
        src_hash: journal::source_fingerprint(&source),
        dst_path: target.display().to_string(),
        from_lang: from.clone(),
        to_lang: args.to.clone(),
        engine: ENGINE_NAME.to_owned(),
        base_url: args.server.clone(),
    };

    let engine = LibreTranslate::new(args.server.clone()).batch_size(args.batch);
    let resumed = store
        .find_translate_job(
            &spec.src_hash,
            &spec.from_lang,
            &spec.to_lang,
            &spec.engine,
            &spec.base_url,
        )
        .ok()
        .flatten()
        .map(|job| {
            let done = store.done_translate_units(job.id).unwrap_or_default();
            done > 0
        })
        .unwrap_or(false);

    let mut stderr = std::io::stderr().lock();
    let mut progress = |done: usize, total: usize| {
        if total > 0 {
            let _ = write!(stderr, "\rпереведено {done}/{total} юнитов\x1b[K");
            let _ = stderr.flush();
        }
    };
    let mut task = task::JobTask {
        source: &source,
        target: &target,
        from: &from,
        to: &args.to,
        spec: &spec,
        jobs: args.jobs,
        progress: &mut progress,
    };
    let stats = task::translate_journaled(&mut task, &engine, &store)?;
    writeln!(stderr, "\x1b[K\r")?;
    if resumed {
        println!("продолжаю необработанный перевод, задача лежит в журнале");
    }
    println!(
        "{} → {} (документов переведено: {})",
        source.display(),
        target.display(),
        stats.documents
    );
    Ok(stats)
}

/// Язык исходного текста: явный флаг или метаданные книги.
fn resolve_source_language(source: &Path, from_flag: &Option<String>) -> anyhow::Result<String> {
    match from_flag {
        Some(from) => Ok(from.clone()),
        None => crate::parse::epub::detect_lang(source)
            .ok_or_else(|| anyhow::anyhow!("не удалось определить язык книги; укажите --from")),
    }
}

/// Название движка в журнале задач.
const ENGINE_NAME: &str = "libretranslate";

/// `--check`: заглядывает в книгу, определяет язык и стучится на сервер.
/// Каждый шаг печатается; при любой неудаче — ошибка выхода.
pub fn check(args: &crate::cli::TranslateArgs) -> anyhow::Result<()> {
    let mut problems = Vec::new();
    match task::estimate(&args.path, args.batch) {
        Ok(plan) => println!("✓ книга: {} юнитов в {} документах", plan.units, plan.documents),
        Err(problem) => problems.push(format!("✗ книга: {problem}")),
    }
    match resolve_source_language(&args.path, &args.from) {
        Ok(from) => println!("✓ язык исходника: {from}"),
        Err(problem) => problems.push(format!("✗ язык исходника: {problem}")),
    }
    match journal::server_ok(&args.server) {
        Ok(()) => println!("✓ сервер: {}", args.server),
        Err(problem) => problems.push(format!("✗ сервер: {problem}")),
    }
    if problems.is_empty() {
        Ok(())
    } else {
        for line in problems {
            eprintln!("{line}");
        }
        anyhow::bail!("проверка пройдена с ошибками");
    }
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
