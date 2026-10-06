//! Состояние читалки: варианты, позиция, навигация, переключение языка, прогресс.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, UNIX_EPOCH};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::align::{Alignment, align};
use crate::cli::find_sidecars;
use crate::model::{Anchor, Document, Layout, anchor_to_scroll, scroll_to_anchor};
use crate::parse;
use crate::store::{Store, StoreError, document_hash};

/// Период автосохранения прогресса.
const SAVE_INTERVAL: Duration = Duration::from_secs(5);
/// Строк на прокрутку колесом мыши.
const WHEEL_LINES: isize = 3;
const DEFAULT_WIDTH: u16 = 80;
const DEFAULT_HEIGHT: u16 = 24;
/// Резерв под префиксы блоков (цитаты, стихи) и скроллбар справа.
pub const TEXT_PAD: u16 = 5;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error(transparent)]
    Parse(#[from] parse::ParseError),
    #[error(transparent)]
    Store(#[from] StoreError),
}

struct Variant {
    lang: String,
    doc: Document,
    /// Выравнивание к базовому варианту; у базового не бывает.
    alignment: Option<Alignment>,
}

pub struct App {
    variants: Vec<Variant>,
    current: usize,
    scroll: usize,
    layout: Layout,
    width: u16,
    height: u16,
    store: Option<Store>,
    book_id: Option<i64>,
    /// Позиция из хранилища: применяется к первой раскладке, ещё до `set_size`.
    pending_anchor: Option<Anchor>,
    quit: bool,
    last_save: Instant,
}

impl App {
    /// Загрузить книгу: база `base_lang`, сайдкары `книга.<lang>.*` и явные
    /// `--variant`. Выравнивание берётся из кэша хранилища или строится заново.
    pub fn load(
        path: &Path,
        base_lang: &str,
        extra: &[(String, PathBuf)],
        store: Option<Store>,
    ) -> Result<Self, AppError> {
        let base = parse::load(path, base_lang)?;

        let mut specs: Vec<(String, PathBuf)> =
            find_sidecars(path).into_iter().filter(|(lang, _)| lang != base_lang).collect();
        for (lang, file) in extra {
            if lang == base_lang {
                continue;
            }
            match specs.iter_mut().find(|(l, _)| l == lang) {
                Some(slot) => slot.1 = file.clone(),
                None => specs.push((lang.clone(), file.clone())),
            }
        }
        specs.sort_by(|a, b| a.0.cmp(&b.0));

        let title = base.title().to_owned();
        let mut variants = vec![Variant { lang: base_lang.to_owned(), doc: base, alignment: None }];

        let book_id = if let Some(store) = &store {
            let (mtime, size) = file_stamp(path);
            Some(store.add_book(&path.display().to_string(), &title, base_lang, mtime, size)?)
        } else {
            None
        };
        let progress = match (&store, book_id) {
            (Some(store), Some(id)) => store.get_progress(id)?,
            _ => None,
        };

        for (lang, file) in specs {
            let doc = parse::load(&file, &lang)?;
            variants.push(Variant { lang, doc, alignment: None });
        }

        let (base_variant, rest) = variants.split_first_mut().expect("базовый вариант есть");
        for variant in rest {
            variant.alignment = Some(build_alignment(
                &base_variant.doc,
                &variant.doc,
                &variant.lang,
                &store,
                book_id,
            )?);
        }

        let saved_lang = progress.as_ref().map(|p| p.variant_lang.clone());
        let mut current = 0;
        if let Some(lang) = saved_lang
            && let Some(i) = variants.iter().position(|v| v.lang == lang)
        {
            current = i;
        }

        let layout = Layout::new(&variants[current].doc, DEFAULT_WIDTH - TEXT_PAD);
        Ok(Self {
            variants,
            current,
            scroll: 0,
            layout,
            width: DEFAULT_WIDTH,
            height: DEFAULT_HEIGHT,
            store,
            book_id,
            pending_anchor: progress.map(|p| p.anchor),
            quit: false,
            last_save: Instant::now(),
        })
    }

    pub fn languages(&self) -> Vec<&str> {
        self.variants.iter().map(|v| v.lang.as_str()).collect()
    }

    pub fn current_lang(&self) -> &str {
        &self.variants[self.current].lang
    }

    pub fn base_lang(&self) -> &str {
        &self.variants[0].lang
    }

    pub fn title(&self) -> &str {
        self.variants[0].doc.title()
    }

    pub fn document(&self) -> &Document {
        &self.variants[self.current].doc
    }

    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    pub fn scroll(&self) -> usize {
        self.scroll
    }

    pub fn set_scroll(&mut self, line: usize) {
        self.scroll = line.min(self.max_scroll());
    }

    pub fn should_quit(&self) -> bool {
        self.quit
    }

    pub fn store(&self) -> Option<&Store> {
        self.store.as_ref()
    }

    /// Текущая позиция в терминах базового документа (через выравнивание).
    pub fn anchor(&self) -> Anchor {
        scroll_to_anchor(&self.layout, self.document(), self.scroll)
    }

    /// Покрытие выравнивания текущего варианта; у базового варианта нет.
    pub fn coverage(&self) -> Option<f32> {
        self.variants[self.current].alignment.as_ref().map(Alignment::coverage)
    }

    pub fn viewport_height(&self) -> usize {
        self.height.saturating_sub(1).into()
    }

    pub fn max_scroll(&self) -> usize {
        self.layout.len().saturating_sub(self.viewport_height())
    }

    /// Новый размер окна: позиция переходом через якорь, сохраняя абзац на месте.
    pub fn set_size(&mut self, width: u16, height: u16) {
        let anchor = match self.pending_anchor.take() {
            Some(anchor) => Some(anchor),
            None => Some(self.anchor()),
        };
        self.width = width;
        self.height = height;
        self.layout = Layout::new(self.document(), width.saturating_sub(TEXT_PAD));
        if let Some(anchor) = anchor {
            self.scroll = anchor_to_scroll(&self.layout, self.document(), anchor);
        }
    }

    /// Переключиться на вариант `index`. Якорь переводится через базу, поэтому
    /// позиция чтения не прыгает. Вне диапазона — `false`.
    pub fn switch_lang(&mut self, index: usize) -> bool {
        if index >= self.variants.len() {
            return false;
        }
        self.resolve_pending();
        if index == self.current {
            return true;
        }
        let anchor = self.translate(self.anchor(), index);
        self.current = index;
        self.layout = Layout::new(self.document(), self.width.saturating_sub(TEXT_PAD));
        self.scroll = anchor_to_scroll(&self.layout, self.document(), anchor);
        true
    }

    /// Следующий язык по кругу (клавиша `L`).
    pub fn next_lang(&mut self) -> bool {
        let next = (self.current + 1) % self.variants.len();
        self.switch_lang(next)
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        let control = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Char('c') if control => self.quit = true,
            KeyCode::Char('j') | KeyCode::Down => self.scroll_by(1),
            KeyCode::Char('k') | KeyCode::Up => self.scroll_by(-1),
            KeyCode::Char(' ') | KeyCode::PageDown => {
                self.scroll_by(self.viewport_height() as isize)
            }
            KeyCode::PageUp => self.scroll_by(-(self.viewport_height() as isize)),
            KeyCode::Char('d') if control => self.scroll_by((self.viewport_height() / 2) as isize),
            KeyCode::Char('u') if control => {
                self.scroll_by(-((self.viewport_height() / 2) as isize))
            }
            KeyCode::Char('g') => self.scroll = 0,
            KeyCode::Char('G') => self.scroll = self.max_scroll(),
            KeyCode::Char('l' | 'L') => {
                self.next_lang();
            }
            KeyCode::Char(c @ '1'..='9') => {
                self.switch_lang(c as usize - '1' as usize);
            }
            _ => {}
        }
    }

    /// Колесо мыши: `up` — к началу, иначе к концу.
    pub fn handle_wheel(&mut self, up: bool) {
        let delta = if up { -WHEEL_LINES } else { WHEEL_LINES };
        self.scroll_by(delta);
    }

    /// Сохранить позицию и текущий язык в хранилище.
    pub fn save_progress(&mut self) -> Result<(), StoreError> {
        if let (Some(store), Some(book_id)) = (&self.store, self.book_id) {
            let anchor = self.anchor();
            let lang = self.current_lang().to_owned();
            store.set_progress(book_id, anchor, &lang)?;
            self.last_save = Instant::now();
        }
        Ok(())
    }

    /// Периодический тик: раз в [`SAVE_INTERVAL`] пишем прогресс.
    pub fn tick(&mut self) -> Result<(), StoreError> {
        if self.last_save.elapsed() >= SAVE_INTERVAL {
            self.save_progress()?;
        }
        Ok(())
    }

    fn scroll_by(&mut self, delta: isize) {
        let max = self.max_scroll() as isize;
        self.scroll = (self.scroll as isize + delta).clamp(0, max) as usize;
    }

    fn resolve_pending(&mut self) {
        if let Some(anchor) = self.pending_anchor.take() {
            self.scroll = anchor_to_scroll(&self.layout, self.document(), anchor);
        }
    }

    /// Якорь текущего варианта → целевой, через базу.
    fn translate(&self, anchor: Anchor, to: usize) -> Anchor {
        if to == 0 {
            self.variants[self.current]
                .alignment
                .as_ref()
                .map_or(anchor, |a| a.translate_var_to_base(anchor))
        } else if self.current == 0 {
            self.variants[to].alignment.as_ref().map_or(anchor, |a| a.translate_base_to_var(anchor))
        } else {
            let via_base = self.variants[self.current]
                .alignment
                .as_ref()
                .map_or(anchor, |a| a.translate_var_to_base(anchor));
            self.variants[to]
                .alignment
                .as_ref()
                .map_or(via_base, |a| a.translate_base_to_var(via_base))
        }
    }
}

/// Выравнивание варианта с базой: из кэша хранилища либо заново (и в кэш).
fn build_alignment(
    base: &Document,
    var: &Document,
    lang: &str,
    store: &Option<Store>,
    book_id: Option<i64>,
) -> Result<Alignment, AppError> {
    let base_hash = document_hash(base);
    let var_hash = document_hash(var);
    if let (Some(store), Some(id)) = (store.as_ref(), book_id)
        && let Some((map, _)) = store.load_alignment(id, lang, base_hash, var_hash)?
    {
        return Ok(Alignment::from_map(&map, base.len(), var.len()));
    }
    let alignment = align(base, var);
    if let (Some(store), Some(id)) = (store.as_ref(), book_id) {
        store.save_alignment(
            id,
            lang,
            base_hash,
            var_hash,
            &alignment.base_to_var,
            alignment.coverage(),
        )?;
    }
    Ok(alignment)
}

fn file_stamp(path: &Path) -> (i64, i64) {
    std::fs::metadata(path).map_or((0, 0), |meta| {
        let mtime = meta
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |age| age.as_secs() as i64);
        (mtime, meta.len() as i64)
    })
}
