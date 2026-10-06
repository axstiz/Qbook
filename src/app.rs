//! Состояние читалки: варианты, позиция, навигация, переключение языка, прогресс.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, UNIX_EPOCH};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::align::{Alignment, align};
use crate::cli::find_sidecars;
use crate::model::{Anchor, Block, Document, Layout, LineInfo, anchor_to_scroll, scroll_to_anchor};
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
/// Ширины постоянных колонок и минимальная ширина центральной рамки — та же
/// геометрия, что использует рендер `ui::reader`.
pub const LEFT_W: u16 = 24;
pub const RIGHT_W: u16 = 36;
pub const MIN_CENTER: u16 = 18;

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

/// Какой блок получает клавиши в трёхколоночной btop-оболочке.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReaderFocus {
    /// Центральный текст и чтение.
    Text,
    /// Левая колонка оглавления.
    Toc,
    /// Правая колонка, список заметок.
    Bookmarks,
    /// Правая колонка, список команд.
    Commands,
}

/// Минимальная высота для полной btop-оболочки; ниже — компактный рендер
/// статус-баром.
const MIN_UI_HEIGHT: u16 = 6;

/// Имена семи цветов заметки — их показывает тост при перекраске.
const NOTE_COLOR_NAMES: [&str; 7] =
    ["красный", "зелёный", "жёлтый", "синий", "пурпурный", "голубой", "белый"];
/// Список полезных команд для правой колонки «Команды»: имя, аргументы, смысл.
pub const COMMANDS: &[(&str, &str, &str)] = &[
    ("open", "<путь>", "открыть книгу"),
    ("lang", "t|<код>", "язык по кругу / по коду"),
    ("goto", "<блок>", "перейти к блоку"),
    ("toc", "", "показать/скрыть главы"),
    ("notes", "", "показать/скрыть заметки"),
    ("panels", "", "только текст ⇄ все колонки"),
    ("wider", "", "шире колонку"),
    ("narrower", "", "уже колонку"),
    ("help", "", "справка по клавишам"),
    ("shelf", "", "полка"),
    ("b", "", "закладка здесь"),
    ("q", "", "выход"),
];
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
    /// Выбранный цвет создаваемой заметки (`c`/`C` в окне ввода).
    color: u8,
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
    bookmark_cursor: usize,
    /// Постоянные колонки btop-оболочки: слева главы, справа заметки/команды.
    show_toc: bool,
    show_bookmarks: bool,
    show_commands: bool,
    toc_cursor: usize,
    command_cursor: usize,
    /// Какой блок получает клавиши.
    focus: ReaderFocus,
    help_open: bool,
    /// Смещение ширины переноса от автоширины (клавиши `[`/`]`).
    col_extra: i16,
    /// Язык по умолчанию для книг, добавляемых через prompt.
    default_lang: String,
    /// Тост в статус-баре и остаток его жизни в тиках.
    notice: Option<String>,
    notice_left: u8,
    /// Курсор над строками текста: выбор строки заметки или выделение.
    pick: Option<Pick>,
    /// Последний скопированный фрагмент (для тестов).
    copied: Option<String>,
    /// Фрагмент для отправки терминалу через OSC 52 — забирает `main`.
    clipboard_out: Option<String>,
}

/// Режим курсора над строками текста.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PickKind {
    /// Выбор строки, к которой прикрепится заметка (только видимая область).
    Note,
    /// Визуальное выделение с копированием.
    Select,
}

