//! Состояние читалки: варианты, позиция, навигация, переключение языка, прогресс.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, UNIX_EPOCH};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::align::{Alignment, align};
use crate::cli::{find_sidecars, lang_from_filename};
use crate::config::{Config, Theme};
use crate::keys::{KeyCmd, current_bindings, match_key};
use crate::model::{Anchor, Block, Document, Layout, LineInfo, anchor_to_scroll, scroll_to_anchor};
use crate::parse;
use crate::store::{Bookmark, DEFAULT_NOTE_COLOR, Store, StoreError, document_hash};

/// Период автосохранения прогресса.
const SAVE_INTERVAL: Duration = Duration::from_secs(5);
const DEFAULT_WIDTH: u16 = 80;
const DEFAULT_HEIGHT: u16 = 24;
/// Резерв под префиксы блоков (цитаты, стихи) и скроллбар справа.
pub const TEXT_PAD: u16 = 5;
/// Внутренний горизонтальный отступ текста от рамки — та же геометрия у рендера.
pub const TEXT_INDENT: u16 = 1;
/// Шаг клавиш `[`/`]`: уже/шире колонка, и её минимальная ширина.
const COL_STEP: i16 = 4;
const COL_MIN: u16 = 20;
/// Пустые строки хвоста: документ докручивается на это число строк после
/// последней, проценты от них не зависят.
const SCROLL_TAIL: usize = 13;
/// Ширины постоянных колонок и минимальная ширина центральной рамки — та же
/// геометрия, что использует рендер `ui::reader`.
pub const LEFT_W: u16 = 36;
pub const RIGHT_W: u16 = 36;
pub const MIN_CENTER: u16 = 18;
/// Внешнее поле окна: рамка и колонки отступают от краёв терминала.
pub const WIN_PAD: u16 = 1;

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
    /// Поиск по тексту/закладкам/оглавлению.
    Search,
}

/// Предел ручного изменения колонки (`[`/`]`) — колонка не сдвинулась.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResizeLimit {
    Narrow,
    Wide,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchMode {
    Text,
    Bookmarks,
    Toc,
}

#[derive(Debug, Clone)]
pub enum SearchHit {
    Text {
        layout_index: usize,
        block: usize,
        line_in_block: usize,
        start_char: usize,
        end_char: usize,
    },
    Bookmark {
        bookmark_id: i64,
        index_in_list: usize,
    },
    Toc {
        toc_index: usize,
    },
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
    /// Правая колонка, живой список результатов поиска.
    SearchResults,
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
    ("theme", "list|set|save", "темы: список / смена / запись"),
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
    /// Колонка упёрлась в предел: подсказку показывает статус-бар.
    resize_limit: Option<ResizeLimit>,
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
    /// Цвета и настройки по умолчанию из конфига.
    config: Config,
    // Поиск
    search_active: bool,
    search_query: String,
    search_mode: SearchMode,
    search_hits: Vec<SearchHit>,
    search_index: usize,
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
    /// Открыть книгу, определив базовый язык автоматически: суффикс имени
    /// (`книга.ru.md`), метаданные EPUB (`dc:language`) или эвристику текста
    /// (кириллица/латиница). Распознаём только `en` и `ru`.
    pub fn load_auto(
        path: &Path,
        extra: &[(String, PathBuf)],
        store: Option<Store>,
    ) -> Result<Self, AppError> {
        Self::load_auto_with(path, extra, store, Config::default())
    }

    /// То же с явной конфигурацией (пресет плюс файл из `main`).
    pub fn load_auto_with(
        path: &Path,
        extra: &[(String, PathBuf)],
        store: Option<Store>,
        config: Config,
    ) -> Result<Self, AppError> {
        let lang = lang_from_filename(path)
            .or_else(|| parse::detect_lang(path))
            .unwrap_or_else(|| "en".to_owned());
        Self::load_with(path, &lang, extra, store, config)
    }

