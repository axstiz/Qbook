//! Журналируемая задача перевода: состояние юнитов в SQLite, черновики на диске.
//!
//! Resume работает в два шага: у переведённого юнита в базе ставится `done`,
//! а сам переведённый текст сохраняется в черновик (`translate-jobs/<id>/<док>.json`).
//! После сбоя перезапуск берёт готовые юниты из черновиков и переводит только
//! недоделанные, не обращаясь к серверу повторно.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};

use serde_json::json;

use crate::store::{Store, TranslateJob, TranslateJobSpec, UNIT_DONE};

/// Ключ идентичности источника в журнале: дактилоскопия содержимого файла.
pub fn source_fingerprint(path: &Path) -> String {
    let mut data = Vec::new();
    if let Ok(mut file) = std::fs::File::open(path)
        && file.read_to_end(&mut data).is_ok()
    {
        return format!("bytes-{:016x}", fnv1a(&data));
    }
    format!("mtime-{:016x}", fnv1a(path.to_string_lossy().as_bytes()))
}

/// FNV-1a (64-битная версия): быстрый хеш без зависимостей.
fn fnv1a(data: &[u8]) -> u64 {
    let mut state = 0xcbf29ce484222325;
    for byte in data {
        state ^= u64::from(*byte);
        state = state.wrapping_mul(0x00000100000001b3);
    }
    state
}

/// Черновик одного документа: индекс юнита → переведённое содержимое элемента.
type Draft = BTreeMap<String, String>;

/// Открытый журнал задачи перевода.
pub struct Journal<'a> {
    store: &'a Store,
    job: TranslateJob,
    resumed: bool,
    drafts: HashMap<String, Draft>,
    drafts_dir: PathBuf,
}

impl<'a> Journal<'a> {
    /// Открывает (или возобновляет) задачу по комбинации параметров перевода.
    pub fn open(store: &'a Store, spec: &TranslateJobSpec) -> Result<(Self, bool), JournalError> {
        let job = match store.find_translate_job(
            &spec.src_hash,
            &spec.from_lang,
            &spec.to_lang,
            &spec.engine,
            &spec.base_url,
        )? {
            Some(job) => job,
            None => store.create_translate_job(spec)?,
        };
        let drafts_dir = store.dir().join("translate-jobs").join(job.id.to_string());
        std::fs::create_dir_all(&drafts_dir)?;
        let drafts = load_drafts(&drafts_dir)?;
        let resumed = store.done_translate_units(job.id)? > 0;
        store.touch_translate_job(job.id)?;
        Ok((Self { store, job, resumed, drafts, drafts_dir }, resumed))
    }

    /// Перезапуск с уже готовыми юнитами.
    pub fn resumed(&self) -> bool {
        self.resumed
    }

    /// Идентификатор задачи.
    pub fn job_id(&self) -> i64 {
        self.job.id
    }

    /// Регистрирует юниты задачи, если их ещё нет.
    pub fn register(&self, units: &[String]) -> Result<(), JournalError> {
        self.store.register_translate_units(self.job.id, units)?;
        Ok(())
    }

    /// Всего и готово юнитов — для прогресса.
    pub fn counts(&self) -> Result<(usize, usize), JournalError> {
        Ok((
            self.store.total_translate_units(self.job.id)?,
            self.store.done_translate_units(self.job.id)?,
        ))
    }

    /// Юниты, требующие работы: не «done» в журнале из переданного списка.
    pub fn pending(&self, units: &[String]) -> Result<Vec<String>, JournalError> {
        let undone: HashSet<String> =
            self.store.pending_translate_units(self.job.id)?.into_iter().collect();
        Ok(units.iter().filter(|id| undone.contains(*id)).cloned().collect())
    }

    /// Черновик-результат юнита, если он уже сохранён (сбой между записью
    /// черновика и пометкой `done`).
    pub fn draft(&self, unit: &str) -> Option<&str> {
        let (doc, index) = unit.rsplit_once('#')?;
        self.drafts.get(doc)?.get(index).map(String::as_str)
    }

    /// Отметка юнита готовым: перевод кладётся в черновик, состояние — `done`.
    pub fn mark_done(&mut self, unit: &str, rebuilt: &str) -> Result<(), JournalError> {
        let (doc, index) =
            unit.rsplit_once('#').ok_or_else(|| JournalError::BadUnit(unit.to_owned()))?;
        self.store.set_translate_unit_state(self.job.id, unit, UNIT_DONE)?;
        let draft = self.drafts.entry(doc.to_owned()).or_default();
        draft.insert(index.to_owned(), rebuilt.to_owned());
        write_draft(&self.drafts_dir.join(format!("{}.json", sanitize(doc))), draft)?;
        Ok(())
    }

    /// Успешное завершение задачи.
    pub fn finish(&self) -> Result<(), JournalError> {
        self.store.touch_translate_job(self.job.id)?;
        Ok(())
    }
}