struct Pick {
    kind: PickKind,
    /// Курсор, строка раскладки.
    row: usize,
    /// Якорь выделения (для `Select`), строка раскладки.
    base: usize,
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
        // Один вариант на язык: при дубле предпочитаем сайдкар с расширением
        // исходной книги (`book.ru.md` для `book.md`), далее — по алфавиту имени.
        let ext = path.extension().and_then(|e| e.to_str());
        specs.sort_by(|a, b| {
            a.0.cmp(&b.0).then_with(|| {
                let same_a = usize::from(a.1.extension().and_then(|e| e.to_str()) == ext);
                let same_b = usize::from(b.1.extension().and_then(|e| e.to_str()) == ext);
                same_a.cmp(&same_b).reverse().then_with(|| a.1.cmp(&b.1))
            })
        });
        specs.dedup_by_key(|(lang, _)| lang.clone());
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
            bookmark_cursor: 0,
            show_toc: true,
            show_bookmarks: true,
            show_commands: true,
            toc_cursor: 0,
            command_cursor: 0,
            focus: ReaderFocus::Text,
            help_open: false,
            col_extra: 0,
            pick: None,
            copied: None,
            clipboard_out: None,
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
            bookmark_cursor: 0,
            show_toc: true,
            show_bookmarks: true,
            show_commands: true,
            toc_cursor: 0,
            command_cursor: 0,
            focus: ReaderFocus::Text,
            help_open: false,
            col_extra: 0,
            pick: None,
            copied: None,
            clipboard_out: None,
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

    pub fn bookmarks_visible(&self) -> bool {
        self.show_bookmarks
    }

    pub fn bookmark_cursor(&self) -> usize {
        self.bookmark_cursor
    }

    pub fn toc_visible(&self) -> bool {
        self.show_toc
    }

    pub fn commands_visible(&self) -> bool {
        self.show_commands
    }

    pub fn focus(&self) -> ReaderFocus {
        self.focus
    }

    pub fn toc_cursor(&self) -> usize {
        self.toc_cursor
    }

    pub fn command_cursor(&self) -> usize {
        self.command_cursor
    }

    pub fn help_open(&self) -> bool {
        self.help_open
    }

    /// Ширина рамки текста при текущих колонках — та же геометрия, что у рендера.
    fn center_width(&self) -> u16 {
        let full = self.width;
        let right = self.bookmarks_visible() || self.commands_visible();
        if self.toc_visible() && right && full >= LEFT_W + RIGHT_W + 2 + MIN_CENTER {
            full - LEFT_W - RIGHT_W - 2
        } else if self.toc_visible() && full >= LEFT_W + 1 + MIN_CENTER {
            full - LEFT_W - 1
        } else if right && full >= 1 + MIN_CENTER + RIGHT_W {
            full - RIGHT_W - 1
        } else {
            full
        }
    }

