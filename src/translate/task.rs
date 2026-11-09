//! Планирование и журналируемый перевод целой книги.
//!
//! Гранулярность resume — юнит (блочный элемент документа). Готовые юниты
//! живут в журнале (SQLite `done`) и в черновиках на диске, поэтому после
//! сбоя перезапуск продолжает с места остановки. Документы с недоделками
//! переводит пул из `jobs` потоков; журнальный ход и сборка — основной поток.

use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{Read, Seek};
use std::ops::Range;
use std::path::Path;
use std::sync::{Arc, Mutex, mpsc};

use crate::parse::epub::content_documents;
use crate::store::{Store, TranslateJobSpec};
use zip::ZipArchive;

use super::Journal;
use super::epub::{Entry, Stats, read_entries, write_archive};
use super::xhtml::{DocKind, Unit, find_units};
use super::{Engine, TranslateError, splice};

/// Состав книги: сколько работы предстоит (`--dry-run`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Estimate {
    /// Контент-документов с непустыми юнитами.
    pub documents: usize,
    /// Единиц перевода в них.
    pub units: usize,
    /// Обращений к серверу при указанном размере партии.
    pub batches: usize,
}

/// Считает работу по плану: читает zip, классифицирует документы, пересчитывает
/// юниты. Ничего не отправляет серверу.
pub fn estimate(source: &Path, batch: usize) -> Result<Estimate, TranslateError> {
    let shown = source.display().to_string();
    let mut zip = open_zip(source, &shown)?;
    let paths = content_documents(&mut zip).map_err(wrap_zip_problem(&shown))?;
    let mut estimate = Estimate::default();
    for (name, kind) in plan(&paths) {
        let text = read_doc(&mut zip, &name, &shown)?;
        let units = find_units(&text, kind).into_iter().filter(|u| !u.is_blank()).count();
        estimate.units += units;
        if units > 0 {
            estimate.documents += 1;
        }
    }
    estimate.batches = estimate.units.div_ceil(batch.max(1));
    Ok(estimate)
}

/// Документы в порядке spine и их классы.
fn plan(paths: &crate::parse::epub::ContentPaths) -> Vec<(String, DocKind)> {
    let mut out: Vec<(String, DocKind)> =
        paths.body.iter().map(|name| (name.clone(), DocKind::Body)).collect();
    if let Some(nav) = &paths.nav {
        out.push((nav.clone(), DocKind::Nav));
    }
    if let Some(ncx) = &paths.ncx {
        out.push((ncx.clone(), DocKind::Ncx));
    }
    out
}

fn read_doc<R: Read + Seek>(
    zip: &mut ZipArchive<R>,
    name: &str,
    shown: &str,
) -> Result<String, TranslateError> {
    let mut entry = zip.by_name(name).map_err(|problem| TranslateError::Epub {
        path: shown.to_owned(),
        problem: problem.to_string(),
    })?;
    let mut text = String::new();
    entry
        .read_to_string(&mut text)
        .map_err(|source| TranslateError::Io { path: name.to_owned(), source })?;
    Ok(text)
}

fn open_zip(source: &Path, shown: &str) -> Result<ZipArchive<std::fs::File>, TranslateError> {
    let file = std::fs::File::open(source)
        .map_err(|source| TranslateError::Io { path: shown.to_owned(), source })?;
    ZipArchive::new(file).map_err(|problem| TranslateError::Epub {
        path: shown.to_owned(),
        problem: problem.to_string(),
    })
}

/// Перенос ошибки парсера на путь книги.
fn wrap_zip_problem(shown: &str) -> impl Fn(crate::parse::ParseError) -> TranslateError + '_ {
    let path = shown.to_owned();
    move |error| TranslateError::Epub { path: path.clone(), problem: error.to_string() }
}

/// Ключ юнита в журнале: `<документ>#<индекс>`.
fn unit_id(doc: &str, index: usize) -> String {
    format!("{doc}#{index}")
}

/// Результат воркера: переведённые юниты одного документа.
struct DocResult {
    doc_index: usize,
    replacements: Vec<(usize, String)>,
}

/// Одно задание перевода: параметры конвейера.
pub struct JobTask<'a> {
    pub source: &'a Path,
    pub target: &'a Path,
    pub from: &'a str,
    pub to: &'a str,
    pub spec: &'a TranslateJobSpec,
    /// Документов в параллели.
    pub jobs: usize,
    /// Ход прогресса: аргументы — готово и всего юнитов.
    pub progress: &'a mut dyn FnMut(usize, usize),
}