/// Ошибка журнала перевода.
#[derive(Debug, thiserror::Error)]
pub enum JournalError {
    /// Сбой хранилища или файловой системы.
    #[error(transparent)]
    Store(#[from] crate::store::StoreError),
    /// Сбой чтения/записи черновиков.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// Сборка JSON-черновика.
    #[error("черновик перевода: {0}")]
    Json(#[from] serde_json::Error),
    /// Юнит без формы `<документ>#<индекс>`.
    #[error("юнит без индекса документа: {0}")]
    BadUnit(String),
}

fn load_drafts(dir: &Path) -> Result<HashMap<String, Draft>, JournalError> {
    let mut drafts = HashMap::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let doc = path.file_stem().and_then(|s| s.to_str()).unwrap_or_default().to_owned();
        let doc = unsanitize(&doc);
        let raw = std::fs::read_to_string(&path)?;
        let draft: Draft = serde_json::from_str(&raw)?;
        drafts.insert(doc, draft);
    }
    Ok(drafts)
}

fn write_draft(path: &Path, draft: &Draft) -> Result<(), JournalError> {
    let raw = serde_json::to_string(draft)?;
    std::fs::write(path, raw)?;
    Ok(())
}

/// Путь к файлу черновика: слэши документа недопустимы в имени файла.
fn sanitize(doc: &str) -> String {
    doc.replace('/', "__")
}

/// Откат sanitize (`OEBPS__c1.xhtml` → `OEBPS/c1.xhtml`).
fn unsanitize(name: &str) -> String {
    name.replace("__", "/")
}

/// Проверка окружения `--check`: сервер выбранного движка отвечает.
pub fn server_ok(url: &str, engine: &str) -> Result<(), String> {
    let transport = super::libretranslate::UreqTransport;
    let (endpoint, body) = match engine {
        "ollama" => (
            format!("{}/v1/chat/completions", url.trim_end_matches('/')),
            json!({
                "model": "qbook-ping",
                "stream": false,
                "messages": [{ "role": "user", "content": "ping" }]
            }),
        ),
        _ => (
            format!("{}/translate", url.trim_end_matches('/')),
            json!({ "q": [], "source": "en", "target": "ru", "format": "text" }),
        ),
    };
    super::libretranslate::Transport::post_json(&transport, &endpoint, &body)
        .map(|_| ())
        .map_err(|problem| format!("{url}: {problem}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::TranslateJobSpec;

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_in(dir.path().join("data")).unwrap();
        (dir, store)
    }

    fn spec(src: &Path) -> TranslateJobSpec {
        TranslateJobSpec {
            src_path: src.display().to_string(),
            src_hash: "h1".to_owned(),
            dst_path: src.with_file_name("out.epub").display().to_string(),
            from_lang: "en".to_owned(),
            to_lang: "ru".to_owned(),
            engine: "libre".to_owned(),
            base_url: "http://localhost:5000".to_owned(),
        }
    }

    fn unit(doc: &str, index: usize) -> String {
        format!("{doc}#{index}")
    }

    #[test]
    fn opens_creates_job_and_registers_units() {
        let (_dir, store) = store();
        {
            let (journal, resumed) = Journal::open(&store, &spec(Path::new("b.epub"))).unwrap();
            assert!(!resumed);
            journal
                .register(&[unit("c.xhtml", 0), unit("c.xhtml", 1), unit("n.xhtml", 0)])
                .unwrap();
            assert_eq!(journal.counts().unwrap(), (3, 0));
            assert_eq!(journal.pending(&[unit("c.xhtml", 0)]).unwrap(), [unit("c.xhtml", 0)]);
        }
    }

    #[test]
    fn resume_after_interruption_skips_done_units() {
        let (_dir, store) = store();
        let src = Path::new("b.epub");
        {
            let (mut first, _) = Journal::open(&store, &spec(src)).unwrap();
            first.register(&[unit("c.xhtml", 0)]).unwrap();
            first.mark_done(&unit("c.xhtml", 0), "<p><em>got</em></p>").unwrap();
        }
        let (second, resumed) = Journal::open(&store, &spec(src)).unwrap();
        assert!(resumed);
        assert!(second.pending(&[unit("c.xhtml", 0)]).unwrap().is_empty());
        assert_eq!(second.draft(&unit("c.xhtml", 0)), Some("<p><em>got</em></p>"));
    }

    #[test]
    fn draft_survives_state_loss() {
        let (_dir, store) = store();
        let src = Path::new("b.epub");
        let job_id;
        {
            let (mut first, _) = Journal::open(&store, &spec(src)).unwrap();
            first.register(&[unit("c1.xhtml", 2)]).unwrap();
            first.mark_done(&unit("c1.xhtml", 2), "<b>got</b>").unwrap();
            job_id = first.job_id();
        }
        // Симуляция сбоя: откатываем состояние в базе к pending.
        store.set_translate_unit_state(job_id, "c1.xhtml#2", 0).unwrap();
        let (second, resumed) = Journal::open(&store, &spec(src)).unwrap();
        assert!(!resumed, "в базе нет готовых юнитов");
        assert_eq!(second.draft("c1.xhtml#2"), Some("<b>got</b>"));
    }

    #[test]
    fn source_fingerprint_is_stable_and_distinct() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.epub");
        std::fs::write(&path, b"hello").unwrap();
        assert_eq!(source_fingerprint(&path), source_fingerprint(&path));
        assert!(!source_fingerprint(&path).is_empty());
        std::fs::write(&path, b"changed").unwrap();
        std::fs::write(&path, b"changed").unwrap();
        assert_ne!(
            source_fingerprint(&path),
            source_fingerprint(&dir.path().join("y.epub")),
            "по разным данным — разные отпечатки"
        );
    }
}
