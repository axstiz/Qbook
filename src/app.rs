//! Состояние читалки: варианты, позиция, навигация, переключение языка, прогресс.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, UNIX_EPOCH};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::align::{Alignment, align};
use crate::cli::find_sidecars;
use crate::model::{Anchor, Document, Layout, anchor_to_scroll, scroll_to_anchor};
use crate::parse;
use crate::store::{Bookmark, DEFAULT_NOTE_COLOR, Store, StoreError, document_hash};

/// Период автосохранения прогресса.
const SAVE_INTERVAL: Duration = Duration::from_secs(5);
/// Строк на прокрутку колесом мыши.
const WHEEL_LINES: isize = 3;
const DEFAULT_WIDTH: u16 = 80;
const DEFAULT_HEIGHT: u16 = 24;
/// Резерв под префиксы блоков (цитаты, стихи) и скроллбар справа.
pub const TEXT_PAD: u16 = 5;
/// Шаг клавиш `[`/`]`: уже/шире колонка, и её минимальная ширина.
const COL_STEP: i16 = 4;
const COL_MIN: u16 = 20;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error(transparent)]
    Parse(#[from] parse::ParseError),
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// Экран приложения: полка или читалка.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Shelf,
    Reader,
}

/// Что именно набирает пользователь в prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputPurpose {
    AddBook,
    RenameBookmark,
    NewBookmark,
    /// Командная строка ex-команд (клавиша `:` или панель «Команды»).
    Command,
}

/// Минимальная высота для полной btop-оболочки; ниже — компактный рендер
/// статус-баром.
const MIN_UI_HEIGHT: u16 = 6;

/// Имена семи цветов заметки — их показывает тост при перекраске.
const NOTE_COLOR_NAMES: [&str; 7] =
    ["красный", "зелёный", "жёлтый", "синий", "пурпурный", "голубой", "белый"];
/// Сколько тиков автосохранения живёт тост (~2.5 секунды).
const NOTICE_TICKS: u8 = 5;

/// Строка полки, готовая к отрисовке.
#[derive(Debug, Clone)]
pub struct ShelfBook {
    pub book_id: i64,
    pub title: String,
    pub path: String,
    pub base_lang: String,
    /// Базовый язык и все варианты перевода, по алфавиту после базы.
    pub langs: Vec<String>,
    pub percent: f32,
    /// Дата последнего открытия, а до первого открытия — дата файла (unix).
    pub date: i64,
}