/// Перевод книги с журналом: resume, прогресс, параллельная загрузка.
/// Записывает целевой сайдкар только после полного успеха.
pub fn translate_journaled(
    task: &mut JobTask,
    engine: &(impl Engine + Sync),
    store: &Store,
) -> Result<Stats, TranslateError> {
    let JobTask { source, target, from, to, spec, jobs, progress } = task;
    let source: &Path = source;
    let target: &Path = target;
    let from: &str = from;
    let to: &str = to;
    let spec: &TranslateJobSpec = spec;
    let jobs: usize = *jobs;
    let shown = source.display().to_string();
    let mut zip = open_zip(source, &shown)?;
    let paths = content_documents(&mut zip).map_err(wrap_zip_problem(&shown))?;
    let plan = plan(&paths);

    let mut entries = read_entries(&mut zip, &shown)?;
    let mut entry_index: HashMap<String, usize> = HashMap::new();
    for (index, entry) in entries.iter().enumerate() {
        entry_index.insert(entry.name.clone(), index);
    }

    // План: юниты каждого документа, пронумерованные по не-пустым в нём.
    let mut docs: Vec<(String, Vec<(usize, Unit)>)> = Vec::new();
    let mut all_units: Vec<String> = Vec::new();
    for (name, kind) in plan {
        let Some(&entry_no) = entry_index.get(&name) else { continue };
        let Ok(text) = std::str::from_utf8(&entries[entry_no].data) else { continue };
        let units: Vec<(usize, Unit)> = find_units(text, kind)
            .into_iter()
            .filter(|unit| !unit.is_blank())
            .enumerate()
            .collect();
        if units.is_empty() {
            continue;
        }
        all_units.extend(units.iter().map(|(number, _)| unit_id(&name, *number)));
        docs.push((name, units));
    }

    let (mut journal, _resumed) = Journal::open(store, spec)?;
    journal.register(&all_units)?;
    let (total, started_done) = journal.counts()?;
    (progress)(started_done, total);

    let pending: HashSet<String> = journal.pending(&all_units)?.into_iter().collect();
    let queue: Vec<(usize, Vec<(usize, Unit)>)> = (0..docs.len())
        .map(|doc_index| {
            let (name, units) = &docs[doc_index];
            let work: Vec<(usize, Unit)> = units
                .iter()
                .filter(|(number, _)| pending.contains(&unit_id(name, *number)))
                .cloned()
                .collect();
            (doc_index, work)
        })
        .filter(|(_, work)| !work.is_empty())
        .collect();
    let expected = queue.len();

    let mut outcome_error: Option<TranslateError> = None;
    let (send_results, receive_results) = mpsc::channel::<Result<DocResult, TranslateError>>();
    std::thread::scope(|scope| {
        let shared_queue = Arc::new(Mutex::new(VecDeque::from(queue)));
        let workers = jobs.max(1).min(expected);
        for _ in 0..workers {
            let queue = Arc::clone(&shared_queue);
            let sender = send_results.clone();
            scope.spawn(move || {
                loop {
                    let Some((doc_index, work)) =
                        queue.lock().expect("очередь документов").pop_front()
                    else {
                        break;
                    };
                    let outcome = (|| -> Result<Vec<(usize, String)>, TranslateError> {
                        let templates: Vec<String> =
                            work.iter().map(|(_, unit)| unit.template()).collect();
                        let translated = engine.translate(&templates, from, to)?;
                        if translated.len() != work.len() {
                            return Err(TranslateError::Engine(format!(
                                "ожидалось {} переводов, получено {}",
                                work.len(),
                                translated.len()
                            )));
                        }
                        let mut out = Vec::with_capacity(work.len());
                        for ((number, unit), result) in work.iter().zip(translated) {
                            let rebuilt = accumulate(unit, &result, engine, from, to)?;
                            out.push((*number, rebuilt));
                        }
                        Ok(out)
                    })();
                    let _ = sender
                        .send(outcome.map(|replacements| DocResult { doc_index, replacements }));
                }
            });
        }
        drop(send_results);

        let mut done = started_done;
        for _ in 0..expected {
            let received = match receive_results.recv() {
                Ok(received) => received,
                Err(_) => break, // все воркеры погибли сами — ошибок там и нет
            };
            let result = match received {
                Ok(result) => result,
                Err(problem) => {
                    outcome_error = Some(problem);
                    break;
                }
            };
            let (doc_name, _) = &docs[result.doc_index];
            for (number, rebuilt) in result.replacements {
                if let Err(journal_failure) =
                    journal.mark_done(&unit_id(doc_name, number), &rebuilt)
                {
                    outcome_error = Some(journal_failure.into());
                    break;
                }
                done += 1;
                (progress)(done, total);
            }
        }
    });
    if let Some(problem) = outcome_error {
        return Err(problem);
    }

    let stats = assemble(&docs, &mut entries, &entry_index, &journal)?;
    write_archive(&entries, target)?;
    journal.finish()?;
    Ok(stats)
}

/// Прибивает переведённые юниты к документам и пересобирает записи архива.
fn assemble(
    docs: &[(String, Vec<(usize, Unit)>)],
    entries: &mut [Entry],
    entry_index: &HashMap<String, usize>,
    journal: &Journal,
) -> Result<Stats, TranslateError> {
    let mut stats = Stats::default();
    for (name, units) in docs {
        let Some(&index) = entry_index.get(name) else { continue };
        let Ok(text) = std::str::from_utf8(&entries[index].data) else { continue };
        let mut replacements: Vec<(Range<usize>, String)> = Vec::with_capacity(units.len());
        for (number, unit) in units {
            let id = unit_id(name, *number);
            let Some(rebuilt) = journal.draft(&id) else {
                return Err(TranslateError::Epub {
                    path: name.clone(),
                    problem: format!("черновик юнита утерян: {id}"),
                });
            };
            replacements.push((unit.range.clone(), rebuilt.to_owned()));
        }
        let rebuilt = splice(text, replacements);
        if rebuilt != text {
            stats.documents += 1;
        }
        entries[index].data = rebuilt.into_bytes();
    }
    Ok(stats)
}

/// Перевод одного юнита: полноценный rebuild или фолбэк по частям.
fn accumulate(
    unit: &Unit,
    result: &str,
    engine: &dyn Engine,
    from: &str,
    to: &str,
) -> Result<String, TranslateError> {
    if let Some(rebuilt) = unit.rebuild(result) {
        return Ok(rebuilt);
    }
    let parts: Vec<String> = unit.text_parts().into_iter().map(str::to_owned).collect();
    let done = engine.translate(&parts, from, to)?;
    Ok(unit.rebuild_parts(&done))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_ids_are_stable() {
        assert_eq!(unit_id("OEBPS/c1.xhtml", 3), "OEBPS/c1.xhtml#3");
    }
}