    /// Загрузить книгу: база `base_lang`, сайдкары `книга.<lang>.*` и явные
    /// `--variant`. Выравнивание берётся из кэша хранилища или строится заново.
    pub fn load(
        path: &Path,
        base_lang: &str,
        extra: &[(String, PathBuf)],
        store: Option<Store>,
    ) -> Result<Self, AppError> {
        Self::load_with(path, base_lang, extra, store, Config::default())
    }

    /// То же с явной конфигурацией.
    pub fn load_with(
        path: &Path,
        base_lang: &str,
        extra: &[(String, PathBuf)],
        store: Option<Store>,
        config: Config,
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
        if let Some(id) = book_id
            && let Some(store) = &store
            && let Some(existing) = store.get_book(id)?
            && existing.base_lang != base_lang
        {
            store.update_base_lang(id, base_lang)?;
        }
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
            show_toc: config.defaults.toc,
            show_bookmarks: config.defaults.bookmarks,
            show_commands: config.defaults.commands,
            toc_cursor: 0,
            command_cursor: 0,
            focus: ReaderFocus::Text,
            help_open: false,
            col_extra: config.defaults.column_extra,
            resize_limit: None,
            pick: None,
            copied: None,
            clipboard_out: None,
            default_lang: base_lang.to_owned(),
            notice: None,
            notice_left: 0,
            config,
            search_active: false,
            search_query: String::new(),
            search_mode: SearchMode::Text,
            search_hits: Vec::new(),
            search_index: 0,
        };
        app.reload_bookmarks();
        Ok(app)
    }

    /// Полка: список книг из хранилища. Книга открывается клавишей `Enter`.
    pub fn shelf(store: Option<Store>, default_lang: &str) -> Result<Self, AppError> {
        Self::shelf_with(store, default_lang, Config::default())
    }

    /// Полка с явной конфигурацией.
    pub fn shelf_with(
        store: Option<Store>,
        default_lang: &str,
        config: Config,
    ) -> Result<Self, AppError> {
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
            show_toc: config.defaults.toc,
            show_bookmarks: config.defaults.bookmarks,
            show_commands: config.defaults.commands,
            toc_cursor: 0,
            command_cursor: 0,
            focus: ReaderFocus::Text,
            help_open: false,
            col_extra: config.defaults.column_extra,
            resize_limit: None,
            pick: None,
            copied: None,
            clipboard_out: None,
            default_lang: default_lang.to_owned(),
            notice: None,
            notice_left: 0,
            config,
            search_active: false,
            search_query: String::new(),
            search_mode: SearchMode::Text,
            search_hits: Vec::new(),
            search_index: 0,
        };
        app.reload_shelf();
        Ok(app)
    }

    /// Конфигурация приложения (тема и настройки по умолчанию).
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Цветовые роли текущей темы — ими рисуются бар, рамки и списки.
    pub fn colors(&self) -> &Theme {
        &self.config.theme
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

    /// Правая колонка нужна для заметок, команд или живого списка поиска.
    pub fn right_column_visible(&self) -> bool {
        self.show_bookmarks || self.show_commands || self.search_active
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

    /// Ширина рамки текста при текущих колонках — та же геометрия, что у рендера:
    /// колонки раскладываются внутри поля `WIN_PAD`, а не по краям окна.
    fn center_width(&self) -> u16 {
        let full = self.width.saturating_sub(2 * WIN_PAD);
        let right = self.right_column_visible();
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

    /// Ширина переноса: авто = внутренняя ширина рамки минус поля; `[`/`]`
    /// двигают колонку вокруг авто в пределах `column_limits`. Возвращает ширину
    /// строки текста, а не рамки — справа остаётся пустое место.
    pub fn wrap_width(&self) -> u16 {
        let (low, high) = self.column_limits();
        let base = self.column_base();
        (base + i64::from(self.col_extra)).clamp(i64::from(low), i64::from(high)) as u16
    }

    /// Диапазон ширины переноса при текущем окне: от минимальной колонки
    /// до ширины, у которой остаётся один столбец на скроллбар.
    fn column_limits(&self) -> (u16, u16) {
        let inner = self.center_width().saturating_sub(2).saturating_sub(2 * TEXT_INDENT);
        let high = inner.saturating_sub(1);
        (COL_MIN.min(high), high)
    }

    /// Автоширина переноса — база, вокруг которой ходят `[`/`]`.
    fn column_base(&self) -> i64 {
        i64::from(self.center_width().saturating_sub(2).saturating_sub(2 * TEXT_INDENT))
            - i64::from(TEXT_PAD)
    }

    /// В какой предел упёрлась ручная колонка — подсказку рисует статус-бар.
    pub fn column_limit(&self) -> Option<ResizeLimit> {
        self.resize_limit
    }

    /// Закладка на текущей строке — её метку показывает статус-бар. Метка
    /// завязывает на строку (якорь в строку через текущую раскладку), а не на весь блок.
    pub fn bookmark_at(&self) -> Option<&Bookmark> {
        let scroll = self.scroll;
        self.bookmarks
            .iter()
            .find(|b| anchor_to_scroll(&self.layout, self.document(), b.anchor) == scroll)
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
        self.content_max_scroll() + SCROLL_TAIL
    }

    /// Последняя строка содержимого (без пустого хвоста) — предел, на котором
    /// прогресс достигает 100%.
    pub fn content_max_scroll(&self) -> usize {
        self.layout.len().saturating_sub(self.viewport_height())
    }

    /// Новый размер окна: позиция переходом через якорь, сохраняя абзац на месте.
    pub fn set_size(&mut self, width: u16, height: u16) {
        let anchor = self.pending_anchor.take().unwrap_or_else(|| self.anchor());
        self.width = width;
        self.height = height;
        self.resize_limit = None;
        self.apply_layout(anchor);
    }

    /// Перестроить раскладку под текущую ширину переноса и вернуть якорь на место.
    fn apply_layout(&mut self, anchor: Anchor) {
        self.layout = Layout::new(self.document(), self.wrap_width());
        self.scroll = anchor_to_scroll(&self.layout, self.document(), anchor);
    }

    /// Клавиши `[`/`]`: уже/шире колонку с сохранением позиции чтения.
    /// У предела колонка не двигается и не перестраивается — статус-бар
    /// получает подсказку `resize_limit`.
    fn resize_column(&mut self, delta: i16) {
        let (low, high) = self.column_limits();
        let next = (i64::from(self.wrap_width()) + i64::from(delta))
            .clamp(i64::from(low), i64::from(high));
        if next == i64::from(self.wrap_width()) {
            self.resize_limit =
                Some(if delta > 0 { ResizeLimit::Wide } else { ResizeLimit::Narrow });
            return;
        }
        self.col_extra = (next - self.column_base()) as i16;
        self.resize_limit = None;
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
        } else if let Some(cmd) = match_key(&current_bindings(self.focus, self.search_active), key)
        {
            self.apply_cmd(cmd);
        }
    }

    /// Исполнить команду активного окна. Порядок и условия привязок задаёт
    /// `keys::current_bindings`, здесь — только эффект команды.
    fn apply_cmd(&mut self, cmd: KeyCmd) {
        match cmd {
            KeyCmd::FocusText => self.focus = ReaderFocus::Text,
            KeyCmd::FocusToc => self.focus_toc(),
            KeyCmd::FocusBookmarks => self.focus_bookmarks(),
            KeyCmd::FocusCommands => self.focus_commands(),
            KeyCmd::TogglePanels => self.toggle_panels(),
            KeyCmd::ToggleToc => self.toggle_toc(),
            KeyCmd::ToggleBookmarks => self.toggle_bookmarks(),
            KeyCmd::ToggleCommands => self.toggle_commands(),
            KeyCmd::Quit => self.quit = true,
            KeyCmd::GoShelf => self.go_shelf(),
            KeyCmd::Help => self.help_open = true,
            KeyCmd::StartSearch => self.start_search(),
            KeyCmd::EscSearchPanel => self.focus = ReaderFocus::Text,
            KeyCmd::CloseSearch => self.close_search(),
            KeyCmd::SearchNext => self.search_step(true),
            KeyCmd::SearchPrev => self.search_step(false),
            KeyCmd::ScrollDown => self.scroll_by(1),
            KeyCmd::ScrollUp => self.scroll_by(-1),
            KeyCmd::ScrollPageDown => self.scroll_by(self.viewport_height() as isize),
            KeyCmd::ScrollPageUp => self.scroll_by(-(self.viewport_height() as isize)),
            KeyCmd::ScrollHalfDown => self.scroll_by((self.viewport_height() / 2) as isize),
            KeyCmd::ScrollHalfUp => self.scroll_by(-((self.viewport_height() / 2) as isize)),
            KeyCmd::ScrollTop => self.scroll = 0,
            KeyCmd::ScrollBottom => self.scroll = self.max_scroll(),
            KeyCmd::NextLang => {
                self.next_lang();
            }
            KeyCmd::StartCommand => self.start_command(),
            KeyCmd::StartNotePick => self.start_note_pick(),
            KeyCmd::StartSelect => self.start_select(),
            KeyCmd::JumpBookmarkNext => self.jump_bookmark(true),
            KeyCmd::JumpBookmarkPrev => self.jump_bookmark(false),
            KeyCmd::NarrowColumn => self.resize_column(-COL_STEP),
            KeyCmd::WidenColumn => self.resize_column(COL_STEP),
            KeyCmd::TocCursorDown => {
                let last = self.document().toc().len().saturating_sub(1);
                self.toc_cursor = (self.toc_cursor + 1).min(last);
            }
            KeyCmd::TocCursorUp => self.toc_cursor = self.toc_cursor.saturating_sub(1),
            KeyCmd::TocEnter => {
                if let Some(item) = self.document().toc().get(self.toc_cursor).cloned() {
                    let base = self.translate(Anchor::at_block(item.block), 0);
                    self.goto_anchor(base);
                }
                self.focus = ReaderFocus::Text;
            }
            KeyCmd::BookmarkCursorDown => {
                self.bookmark_cursor =
                    (self.bookmark_cursor + 1).min(self.bookmarks.len().saturating_sub(1));
            }
            KeyCmd::BookmarkCursorUp => {
                self.bookmark_cursor = self.bookmark_cursor.saturating_sub(1);
            }
            KeyCmd::BookmarkEnter => {
                if let Some(bookmark) = self.bookmarks.get(self.bookmark_cursor).cloned() {
                    self.goto_anchor(bookmark.anchor);
                    self.focus = ReaderFocus::Text;
                }
            }
            KeyCmd::RenameBookmark => {
                if let Some(bookmark) = self.bookmarks.get(self.bookmark_cursor) {
                    self.typing = Some(Typing {
                        purpose: InputPurpose::RenameBookmark,
                        buffer: bookmark.label.clone(),
                        bookmark_id: Some(bookmark.id),
                        anchor: None,
                        color: 0,
                    });
                }
            }
            KeyCmd::RecolorNext => self.recolor_by(1),
            KeyCmd::RecolorPrev => self.recolor_by(-1),
            KeyCmd::DeleteBookmark => self.delete_selected_bookmark(),
            KeyCmd::CommandCursorDown => {
                let last = COMMANDS.len().saturating_sub(1);
                self.command_cursor = (self.command_cursor + 1).min(last);
            }
            KeyCmd::CommandCursorUp => {
                self.command_cursor = self.command_cursor.saturating_sub(1);
            }
            KeyCmd::CommandEnter => {
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
            KeyCmd::SearchCursorDown => self.move_search_cursor(1),
            KeyCmd::SearchCursorUp => self.move_search_cursor(-1),
            KeyCmd::SearchEnter => self.focus = ReaderFocus::Text,
        }
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

    /// Shift+3: показать заметки и встать в список. Пока идёт поиск —
    /// правая колонка занята результатами, фокус уходит в них.
    fn focus_bookmarks(&mut self) {
        if self.search_active {
            self.focus = ReaderFocus::SearchResults;
            return;
        }
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

    /// Shift+4: показать команды и встать в список. При активном поиске —
    /// фокус в результаты (колонка одна).
    fn focus_commands(&mut self) {
        if self.search_active {
            self.focus = ReaderFocus::SearchResults;
            return;
        }
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
        let max_row = self.layout.len().saturating_sub(1);
        let top = self.scroll.min(max_row);
        let bottom = self.scroll.saturating_add(self.viewport_height()).saturating_sub(1);
        row.clamp(top, bottom.min(max_row).max(top))
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

    /// Подсказки действий в слоте, пока курсор pick активен: пары
    /// `(клавиши, описание)`, рендер красит клавиши отдельно.
    pub fn pick_hint(&self) -> Option<&'static [(&'static str, &'static str)]> {
        match &self.pick {
            Some(Pick { kind: PickKind::Note, .. }) => {
                Some(&[("↑/↓", "строка"), ("Enter", "— заметка"), ("Esc", "— отмена")])
            }
            Some(Pick { kind: PickKind::Select, .. }) => {
                Some(&[("↑/↓", "выделение"), ("Enter/y", "— копировать"), ("Esc", "— отмена")])
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

    fn handle_help_key(&mut self, key: KeyEvent) {
        if matches!(key.code, KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('h')) {
            self.help_open = false;
        }
        // `q` в справке не выходит из приложения.
    }

    fn handle_shelf_key(&mut self, key: KeyEvent) {
        let control = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Char('Q') => self.quit = true,
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

    fn handle_typing(&mut self, key: KeyEvent) {
        let in_search = matches!(self.typing, Some(Typing { purpose: InputPurpose::Search, .. }));
        match key.code {
            KeyCode::Esc => {
                if in_search {
                    self.close_search();
                }
                self.typing = None;
                self.shelf_error = None;
            }
            // В поиске Tab переключает режим: текст → закладки → главы;
            // список результатов пересчитывается на лету.
            KeyCode::Tab if in_search => {
                self.search_mode = match self.search_mode {
                    SearchMode::Text => SearchMode::Bookmarks,
                    SearchMode::Bookmarks => SearchMode::Toc,
                    SearchMode::Toc => SearchMode::Text,
                };
                self.refresh_search();
            }
            // `↑`/`↓` в строке запроса ходят по живому списку совпадений.
            KeyCode::Down if in_search => self.move_search_cursor(1),
            KeyCode::Up if in_search => self.move_search_cursor(-1),
            KeyCode::Enter if in_search => self.commit_search(),
            KeyCode::Enter => self.commit_typing(),
            KeyCode::Backspace => {
                if let Some(t) = &mut self.typing {
                    t.buffer.pop();
                }
                if in_search {
                    self.refresh_search();
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
                if in_search {
                    self.refresh_search();
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
            // Enter в поиске уходит в commit_search раньше; ветка — для
            // полноты match (typing уже take()нут).
            InputPurpose::Search => {
                self.typing = Some(typing);
                self.commit_search();
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
            "theme" => self.command_theme(arg),
            other => Some(format!(": нет команды «{other}»")),
        }
    }

    /// Открыть поиск по `/`: строка запроса в слоте, режим Text.
    fn start_search(&mut self) {
        self.search_active = true;
        self.search_query.clear();
        self.search_hits.clear();
        self.search_index = 0;
        self.typing = Some(Typing {
            purpose: InputPurpose::Search,
            buffer: String::new(),
            bookmark_id: None,
            anchor: None,
            color: 0,
        });
    }

    /// Закрыть поиск: подсветка, панель и результаты гаснут.
    fn close_search(&mut self) {
        self.search_active = false;
        self.search_query.clear();
        self.search_hits.clear();
        self.search_index = 0;
        if self.focus == ReaderFocus::SearchResults {
            self.focus = ReaderFocus::Text;
        }
    }

    /// Живой фильтр: пересобрать совпадения по текущему буферу и режиму,
    /// не прыгая и не тостя. Курсор — первое совпадение после текущей
    /// позиции (Text), иначе начало списка.
    fn refresh_search(&mut self) {
        let query = self.typing_buffer().unwrap_or_default().to_owned();
        self.search_query = query.clone();
        self.search_hits.clear();
        self.search_index = 0;
        if query.is_empty() {
            return;
        }
        let needle = query.to_lowercase();
        match self.search_mode {
            SearchMode::Text => {
                let doc = self.document().clone();
                let layout = self.layout.clone();
                for (index, info) in layout.lines().iter().enumerate() {
                    let Some(block) = doc.block(info.block) else { continue };
                    let slice = info.slice(block);
                    let Some(offset) = slice.to_lowercase().find(&needle) else { continue };
                    let start_char = slice[..offset].chars().count();
                    let end_char =
                        start_char + slice[offset..].chars().count().min(needle.chars().count());
                    self.search_hits.push(SearchHit::Text {
                        layout_index: index,
                        block: info.block,
                        line_in_block: info.line_in_block,
                        start_char,
                        end_char,
                    });
                }
                self.search_index = self
                    .search_hits
                    .iter()
                    .position(|hit| matches!(hit, SearchHit::Text { layout_index, .. } if *layout_index > self.scroll))
                    .unwrap_or(0);
            }
            SearchMode::Bookmarks => {
                for (index, bookmark) in self.bookmarks.iter().enumerate() {
                    if bookmark.label.to_lowercase().contains(&needle) {
                        self.search_hits.push(SearchHit::Bookmark {
                            bookmark_id: bookmark.id,
                            index_in_list: index,
                        });
                    }
                }
            }
            SearchMode::Toc => {
                let titles: Vec<String> =
                    self.document().toc().iter().map(|item| item.title.clone()).collect();
                for (index, title) in titles.iter().enumerate() {
                    if title.to_lowercase().contains(&needle) {
                        self.search_hits.push(SearchHit::Toc { toc_index: index });
                    }
                }
            }
        }
    }

    /// `Enter` в строке поиска: прыжок к курсору списка, prompt закрывается,
    /// панель результатов и подсветка остаются.
    fn commit_search(&mut self) {
        let query = self.search_query.clone();
        if query.is_empty() {
            self.set_notice(": пустой запрос".to_owned());
        } else if self.search_hits.is_empty() {
            self.set_notice(format!(": не найдено «{query}»"));
        } else {
            self.goto_search_hit();
        }
        self.typing = None;
        self.focus = ReaderFocus::Text;
    }

    /// `↑`/`↓` в строке поиска и в панели: шаг курсора с зацикливанием
    /// и живым прыжком текста к совпадению.
    fn move_search_cursor(&mut self, delta: i32) {
        let len = self.search_hits.len();
        if len == 0 {
            return;
        }
        let current = self.search_index as i32;
        self.search_index = (current + delta).rem_euclid(len as i32) as usize;
        self.goto_search_hit();
    }

    /// `n`/`N`: следующий/предыдущий результат с зацикливанием.
    fn search_step(&mut self, forward: bool) {
        self.move_search_cursor(if forward { 1 } else { -1 });
    }

    /// Перейти к текущему результату: текст — центрируем строку вьюпортом,
    /// закладки и главы — штатным переходом по якорю.
    fn goto_search_hit(&mut self) {
        let Some(hit) = self.search_hits.get(self.search_index).cloned() else { return };
        match hit {
            SearchHit::Text { layout_index, .. } => {
                let viewport = self.viewport_height();
                let target = layout_index.saturating_sub(viewport / 2);
                self.set_scroll(target);
            }
            SearchHit::Bookmark { index_in_list, .. } => {
                if let Some(bookmark) = self.bookmarks.get(index_in_list).cloned() {
                    self.goto_anchor(bookmark.anchor);
                }
            }
            SearchHit::Toc { toc_index } => {
                if let Some(item) = self.document().toc().get(toc_index).cloned() {
                    let base = self.translate(Anchor::at_block(item.block), 0);
                    self.goto_anchor(base);
                }
            }
        }
    }

    /// Активен ли поиск — рендер подсвечивает текущее совпадение.
    pub fn search_active(&self) -> bool {
        self.search_active
    }

    /// Живой список совпадений для панели результатов и тестов.
    pub fn search_results(&self) -> &[SearchHit] {
        &self.search_hits
    }

    /// Сколько совпадений сейчас найдено.
    pub fn search_count(&self) -> usize {
        self.search_hits.len()
    }

    /// Курсор в списке результатов — индекс текущего совпадения.
    pub fn search_cursor(&self) -> usize {
        self.search_index
    }

    /// Текст строки списка результатов: строка раскладки с совпадением,
    /// метка заметки или заголовок главы — один формат для рендера и тестов.
    pub fn search_row_label(&self, index: usize) -> Option<String> {
        match self.search_hits.get(index)? {
            SearchHit::Text { layout_index, .. } => {
                let info = self.layout.line(*layout_index)?;
                let block = self.document().block(info.block)?;
                Some(info.slice(block).trim().to_owned())
            }
            SearchHit::Bookmark { index_in_list, .. } => {
                self.bookmarks.get(*index_in_list).map(|b| b.label.clone())
            }
            SearchHit::Toc { toc_index } => {
                self.document().toc().get(*toc_index).map(|item| item.title.clone())
            }
        }
    }

    /// Текущий режим поиска — префикс в слоте (`/t:`, `/b:`, `/o:`).
    pub fn search_mode(&self) -> SearchMode {
        self.search_mode
    }

    /// Последний подтверждённый запрос — для заголовка панели и тостов.
    pub fn search_query(&self) -> &str {
        &self.search_query
    }

    /// Строка раскладки текущего совпадения (для подсветки) — `None`, если
    /// текущий результат не из текста.
    pub fn search_line(&self) -> Option<usize> {
        if !self.search_active {
            return None;
        }
        match self.search_hits.get(self.search_index) {
            Some(SearchHit::Text { layout_index, .. }) => Some(*layout_index),
            _ => None,
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

    /// `:theme list|set <имя>|save` — просмотр, смена и запись текущей темы.
    fn command_theme(&mut self, arg: &str) -> Option<String> {
        let (action, name) = arg.split_once(' ').map_or((arg, ""), |(a, b)| (a, b.trim()));
        match action {
            "list" => Some(format!(": темы · {}", crate::config::PRESETS.join(", "))),
            "set" => {
                if name.is_empty() {
                    return Some(": theme set нужен пресет: btop, mono, light".to_owned());
                }
                match crate::config::load(Some(name)) {
                    Ok(config) => {
                        self.config = config;
                        Some(format!(": тема «{name}»"))
                    }
                    Err(error) => Some(format!(": {error}")),
                }
            }
            "save" => match crate::config::save(&self.config) {
                Ok(path) => Some(format!(": конфиг записан — {}", path.display())),
                Err(error) => Some(format!(": {error}")),
            },
            _ => Some(": нужен list, set <имя> или save".to_owned()),
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
        let path = Path::new(arg);
        let store = match &self.store {
            Some(store) => store.reopen().ok(),
            None => None,
        };
        match App::load_auto(path, &[], store) {
            Ok(mut app) => {
                let title = app.title().to_owned();
                app.default_lang = app.base_lang().to_owned();
                *self = app;
                self.set_notice(format!(": открыт «{title}»"));
                None
            }
            Err(e) => Some(format!(": не открыть «{arg}» — {e}")),
        }
    }

    fn register_book(&mut self, path: &Path) -> Result<(), String> {
        let lang = lang_from_filename(path)
            .or_else(|| parse::detect_lang(path))
            .unwrap_or_else(|| "en".to_owned());
        let doc = parse::load(path, &lang).map_err(|e| e.to_string())?;
        let title = doc.title().to_owned();
        let (mtime, size) = file_stamp(path);
        let key = path.display().to_string();
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
        self.close_search();
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
        let lines = self.config.defaults.wheel_lines;
        let delta = if up { -lines } else { lines };
        self.scroll_by(delta);
    }

    /// Прогресс чтения в процентах (0..=100).
    pub fn percent(&self) -> f32 {
        let max = self.content_max_scroll();
        if max == 0 { 100.0 } else { self.scroll.min(max) as f32 / max as f32 * 100.0 }
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