struct Typing {
    purpose: InputPurpose,
    buffer: String,
    /// Какую закладку переименовываем.
    bookmark_id: Option<i64>,
    /// Якорь, на который вешается новая заметка.
    anchor: Option<Anchor>,
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
    screen: Screen,
    shelf_books: Vec<ShelfBook>,
    shelf_cursor: usize,
    shelf_error: Option<String>,
    typing: Option<Typing>,
    /// Кэш закладок текущей книги, отсортирован по позиции.
    bookmarks: Vec<Bookmark>,
    bookmarks_open: bool,
    bookmark_cursor: usize,
    toc_open: bool,
    toc_cursor: usize,
    help_open: bool,
    /// Смещение ширины переноса от автоширины (клавиши `[`/`]`).
    col_extra: i16,
    /// Язык по умолчанию для книг, добавляемых через prompt.
    default_lang: String,
    /// Тост в статус-баре и остаток его жизни в тиках.
    notice: Option<String>,
    notice_left: u8,
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
            let id = store.add_book(&path.display().to_string(), &title, base_lang, mtime, size)?;
            let _ = store.touch_book(id);
            Some(id)
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
        let mut app = Self {
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
            screen: Screen::Reader,
            shelf_books: Vec::new(),
            shelf_cursor: 0,
            shelf_error: None,
            typing: None,
            bookmarks: Vec::new(),
            bookmarks_open: false,
            bookmark_cursor: 0,
            toc_open: false,
            toc_cursor: 0,
            help_open: false,
            col_extra: 0,
            default_lang: base_lang.to_owned(),
            notice: None,
            notice_left: 0,
        };
        app.reload_bookmarks();
        Ok(app)
    }

    /// Полка: список книг из хранилища. Книга открывается клавишей `Enter`.
    pub fn shelf(store: Option<Store>, default_lang: &str) -> Result<Self, AppError> {
        let doc = Document::new(default_lang, "", Vec::new());
        let layout = Layout::new(&doc, DEFAULT_WIDTH - TEXT_PAD);
        let mut app = Self {
            variants: vec![Variant { lang: default_lang.to_owned(), doc, alignment: None }],
            current: 0,
            scroll: 0,
            layout,
            width: DEFAULT_WIDTH,
            height: DEFAULT_HEIGHT,
            store,
            book_id: None,
            pending_anchor: None,
            quit: false,
            last_save: Instant::now(),
            screen: Screen::Shelf,
            shelf_books: Vec::new(),
            shelf_cursor: 0,
            shelf_error: None,
            typing: None,
            bookmarks: Vec::new(),
            bookmarks_open: false,
            bookmark_cursor: 0,
            toc_open: false,
            toc_cursor: 0,
            help_open: false,
            col_extra: 0,
            default_lang: default_lang.to_owned(),
            notice: None,
            notice_left: 0,
        };
        app.reload_shelf();
        Ok(app)
    }

    pub fn screen(&self) -> Screen {
        self.screen
    }

    pub fn shelf_books(&self) -> &[ShelfBook] {
        &self.shelf_books
    }

    pub fn shelf_cursor(&self) -> usize {
        self.shelf_cursor
    }

    pub fn shelf_error(&self) -> Option<&str> {
        self.shelf_error.as_deref()
    }

    pub fn typing_buffer(&self) -> Option<&str> {
        self.typing.as_ref().map(|t| t.buffer.as_str())
    }

    pub fn typing_purpose(&self) -> Option<InputPurpose> {
        self.typing.as_ref().map(|t| t.purpose)
    }

    pub fn bookmarks(&self) -> &[Bookmark] {
        &self.bookmarks
    }

    pub fn bookmarks_open(&self) -> bool {
        self.bookmarks_open
    }

    pub fn bookmark_cursor(&self) -> usize {
        self.bookmark_cursor
    }

    pub fn toc_open(&self) -> bool {
        self.toc_open
    }

    pub fn toc_cursor(&self) -> usize {
        self.toc_cursor
    }

    pub fn help_open(&self) -> bool {
        self.help_open
    }

    /// Ширина переноса: автоширина со смещением `[`/`]`, зажатая в разумных пределах.
    pub fn wrap_width(&self) -> u16 {
        let max = self.width.saturating_sub(2);
        let low = COL_MIN.min(max);
        self.width.saturating_sub(TEXT_PAD).saturating_add_signed(self.col_extra).clamp(low, max)
    }

    /// Закладка на текущем блоке — её метку показывает статус-бар.
    pub fn bookmark_at(&self) -> Option<&Bookmark> {
        let block = self.anchor().block;
        self.bookmarks.iter().find(|b| b.anchor.block == block)
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
        if self.height < MIN_UI_HEIGHT {
            self.height.saturating_sub(1).into()
        } else {
            // Клавиатурный бар (1), слот тоста (1) и рамка текста (2 борта).
            self.height.saturating_sub(4).into()
        }
    }

    pub fn max_scroll(&self) -> usize {
        self.layout.len().saturating_sub(self.viewport_height())
    }

    /// Новый размер окна: позиция переходом через якорь, сохраняя абзац на месте.
    pub fn set_size(&mut self, width: u16, height: u16) {
        let anchor = self.pending_anchor.take().unwrap_or_else(|| self.anchor());
        self.width = width;
        self.height = height;
        self.apply_layout(anchor);
    }

    /// Перестроить раскладку под текущую ширину переноса и вернуть якорь на место.
    fn apply_layout(&mut self, anchor: Anchor) {
        self.layout = Layout::new(self.document(), self.wrap_width());
        self.scroll = anchor_to_scroll(&self.layout, self.document(), anchor);
    }

    /// Клавиши `[`/`]`: уже/шире колонку с сохранением позиции чтения.
    fn resize_column(&mut self, delta: i16) {
        self.col_extra = self.col_extra.saturating_add(delta);
        let anchor = self.anchor();
        self.apply_layout(anchor);
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
        self.apply_layout(anchor);
        true
    }

    /// Следующий язык по кругу (клавиша `t`).
    pub fn next_lang(&mut self) -> bool {
        let next = (self.current + 1) % self.variants.len();
        self.switch_lang(next)
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        if self.typing.is_some() {
            self.handle_typing(key);
        } else if self.screen == Screen::Shelf {
            self.handle_shelf_key(key);
        } else if self.toc_open {
            self.handle_toc_key(key);
        } else if self.help_open {
            self.handle_help_key(key);
        } else if self.bookmarks_open {
            self.handle_panel_key(key);
        } else {
            self.handle_reader_key(key);
        }
    }

    fn handle_reader_key(&mut self, key: KeyEvent) {
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
            KeyCode::Char('t') => {
                self.next_lang();
            }
            KeyCode::Char('1') => {
                self.close_panels();
            }
            KeyCode::Char('2') | KeyCode::Char('o') => self.toggle_toc(),
            KeyCode::Char('3') | KeyCode::Char('B') => self.toggle_bookmarks(),
            KeyCode::Char('5') | KeyCode::Char(':') => self.start_command(),
            KeyCode::Char('h') => self.go_shelf(),
            KeyCode::Char('b') => self.add_bookmark(),
            KeyCode::Char('n') => self.jump_bookmark(true),
            KeyCode::Char('p') => self.jump_bookmark(false),
            KeyCode::Char('?') => self.help_open = true,
            KeyCode::Char('[') => self.resize_column(-COL_STEP),
            KeyCode::Char(']') => self.resize_column(COL_STEP),
            _ => {}
        }
    }

    /// Оглавление: открыть, если закрыто (курсор на текущий раздел), иначе закрыть.
    fn toggle_toc(&mut self) {
        if self.toc_open {
            self.toc_open = false;
            return;
        }
        let block = self.anchor().block;
        self.toc_cursor =
            self.document().toc().iter().rposition(|item| item.block <= block).unwrap_or(0);
        self.toc_open = true;
    }

    /// Цифра `1`: оставить только текст, закрыть боковые панели.
    fn close_panels(&mut self) {
        self.toc_open = false;
        self.bookmarks_open = false;
    }

    /// Открыть командную строку ex-команд (`.5` или `:`).
    fn start_command(&mut self) {
        self.typing = Some(Typing {
            purpose: InputPurpose::Command,
            buffer: String::new(),
            bookmark_id: None,
            anchor: None,
        });
    }

    fn handle_toc_key(&mut self, key: KeyEvent) {
        let last = self.document().toc().len().saturating_sub(1);
        match key.code {
            KeyCode::Esc | KeyCode::Char('o') | KeyCode::Char('2') => self.toc_open = false,
            KeyCode::Char('1') => self.close_panels(),
            KeyCode::Char('3') => self.toggle_bookmarks(),
            KeyCode::Char('j') | KeyCode::Down => self.toc_cursor = (self.toc_cursor + 1).min(last),
            KeyCode::Char('k') | KeyCode::Up => self.toc_cursor = self.toc_cursor.saturating_sub(1),
            KeyCode::Enter => {
                self.toc_open = false;
                if let Some(item) = self.document().toc().get(self.toc_cursor).cloned() {
                    let base = self.translate(Anchor::at_block(item.block), 0);
                    self.goto_anchor(base);
                }
            }
            // `q` здесь не выходит из приложения — сначала закрой оглавление.
            _ => {}
        }
    }

    fn handle_help_key(&mut self, key: KeyEvent) {
        if matches!(key.code, KeyCode::Esc | KeyCode::Char('?')) {
            self.help_open = false;
        }
        // `q` в справке не выходит из приложения.
    }

    fn handle_shelf_key(&mut self, key: KeyEvent) {
        let control = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Char('c') if control => self.quit = true,
            KeyCode::Char('j') | KeyCode::Down => {
                self.shelf_cursor =
                    (self.shelf_cursor + 1).min(self.shelf_books.len().saturating_sub(1));
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.shelf_cursor = self.shelf_cursor.saturating_sub(1);
            }
            KeyCode::Enter => self.open_selected(),
            KeyCode::Char('a') => {
                self.shelf_error = None;
                self.typing = Some(Typing {
                    purpose: InputPurpose::AddBook,
                    buffer: String::new(),
                    bookmark_id: None,
                    anchor: None,
                });
            }
            KeyCode::Char('d') => self.delete_selected(),
            _ => {}
        }
    }

    fn handle_panel_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc | KeyCode::Char('B') | KeyCode::Char('3') | KeyCode::Char('q') => {
                self.bookmarks_open = false;
            }
            KeyCode::Char('1') => self.close_panels(),
            KeyCode::Char('2') => self.toggle_toc(),
            KeyCode::Char('j') | KeyCode::Down => {
                self.bookmark_cursor =
                    (self.bookmark_cursor + 1).min(self.bookmarks.len().saturating_sub(1));
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.bookmark_cursor = self.bookmark_cursor.saturating_sub(1);
            }
            KeyCode::Enter => {
                if let Some(bookmark) = self.bookmarks.get(self.bookmark_cursor).cloned() {
                    self.goto_anchor(bookmark.anchor);
                    self.bookmarks_open = false;
                }
            }
            KeyCode::Char('r') => {
                if let Some(bookmark) = self.bookmarks.get(self.bookmark_cursor) {
                    self.typing = Some(Typing {
                        purpose: InputPurpose::RenameBookmark,
                        buffer: String::new(),
                        bookmark_id: Some(bookmark.id),
                        anchor: None,
                    });
                }
            }
            KeyCode::Char('c') => self.recolor_by(1),
            KeyCode::Char('C') => self.recolor_by(-1),
            KeyCode::Char('D') => self.delete_selected_bookmark(),
            _ => {}
        }
    }

    fn handle_typing(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.typing = None;
                self.shelf_error = None;
            }
            KeyCode::Enter => self.commit_typing(),
            KeyCode::Backspace => {
                if let Some(t) = &mut self.typing {
                    t.buffer.pop();
                }
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                if let Some(t) = &mut self.typing {
                    t.buffer.push(c);
                }
            }
            _ => {}
        }
    }

    fn commit_typing(&mut self) {
        let Some(typing) = self.typing.take() else { return };
        match typing.purpose {
            InputPurpose::AddBook => {
                let path = PathBuf::from(&typing.buffer);
                match self.register_book(&path) {
                    Ok(()) => self.shelf_error = None,
                    Err(message) => {
                        self.shelf_error = Some(message);
                        self.typing = Some(typing);
                    }
                }
            }
            InputPurpose::RenameBookmark => {
                if let Some(id) = typing.bookmark_id
                    && let Some(store) = &self.store
                {
                    let _ = store.rename_bookmark(id, &typing.buffer);
                    if let Some(bookmark) = self.bookmarks.iter_mut().find(|b| b.id == id) {
                        bookmark.label = typing.buffer;
                    }
                }
            }
            InputPurpose::NewBookmark => {
                let Some(store) = self.store.as_ref() else { return };
                let Some(book_id) = self.book_id else { return };
                let Some(anchor) = typing.anchor else { return };
                let label = if typing.buffer.trim().is_empty() {
                    let text = self.document().block(anchor.block).map_or("", |b| b.text.as_str());
                    snippet(text)
                } else {
                    typing.buffer.trim().chars().take(80).collect()
                };
                if store.add_bookmark(book_id, anchor, &label, DEFAULT_NOTE_COLOR).is_ok() {
                    self.reload_bookmarks();
                    self.set_notice(format!("+ заметка: {label}"));
                }
            }
            InputPurpose::Command => {
                let command = typing.buffer.trim().to_owned();
                if let Some(message) = self.handle_command(&command) {
                    self.set_notice(message);
                }
            }
        }
    }

    /// Диспетчер ex-команд. Пустая строка закрывает prompt без действия.
    /// Возвращает тост: подтверждение либо ошибку.
    fn handle_command(&mut self, command: &str) -> Option<String> {
        let (name, arg) = command.split_once(' ').map_or((command, ""), |(a, b)| (a, b.trim()));
        match name {
            "" => None,
            "shelf" => {
                self.go_shelf();
                None
            }
            "q" => {
                self.quit = true;
                None
            }
            "b" => {
                self.add_bookmark();
                None
            }
            "lang" => self.command_lang(arg),
            "goto" => self.command_goto(arg),
            "open" => self.command_open(arg),
            other => Some(format!(": нет команды «{other}»")),
        }
    }

    fn command_lang(&mut self, arg: &str) -> Option<String> {
        if arg.is_empty() || arg == "t" {
            if self.next_lang() {
                Some(format!(": язык · {}", self.current_lang()))
            } else {
                None
            }
        } else if let Some(index) = self.variants.iter().position(|v| v.lang == arg) {
            if self.switch_lang(index) {
                Some(format!(": язык · {arg}"))
            } else {
                Some(format!(": нет языка «{arg}»"))
            }
        } else {
            Some(format!(": нет языка «{arg}»"))
        }
    }

    fn command_goto(&mut self, arg: &str) -> Option<String> {
        let Ok(n) = arg.parse::<usize>() else {
            return Some(": goto нужен номер блока".to_owned());
        };
        if n == 0 || n > self.document().len() {
            return Some(format!(": нет блока {n}"));
        }
        self.goto_anchor(Anchor::at_block(n - 1));
        Some(format!(": блок {n}"))
    }

    fn command_open(&mut self, arg: &str) -> Option<String> {
        if arg.is_empty() {
            return Some(": open нужен путь".to_owned());
        }
        let default_lang = self.default_lang.clone();
        let path = Path::new(arg);
        let store = match &self.store {
            Some(store) => store.reopen().ok(),
            None => None,
        };
        match App::load(path, &default_lang, &[], store) {
            Ok(mut app) => {
                let title = app.title().to_owned();
                let _ = self.register_book(path);
                app.default_lang = default_lang;
                *self = app;
                self.set_notice(format!(": открыт «{title}»"));
                None
            }
            Err(e) => Some(format!(": не открыть «{arg}» — {e}")),
        }
    }

    fn register_book(&mut self, path: &Path) -> Result<(), String> {
        let doc = parse::load(path, &self.default_lang).map_err(|e| e.to_string())?;
        let title = doc.title().to_owned();
        let (mtime, size) = file_stamp(path);
        let key = path.display().to_string();
        let lang = self.default_lang.clone();
        let Some(store) = self.store.as_ref() else {
            return Err("нет открытого хранилища".to_owned());
        };
        store.add_book(&key, &title, &lang, mtime, size).map_err(|e| e.to_string())?;
        self.reload_shelf();
        Ok(())
    }

    fn reload_shelf(&mut self) {
        self.shelf_books.clear();
        let Some(store) = &self.store else {
            self.shelf_cursor = 0;
            return;
        };
        match store.list_books() {
            Ok(books) => {
                for book in books {
                    let mut langs = vec![book.base_lang.clone()];
                    let mut extra: Vec<String> = store
                        .variants(book.id)
                        .map(|list| list.into_iter().map(|v| v.lang).collect())
                        .unwrap_or_default();
                    extra.sort();
                    langs.extend(extra);
                    let percent =
                        store.get_progress(book.id).ok().flatten().map_or(0.0, |p| p.percent);
                    let date =
                        if book.last_opened_at > 0 { book.last_opened_at } else { book.mtime };
                    self.shelf_books.push(ShelfBook {
                        book_id: book.id,
                        title: book.title,
                        path: book.path,
                        base_lang: book.base_lang,
                        langs,
                        percent,
                        date,
                    });
                }
                self.shelf_cursor = self.shelf_cursor.min(self.shelf_books.len().saturating_sub(1));
            }
            Err(err) => self.shelf_error = Some(err.to_string()),
        }
    }

    fn open_selected(&mut self) {
        let Some(item) = self.shelf_books.get(self.shelf_cursor).cloned() else { return };
        let default_lang = self.default_lang.clone();
        // Пробуем через новое соединение: при ошибке основное не теряется.
        let store = match &self.store {
            Some(store) => store.reopen().ok(),
            None => None,
        };
        match App::load(Path::new(&item.path), &item.base_lang, &[], store) {
            Ok(mut app) => {
                app.default_lang = default_lang;
                *self = app;
            }
            Err(err) => {
                self.shelf_error = Some(err.to_string());
            }
        }
    }

    fn delete_selected(&mut self) {
        let Some(item) = self.shelf_books.get(self.shelf_cursor) else { return };
        let id = item.book_id;
        if let Some(store) = &self.store
            && let Err(err) = store.delete_book(id)
        {
            self.shelf_error = Some(err.to_string());
            return;
        }
        self.shelf_error = None;
        self.reload_shelf();
    }

    fn go_shelf(&mut self) {
        let _ = self.save_progress();
        self.screen = Screen::Shelf;
        self.bookmarks_open = false;
        self.typing = None;
        self.shelf_error = None;
        self.bookmarks.clear();
        self.reload_shelf();
    }

    fn toggle_bookmarks(&mut self) {
        if self.bookmarks_open {
            self.bookmarks_open = false;
            return;
        }
        self.reload_bookmarks();
        let block = self.anchor().block;
        self.bookmark_cursor = self
            .bookmarks
            .iter()
            .enumerate()
            .min_by_key(|(_, b)| b.anchor.block.abs_diff(block))
            .map(|(i, _)| i)
            .unwrap_or(0);
        self.bookmarks_open = true;
    }

    fn add_bookmark(&mut self) {
        if self.book_id.is_none() || self.store.is_none() {
            return;
        }
        self.resolve_pending();
        let anchor = self.anchor();
        self.typing = Some(Typing {
            purpose: InputPurpose::NewBookmark,
            buffer: String::new(),
            bookmark_id: None,
            anchor: Some(anchor),
        });
    }

    fn reload_bookmarks(&mut self) {
        self.bookmarks.clear();
        let (Some(store), Some(id)) = (&self.store, self.book_id) else { return };
        if let Ok(mut list) = store.list_bookmarks(id) {
            list.sort_by_key(|b| b.anchor.block);
            self.bookmarks = list;
        }
        self.bookmark_cursor = self.bookmark_cursor.min(self.bookmarks.len().saturating_sub(1));
    }

    fn delete_selected_bookmark(&mut self) {
        let Some(bookmark) = self.bookmarks.get(self.bookmark_cursor) else { return };
        let id = bookmark.id;
        if let Some(store) = &self.store {
            let _ = store.delete_bookmark(id);
        }
        self.reload_bookmarks();
    }

    /// Перекрасить выбранную в панели заметку (`1..7`) и показать тост.
    fn recolor_selected(&mut self, color: u8) {
        let Some(bookmark) = self.bookmarks.get(self.bookmark_cursor) else { return };
        let id = bookmark.id;
        let color = color.min(6);
        if let Some(store) = &self.store {
            let _ = store.set_bookmark_color(id, color);
        }
        if let Some(bookmark) = self.bookmarks.iter_mut().find(|b| b.id == id) {
            bookmark.color = color;
        }
        self.set_notice(format!("цвет: {}", NOTE_COLOR_NAMES[usize::from(color)]));
    }

    fn recolor_by(&mut self, step: i8) {
        let Some(color) = self.bookmarks.get(self.bookmark_cursor).map(|b| b.color) else { return };
        let next = (i32::from(color) + i32::from(step)).rem_euclid(7) as u8;
        self.recolor_selected(next);
    }

    /// Цвет заметки на блоке — маркер▎ в тексте, если такая заметка есть.
    pub fn note_color(&self, block: usize) -> Option<u8> {
        self.bookmarks.iter().find(|b| b.anchor.block == block).map(|b| b.color)
    }

    /// Текст текущего тоста в статус-баре, если он ещё не погас.
    pub fn notice(&self) -> Option<&str> {
        self.notice.as_deref()
    }

    fn set_notice(&mut self, text: String) {
        self.notice = Some(text);
        self.notice_left = NOTICE_TICKS;
    }

    /// Следующая (`forward`) или предыдущая закладка по позиции в базе.
    fn jump_bookmark(&mut self, forward: bool) {
        if self.screen != Screen::Reader {
            return;
        }
        self.reload_bookmarks();
        let current = self.anchor().block;
        let target = if forward {
            self.bookmarks
                .iter()
                .filter(|b| b.anchor.block > current)
                .min_by_key(|b| b.anchor.block)
        } else {
            self.bookmarks
                .iter()
                .filter(|b| b.anchor.block < current)
                .max_by_key(|b| b.anchor.block)
        }
        .cloned();
        if let Some(bookmark) = target {
            self.goto_anchor(bookmark.anchor);
        }
    }

    /// Перейти к якорю в базовых терминах (закладки хранятся в базе).
    fn goto_anchor(&mut self, anchor: Anchor) {
        self.resolve_pending();
        let translated = if self.current == 0 {
            anchor
        } else {
            self.variants[self.current]
                .alignment
                .as_ref()
                .map_or(anchor, |a| a.translate_base_to_var(anchor))
        };
        self.scroll = anchor_to_scroll(&self.layout, self.document(), translated);
    }

    /// Колесо мыши: `up` — к началу, иначе к концу.
    pub fn handle_wheel(&mut self, up: bool) {
        let delta = if up { -WHEEL_LINES } else { WHEEL_LINES };
        self.scroll_by(delta);
    }

    /// Прогресс чтения в процентах (0..=100).
    pub fn percent(&self) -> f32 {
        let max = self.max_scroll();
        if max == 0 { 100.0 } else { self.scroll as f32 / max as f32 * 100.0 }
    }

    /// Сохранить позицию и текущий язык в хранилище.
    pub fn save_progress(&mut self) -> Result<(), StoreError> {
        if let (Some(store), Some(book_id)) = (&self.store, self.book_id) {
            let anchor = self.anchor();
            let lang = self.current_lang().to_owned();
            let percent = self.percent();
            store.set_progress(book_id, anchor, &lang, percent)?;
            self.last_save = Instant::now();
        }
        Ok(())
    }

    /// Периодический тик: раз в [`SAVE_INTERVAL`] пишем прогресс, тост угасает.
    pub fn tick(&mut self) -> Result<(), StoreError> {
        if self.notice_left > 0 {
            self.notice_left -= 1;
            if self.notice_left == 0 {
                self.notice = None;
            }
        }
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

/// Короткая метка закладки: текст абзаца без маркеров, не длиннее 30 знаков.
fn snippet(text: &str) -> String {
    let mut t = text.trim();
    t = t.trim_start_matches('#').trim_start();
    t = t.trim_start_matches('>').trim_start();
    if let Some(rest) = t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")) {
        t = rest;
    } else {
        let digits = t.chars().take_while(|c| c.is_ascii_digit()).count();
        if digits > 0 && digits < 3 && t[digits..].starts_with(". ") {
            t = &t[digits + 2..];
        }
    }
    let mut label: String = t.chars().take(30).collect();
    if t.chars().count() > 30 {
        label.push('…');
    }
    label
}