    /// Ширина переноса: текст занимает всю внутреннюю ширину рамки минус
    /// скроллбар и максимальный отступ префиксов; `[`/`]` подстраивают вручную.
    pub fn wrap_width(&self) -> u16 {
        let inner = self.center_width().saturating_sub(2);
        let max_wrap = inner.saturating_sub(1);
        let low = COL_MIN.min(max_wrap);
        let target = inner.saturating_sub(TEXT_PAD).saturating_add_signed(self.col_extra);
        target.clamp(low, max_wrap)
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
        } else if self.help_open {
            self.handle_help_key(key);
        } else if self.pick.is_some() {
            self.handle_pick_key(key);
        } else if self.panel_digit(key) {
            // Цифры 1–4 переключают постоянные btop-колонки из любого фокуса.
        } else {
            match self.focus {
                ReaderFocus::Text => self.handle_reader_key(key),
                ReaderFocus::Toc => self.handle_toc_key(key),
                ReaderFocus::Bookmarks => self.handle_bookmarks_key(key),
                ReaderFocus::Commands => self.handle_commands_key(key),
            }
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
            KeyCode::Char('5') | KeyCode::Char(':') => self.start_command(),
            KeyCode::Char('h') => self.go_shelf(),
            KeyCode::Char('b') => self.start_note_pick(),
            KeyCode::Char('v') => self.start_select(),
            KeyCode::Char('n') => self.jump_bookmark(true),
            KeyCode::Char('p') => self.jump_bookmark(false),
            KeyCode::Char('?') => self.help_open = true,
            KeyCode::Char('[') => self.resize_column(-COL_STEP),
            KeyCode::Char(']') => self.resize_column(COL_STEP),
            _ => {}
        }
    }

    /// Цифра 1–4 — фокус на блок; Shift+цифра — показать/скрыть.
    /// `o`/`B` — однобуквенные фокус-алиасы глав и заметок.
    fn panel_digit(&mut self, key: KeyEvent) -> bool {
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        match key.code {
            KeyCode::Char('1') if !shift => {
                self.focus = ReaderFocus::Text;
            }
            KeyCode::Char('1') if shift => {
                self.toggle_panels();
            }
            KeyCode::Char('!') => {
                self.toggle_panels();
            }
            KeyCode::Char('2') if !shift => {
                self.focus_toc();
            }
            KeyCode::Char('2') if shift => {
                self.toggle_toc();
            }
            KeyCode::Char('o') => {
                self.focus_toc();
            }
            KeyCode::Char('@') => {
                self.toggle_toc();
            }
            KeyCode::Char('3') if !shift => {
                self.focus_bookmarks();
            }
            KeyCode::Char('3') if shift => {
                self.toggle_bookmarks();
            }
            KeyCode::Char('B') => {
                self.focus_bookmarks();
            }
            KeyCode::Char('#') => {
                self.toggle_bookmarks();
            }
            KeyCode::Char('4') if !shift => {
                self.focus_commands();
            }
            KeyCode::Char('4') if shift => {
                self.toggle_commands();
            }
            KeyCode::Char('$') => {
                self.toggle_commands();
            }
            _ => return false,
        }
        true
    }

    /// Shift+1: только текст ⇄ все колонки.
    fn toggle_panels(&mut self) {
        if self.show_toc || self.show_bookmarks || self.show_commands {
            self.show_toc = false;
            self.show_bookmarks = false;
            self.show_commands = false;
            self.focus = ReaderFocus::Text;
        } else {
            self.show_toc = true;
            self.show_bookmarks = true;
            self.show_commands = true;
        }
        self.reflow();
    }

    /// Цифра `2`: скрыть/показать главы, фокус не меняется.
    fn toggle_toc(&mut self) {
        self.show_toc = !self.show_toc;
        if !self.show_toc && self.focus == ReaderFocus::Toc {
            self.focus = ReaderFocus::Text;
        }
        if self.show_toc {
            self.sync_toc_cursor();
        }
        self.reflow();
    }

    /// Shift+2: показать главы и встать в них курсором.
    fn focus_toc(&mut self) {
        let shown = self.show_toc;
        self.show_toc = true;
        self.sync_toc_cursor();
        self.focus = ReaderFocus::Toc;
        if !shown {
            self.reflow();
        }
    }

    /// Цифра `3`: скрыть/показать список заметок, фокус не меняется.
    fn toggle_bookmarks(&mut self) {
        self.show_bookmarks = !self.show_bookmarks;
        if !self.show_bookmarks && self.focus == ReaderFocus::Bookmarks {
            self.focus = ReaderFocus::Text;
        }
        if self.show_bookmarks {
            self.reload_bookmarks();
            self.sync_bookmark_cursor();
        }
        self.reflow();
    }

    /// Shift+3: показать заметки и встать в список.
    fn focus_bookmarks(&mut self) {
        let shown = self.show_bookmarks;
        self.show_bookmarks = true;
        self.reload_bookmarks();
        self.sync_bookmark_cursor();
        self.focus = ReaderFocus::Bookmarks;
        if !shown {
            self.reflow();
        }
    }

    /// Цифра `4`: скрыть/показать список команд, фокус не меняется.
    fn toggle_commands(&mut self) {
        self.show_commands = !self.show_commands;
        if !self.show_commands && self.focus == ReaderFocus::Commands {
            self.focus = ReaderFocus::Text;
        }
        self.reflow();
    }

    /// Shift+4: показать команды и встать в список.
    fn focus_commands(&mut self) {
        let shown = self.show_commands;
        self.show_commands = true;
        self.focus = ReaderFocus::Commands;
        if !shown {
            self.reflow();
        }
    }

    /// Перестроить раскладку под новую ширину колонок, сохранив позицию чтения.
    fn reflow(&mut self) {
        let anchor = self.anchor();
        self.apply_layout(anchor);
    }

    /// `b`: курсор на строках видимой области — куда прикрепится заметка.
    fn start_note_pick(&mut self) {
        if self.book_id.is_none() || self.store.is_none() {
            return;
        }
        let row =
            self.clamp_to_viewport(anchor_to_scroll(&self.layout, self.document(), self.anchor()));
        self.pick = Some(Pick { kind: PickKind::Note, row, base: row });
        self.set_notice(": ↑/↓ строка · Enter — заметка · Esc".to_owned());
    }

    /// `v`: визуальное выделение от текущей позиции чтения.
    fn start_select(&mut self) {
        let row =
            self.clamp_to_viewport(anchor_to_scroll(&self.layout, self.document(), self.anchor()));
        self.pick = Some(Pick { kind: PickKind::Select, row, base: row });
        self.set_notice(": ↑/↓ выделение · Enter/y копировать · Esc".to_owned());
    }

    fn clamp_to_viewport(&self, row: usize) -> usize {
        let bottom = self.scroll + self.viewport_height().saturating_sub(1);
        row.clamp(self.scroll, bottom.min(self.layout.len().saturating_sub(1)))
    }

    /// Строка курсора pick-режима — для подсветки в рендере.
    pub fn pick_row(&self) -> Option<usize> {
        self.pick.as_ref().map(|p| p.row)
    }

    /// Диапазон выделения (топ..=низ строк раскладки) в режиме `v`.
    pub fn selection_range(&self) -> Option<(usize, usize)> {
        match &self.pick {
            Some(Pick { kind: PickKind::Select, row, base }) => {
                Some((*row.min(base), *row.max(base)))
            }
            _ => None,
        }
    }

    /// Подсказка в слоте, пока курсор pick активен.
    pub fn pick_hint(&self) -> Option<&'static str> {
        match &self.pick {
            Some(Pick { kind: PickKind::Note, .. }) => {
                Some("↑/↓ строка · Enter — заметка · Esc — отмена")
            }
            Some(Pick { kind: PickKind::Select, .. }) => {
                Some("↑/↓ выделение · Enter/y — копировать · Esc — отмена")
            }
            None => None,
        }
    }

    /// Последний скопированный фрагмент — для тестов.
    pub fn copied_text(&self) -> Option<&str> {
        self.copied.as_deref()
    }

    /// Забрать текст, который надо отправить терминалу (OSC 52).
    pub fn take_clipboard(&mut self) -> Option<String> {
        self.clipboard_out.take()
    }

    fn move_pick(&mut self, delta: isize) {
        let Some(p) = &self.pick else { return };
        let kind = p.kind;
        let last = self.layout.len().saturating_sub(1);
        let row = (p.row as isize + delta).clamp(0, last as isize) as usize;
        let row = if kind == PickKind::Note { self.clamp_to_viewport(row) } else { row };
        let pick = self.pick.as_mut().expect("pick активен");
        pick.row = row;
        if kind == PickKind::Select {
            let vp = self.viewport_height();
            if row < self.scroll {
                self.scroll = row;
            } else if row >= self.scroll + vp {
                self.scroll = (row + 1).saturating_sub(vp);
            }
        }
    }

    fn handle_pick_key(&mut self, key: KeyEvent) {
        let kind = self.pick.as_ref().expect("pick активен").kind;
        match key.code {
            KeyCode::Esc => self.pick = None,
            KeyCode::Char('j') | KeyCode::Down => self.move_pick(1),
            KeyCode::Char('k') | KeyCode::Up => self.move_pick(-1),
            KeyCode::Enter => match kind {
                PickKind::Note => self.finish_note_pick(),
                PickKind::Select => self.copy_selection(),
            },
            // Зажатый/повторный `v`: якорь выделения переносится на курсор.
            KeyCode::Char('v') if kind == PickKind::Select => {
                if let Some(pick) = &mut self.pick {
                    pick.base = pick.row;
                }
                self.set_notice(": якорь здесь · ↑/↓ тянуть · y копировать · Esc".to_owned());
            }
            KeyCode::Char('y') if kind == PickKind::Select => self.copy_selection(),
            // Остальные клавиши гасят курсор и обрабатываются как обычные.
            _ => {
                self.pick = None;
                self.handle_key(key);
            }
        }
    }

    /// Подтвердить строку заметки: открыть ввод метки с якорем этой строки.
    fn finish_note_pick(&mut self) {
        let Some(pick) = self.pick.take() else { return };
        let anchor = scroll_to_anchor(&self.layout, self.document(), pick.row);
        self.typing = Some(Typing {
            purpose: InputPurpose::NewBookmark,
            buffer: String::new(),
            bookmark_id: None,
            anchor: Some(anchor),
            color: DEFAULT_NOTE_COLOR,
        });
    }

    /// Скопировать выделенные строки: текст — в системный буфер через OSC 52.
    fn copy_selection(&mut self) {
        let Some((a, b)) = self.selection_range() else { return };
        let doc = self.document();
        let mut out = String::new();
        let mut prev_block: Option<usize> = None;
        for row in a..=b {
            let Some(info) = self.layout.line(row) else { continue };
            let Some(block) = doc.block(info.block) else { continue };
            let text = info.slice(block).trim_end();
            match prev_block {
                Some(prev) if prev == info.block && !out.ends_with('\n') => out.push(' '),
                Some(_) => out.push('\n'),
                None => {}
            }
            out.push_str(text);
            prev_block = Some(info.block);
        }
        let text = out.trim_matches(|c| c == ' ' || c == '\n').to_owned();
        let rows = b - a + 1;
        self.copied = Some(text.clone());
        self.clipboard_out = Some(text);
        self.pick = None;
        self.set_notice(format!(": скопировано строк {rows}"));
    }

    /// Поставить курсор оглавления на раздел, в котором читаем.
    fn sync_toc_cursor(&mut self) {
        let block = self.anchor().block;
        self.toc_cursor =
            self.document().toc().iter().rposition(|item| item.block <= block).unwrap_or(0);
    }

    /// Поставить курсор закладок на ближайшую к текущей позиции.
    fn sync_bookmark_cursor(&mut self) {
        let block = self.anchor().block;
        self.bookmark_cursor = self
            .bookmarks
            .iter()
            .enumerate()
            .min_by_key(|(_, b)| b.anchor.block.abs_diff(block))
            .map(|(i, _)| i)
            .unwrap_or(0);
    }

    /// Открыть командную строку ex-команд (`:` или `5`).
    fn start_command(&mut self) {
        self.typing = Some(Typing {
            purpose: InputPurpose::Command,
            buffer: String::new(),
            bookmark_id: None,
            anchor: None,
            color: 0,
        });
    }

    fn handle_toc_key(&mut self, key: KeyEvent) {
        let last = self.document().toc().len().saturating_sub(1);
        match key.code {
            KeyCode::Esc => self.focus = ReaderFocus::Text,
            KeyCode::Char('j') | KeyCode::Down => self.toc_cursor = (self.toc_cursor + 1).min(last),
            KeyCode::Char('k') | KeyCode::Up => self.toc_cursor = self.toc_cursor.saturating_sub(1),
            KeyCode::Enter => {
                if let Some(item) = self.document().toc().get(self.toc_cursor).cloned() {
                    let base = self.translate(Anchor::at_block(item.block), 0);
                    self.goto_anchor(base);
                }
                self.focus = ReaderFocus::Text;
            }
            // `q` здесь не выходит из приложения — сначала вернись к тексту.
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
                    color: 0,
                });
            }
            KeyCode::Char('d') => self.delete_selected(),
            _ => {}
        }
    }

    fn handle_bookmarks_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => self.focus = ReaderFocus::Text,
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
                    self.focus = ReaderFocus::Text;
                }
            }
            KeyCode::Char('r') => {
                if let Some(bookmark) = self.bookmarks.get(self.bookmark_cursor) {
                    self.typing = Some(Typing {
                        purpose: InputPurpose::RenameBookmark,
                        buffer: String::new(),
                        bookmark_id: Some(bookmark.id),
                        anchor: None,
                        color: 0,
                    });
                }
            }
            KeyCode::Char('c') => self.recolor_by(1),
            KeyCode::Char('C') => self.recolor_by(-1),
            KeyCode::Char('D') => self.delete_selected_bookmark(),
            _ => {}
        }
    }

    /// Список полезных команд правой колонки: Enter подставляет команду в `:`.
    fn handle_commands_key(&mut self, key: KeyEvent) {
        let last = COMMANDS.len().saturating_sub(1);
        match key.code {
            KeyCode::Esc => self.focus = ReaderFocus::Text,
            KeyCode::Char('j') | KeyCode::Down => {
                self.command_cursor = (self.command_cursor + 1).min(last);
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.command_cursor = self.command_cursor.saturating_sub(1);
            }
            KeyCode::Enter => {
                let Some((name, args, _)) = COMMANDS.get(self.command_cursor) else {
                    self.focus = ReaderFocus::Text;
                    return;
                };
                self.start_command();
                if let Some(typing) = &mut self.typing {
                    typing.buffer =
                        if args.is_empty() { name.to_string() } else { format!("{name} ") };
                }
            }
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
            // В окне создания заметки `c`/`C` выбирают цвет, а не набираются.
            KeyCode::Char('c') | KeyCode::Char('C')
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && matches!(
                        self.typing,
                        Some(Typing { purpose: InputPurpose::NewBookmark, .. })
                    ) =>
            {
                self.cycle_pending_color(if key.code == KeyCode::Char('c') { 1 } else { -1 });
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                if let Some(t) = &mut self.typing {
                    t.buffer.push(c);
                }
            }
            _ => {}
        }
    }

    /// Цикл цветов создаваемой заметки прямо в окне ввода.
    fn cycle_pending_color(&mut self, step: i8) {
        let Some(t) = &mut self.typing else { return };
        let next = (i32::from(t.color) + i32::from(step)).rem_euclid(7) as u8;
        t.color = next;
        self.set_notice(format!("цвет: {}", NOTE_COLOR_NAMES[usize::from(next)]));
    }

    /// Выбранный цвет заметки в окне создания — для блока «Заметка».
    pub fn pending_bookmark_color(&self) -> Option<u8> {
        match &self.typing {
            Some(Typing { purpose: InputPurpose::NewBookmark, color, .. }) => Some(*color),
            _ => None,
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
                    && !typing.buffer.trim().is_empty()
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
                if store.add_bookmark(book_id, anchor, &label, typing.color).is_ok() {
                    self.reload_bookmarks();
                    self.sync_bookmark_cursor();
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
            "toc" => {
                self.toggle_toc();
                Some(format!(": главы {}", if self.show_toc { "вкл" } else { "выкл" }))
            }
            "notes" => {
                self.toggle_bookmarks();
                Some(format!(": заметки {}", if self.show_bookmarks { "вкл" } else { "выкл" }))
            }
            "panels" => {
                self.toggle_panels();
                Some(format!(": колонки {}", if self.show_toc { "вкл" } else { "только текст" }))
            }
            "wider" => {
                self.resize_column(COL_STEP);
                Some(format!(": колонка {}", self.layout().width()))
            }
            "narrower" => {
                self.resize_column(-COL_STEP);
                Some(format!(": колонка {}", self.layout().width()))
            }
            "help" => {
                self.help_open = true;
                Some(": справка".to_owned())
            }
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
        self.typing = None;
        self.shelf_error = None;
        self.bookmarks.clear();
        self.reload_shelf();
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
            color: DEFAULT_NOTE_COLOR,
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

    /// Заметка, чей якорь лежит на конкретной строке раскладки: маркер▎
    /// подсвечивает только строку с началом заметки, а не весь абзац.
    pub fn note_line_color(&self, info: &LineInfo) -> Option<u8> {
        let chars = self.document().block(info.block).map_or(0, Block::char_len);
        self.bookmarks.iter().find_map(|b| {
            if b.anchor.block != info.block {
                return None;
            }
            let offset = match chars {
                0 => 0,
                chars => (chars as f32 * b.anchor.frac).round() as usize,
            };
            (offset >= info.start_char && offset < info.end_char.max(info.start_char + 1))
                .then_some(b.color)
        })
    }

    /// Имя цвета заметки для блока «Заметка».
    pub fn note_color_name(color: u8) -> &'static str {
        NOTE_COLOR_NAMES[usize::from(color.min(6))]
    }

    /// Индекс раздела в оглавлении, в котором сейчас читается.
    pub fn active_heading(&self) -> Option<usize> {
        let block = self.anchor().block;
        self.document().toc().iter().rposition(|item| item.block <= block)
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
