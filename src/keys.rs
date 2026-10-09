//! Явная таблица клавиш активного окна ридера: одна привязка — одна команда,
//! плюс метка для бара. Сборка соблюдает приоритет как в старом диспетчере:
//! цифры панелей → глобальные клавиши → команды окна. Из тех же привязок
//! рендер бара строит подсказку, поэтому клавиша и её описание не расходятся.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::ReaderFocus;

/// Клавиша с опциональным требованием модификаторов. `None` — модификатор не
/// проверяется (как в прежних `handle_*`: учитывались только явные ctrl/shift).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyPattern {
    pub code: KeyCode,
    pub ctrl: Option<bool>,
    pub shift: Option<bool>,
}

impl KeyPattern {
    pub const fn new(code: KeyCode) -> Self {
        Self { code, ctrl: None, shift: None }
    }
    pub const fn ctrl(code: KeyCode) -> Self {
        Self { code, ctrl: Some(true), shift: None }
    }
    pub const fn shift(code: KeyCode) -> Self {
        Self { code, ctrl: None, shift: Some(true) }
    }
    pub const fn plain(code: KeyCode) -> Self {
        Self { code, ctrl: Some(false), shift: Some(false) }
    }

    pub fn matches(&self, key: KeyEvent) -> bool {
        if key.code != self.code {
            return false;
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        self.ctrl.is_none_or(|v| v == ctrl) && self.shift.is_none_or(|v| v == shift)
    }
}

/// Все команды окон ридера. Применяются в `crate::app` матчем `KeyCmd -> App`,
/// поэтому тут нет доступа к полям приложения.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyCmd {
    // Переключатели панелей (цифры и алиасы, из любого окна).
    FocusText,
    FocusToc,
    FocusBookmarks,
    FocusCommands,
    TogglePanels,
    ToggleToc,
    ToggleBookmarks,
    ToggleCommands,
    // Глобальные клавиши.
    Quit,
    GoShelf,
    Help,
    StartSearch,
    EscSearchPanel,
    CloseSearch,
    SearchNext,
    SearchPrev,
    // Окно текста.
    ScrollDown,
    ScrollUp,
    ScrollPageDown,
    ScrollPageUp,
    ScrollHalfDown,
    ScrollHalfUp,
    ScrollTop,
    ScrollBottom,
    NextLang,
    StartCommand,
    StartNotePick,
    StartSelect,
    JumpBookmarkNext,
    JumpBookmarkPrev,
    NarrowColumn,
    WidenColumn,
    // Окно глав.
    TocCursorDown,
    TocCursorUp,
    TocEnter,
    // Окно заметок.
    BookmarkCursorDown,
    BookmarkCursorUp,
    BookmarkEnter,
    RenameBookmark,
    RecolorNext,
    RecolorPrev,
    DeleteBookmark,
    // Окно команд.
    CommandCursorDown,
    CommandCursorUp,
    CommandEnter,
    // Окно результатов поиска.
    SearchCursorDown,
    SearchCursorUp,
    SearchEnter,
}

/// Приоритет группы: цифры панелей проверяются раньше глобальных, те — раньше
/// команд окна. Бар показывает только Global и Pane, в этом порядке снизу.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    Digits,
    Global,
    Pane,
}

/// Одна привязка: клавиша, команда и описание для бара (`None` — не показывать).
#[derive(Debug, Clone)]
pub struct Binding {
    pub key: KeyPattern,
    pub cmd: KeyCmd,
    pub group: Group,
    /// Что появляется в клавиатурном баре: ранг порядка и текст.
    pub bar: Option<Bar>,
    /// Глобальная подсказка для правого верхнего угла (вместо бара).
    pub corner: Option<Corner>,
}

/// Угловая подсказка: `S полка` в правом верхнем углу, вне нижнего бара.
#[derive(Debug, Clone, Copy)]
pub struct Corner {
    pub label: &'static str,
    pub desc: &'static str,
}

/// Строка подсказки бара: чем меньше `order`, тем левее; равные — по порядку
/// привязок (панельные команды идут раньше глобальных).
#[derive(Debug, Clone, Copy)]
pub struct Bar {
    pub order: u8,
    pub label: &'static str,
    pub desc: Option<&'static str>,
}

fn bind(key: KeyPattern, cmd: KeyCmd, group: Group) -> Binding {
    Binding { key, cmd, group, bar: None, corner: None }
}

fn bar(
    key: KeyPattern,
    cmd: KeyCmd,
    group: Group,
    label: &'static str,
    desc: &'static str,
) -> Binding {
    Binding { key, cmd, group, bar: Some(Bar { order: 0, label, desc: Some(desc) }), corner: None }
}

fn bar_no_desc(key: KeyPattern, cmd: KeyCmd, group: Group, label: &'static str) -> Binding {
    Binding { key, cmd, group, bar: Some(Bar { order: 0, label, desc: None }), corner: None }
}

/// Глобальная подсказка с явным рангом: глобальные клавиши в баре идут после
/// команд окна и в фиксированном порядке (n/N, Esc, S, h, Q).
fn gbar(
    order: u8,
    key: KeyPattern,
    cmd: KeyCmd,
    label: &'static str,
    desc: &'static str,
) -> Binding {
    Binding {
        key,
        cmd,
        group: Group::Global,
        bar: Some(Bar { order, label, desc: Some(desc) }),
        corner: None,
    }
}

/// Глобальная клавиша, чья подсказка живёт в правом верхнем углу, а не в баре.
fn corner(key: KeyPattern, cmd: KeyCmd, label: &'static str, desc: &'static str) -> Binding {
    Binding { key, cmd, group: Group::Global, bar: None, corner: Some(Corner { label, desc }) }
}

/// Первая привязка, совпавшая с клавишей: единый порядок — сам список.
pub fn match_key(bindings: &[Binding], key: KeyEvent) -> Option<KeyCmd> {
    bindings.iter().find(|b| b.key.matches(key)).map(|b| b.cmd)
}

/// Привязки, активные в текущем окне ридера: цифры → глобальные → окно.
/// Метки бара заполнены под состояние, поэтому рендер не дублирует условия.
pub fn current_bindings(focus: ReaderFocus, search_active: bool) -> Vec<Binding> {
    let mut out = Vec::new();
    out.extend(digits(search_active));
    out.extend(globals(focus, search_active));
    out.extend(pane(focus, search_active));
    out
}

/// Цифры 1–4 фокусируют/прячут постоянные колонки из любого окна.
/// Shift+3/4 и их кириллические синонимы при активном поиске не трогают
/// флаги блоков — правая колонка занята результатами.
fn digits(search_active: bool) -> Vec<Binding> {
    let mut v = vec![
        bind(KeyPattern::plain(KeyCode::Char('1')), KeyCmd::FocusText, Group::Digits),
        bind(KeyPattern::shift(KeyCode::Char('1')), KeyCmd::TogglePanels, Group::Digits),
        bind(KeyPattern::new(KeyCode::Char('!')), KeyCmd::TogglePanels, Group::Digits),
        bind(KeyPattern::plain(KeyCode::Char('2')), KeyCmd::FocusToc, Group::Digits),
        bind(KeyPattern::shift(KeyCode::Char('2')), KeyCmd::ToggleToc, Group::Digits),
        bind(KeyPattern::new(KeyCode::Char('@')), KeyCmd::ToggleToc, Group::Digits),
        bind(KeyPattern::new(KeyCode::Char('"')), KeyCmd::ToggleToc, Group::Digits),
        bind(KeyPattern::new(KeyCode::Char('o')), KeyCmd::FocusToc, Group::Digits),
        bind(KeyPattern::plain(KeyCode::Char('3')), KeyCmd::FocusBookmarks, Group::Digits),
    ];
    if !search_active {
        v.push(bind(KeyPattern::shift(KeyCode::Char('3')), KeyCmd::ToggleBookmarks, Group::Digits));
        v.push(bind(KeyPattern::new(KeyCode::Char('#')), KeyCmd::ToggleBookmarks, Group::Digits));
        v.push(bind(KeyPattern::new(KeyCode::Char('№')), KeyCmd::ToggleBookmarks, Group::Digits));
    }
    v.push(bind(KeyPattern::new(KeyCode::Char('B')), KeyCmd::FocusBookmarks, Group::Digits));
    v.push(bind(KeyPattern::plain(KeyCode::Char('4')), KeyCmd::FocusCommands, Group::Digits));
    if !search_active {
        v.push(bind(KeyPattern::shift(KeyCode::Char('4')), KeyCmd::ToggleCommands, Group::Digits));
        v.push(bind(KeyPattern::new(KeyCode::Char('$')), KeyCmd::ToggleCommands, Group::Digits));
        v.push(bind(KeyPattern::new(KeyCode::Char(';')), KeyCmd::ToggleCommands, Group::Digits));
    }
    v
}

/// Глобальные клавиши: Q/S — только заглавные (строчные q/s в ридере не
/// работают). Их подсказки уезжают в правый верхний угол (`corner`), поэтому
/// в нижний бар не попадают. Двухступенчатый Esc поиска и n/N живут в этом
/// слое, чтобы перекрывать команды окна.
fn globals(focus: ReaderFocus, search_active: bool) -> Vec<Binding> {
    let mut v = vec![
        corner(KeyPattern::new(KeyCode::Char('S')), KeyCmd::GoShelf, "S", "полка"),
        corner(KeyPattern::new(KeyCode::Char('Q')), KeyCmd::Quit, "Q", "выход"),
        bind(KeyPattern::ctrl(KeyCode::Char('c')), KeyCmd::Quit, Group::Global),
        bind(KeyPattern::new(KeyCode::Char('h')), KeyCmd::Help, Group::Global),
    ];
    if focus == ReaderFocus::Text && !search_active {
        v.push(gbar(40, KeyPattern::new(KeyCode::Char('h')), KeyCmd::Help, "h", "справка"));
    }
    v.push(bind(KeyPattern::new(KeyCode::Char('/')), KeyCmd::StartSearch, Group::Global));
    // Двухступенчатый Esc: из панели результатов — к тексту, иначе закрыть поиск.
    if focus == ReaderFocus::SearchResults {
        v.push(gbar(
            20,
            KeyPattern::new(KeyCode::Esc),
            KeyCmd::EscSearchPanel,
            "Esc",
            "закрыть поиск",
        ));
    }
    if search_active && focus != ReaderFocus::SearchResults {
        let b = bind(KeyPattern::new(KeyCode::Esc), KeyCmd::CloseSearch, Group::Global);
        let b = if focus == ReaderFocus::Text {
            Binding {
                key: b.key,
                cmd: b.cmd,
                group: b.group,
                bar: Some(Bar {
                    order: 20, label: "Esc", desc: Some("— закрыть поиск")
                }),
                corner: None,
            }
        } else {
            b
        };
        v.push(b);
    }
    if search_active {
        let n = bind(KeyPattern::new(KeyCode::Char('n')), KeyCmd::SearchNext, Group::Global);
        let n = if focus == ReaderFocus::Text {
            Binding {
                key: n.key,
                cmd: n.cmd,
                group: n.group,
                bar: Some(Bar { order: 10, label: "n/N", desc: Some("дальше/назад") }),
                corner: None,
            }
        } else {
            n
        };
        v.push(n);
        v.push(bind(KeyPattern::new(KeyCode::Char('N')), KeyCmd::SearchPrev, Group::Global));
    }
    v
}

/// Подсказки правого верхнего угла из тех же привязок:
/// `[("S", "полка"), ("Q", "выход")]`.
pub fn corner_hints(focus: ReaderFocus, search_active: bool) -> Vec<(&'static str, &'static str)> {
    current_bindings(focus, search_active)
        .into_iter()
        .filter_map(|b| b.corner.map(|c| (c.label, c.desc)))
        .collect()
}

/// Команды активного окна. `bar` заполняется под состояние: для текста при
/// поиске язык скрыт из бара, а заметка/выделение остаются видимыми.
fn pane(focus: ReaderFocus, search_active: bool) -> Vec<Binding> {
    match focus {
        ReaderFocus::Text => text_pane(search_active),
        ReaderFocus::Toc => toc_pane(),
        ReaderFocus::Bookmarks => bookmarks_pane(),
        ReaderFocus::Commands => commands_pane(),
        ReaderFocus::SearchResults => search_pane(),
    }
}

fn text_pane(search_active: bool) -> Vec<Binding> {
    let mut v = vec![
        bind(KeyPattern::new(KeyCode::Char('j')), KeyCmd::ScrollDown, Group::Pane),
        bind(KeyPattern::new(KeyCode::Down), KeyCmd::ScrollDown, Group::Pane),
        bind(KeyPattern::new(KeyCode::Char('k')), KeyCmd::ScrollUp, Group::Pane),
        bind(KeyPattern::new(KeyCode::Up), KeyCmd::ScrollUp, Group::Pane),
        bind(KeyPattern::new(KeyCode::Char(' ')), KeyCmd::ScrollPageDown, Group::Pane),
        bind(KeyPattern::new(KeyCode::PageDown), KeyCmd::ScrollPageDown, Group::Pane),
        bind(KeyPattern::new(KeyCode::PageUp), KeyCmd::ScrollPageUp, Group::Pane),
        bind(KeyPattern::ctrl(KeyCode::Char('d')), KeyCmd::ScrollHalfDown, Group::Pane),
        bind(KeyPattern::ctrl(KeyCode::Char('u')), KeyCmd::ScrollHalfUp, Group::Pane),
        bind(KeyPattern::new(KeyCode::Char('g')), KeyCmd::ScrollTop, Group::Pane),
        bind(KeyPattern::new(KeyCode::Char('G')), KeyCmd::ScrollBottom, Group::Pane),
    ];
    if search_active {
        v.push(bind(KeyPattern::new(KeyCode::Char('t')), KeyCmd::NextLang, Group::Pane));
        v.push(bar(
            KeyPattern::new(KeyCode::Char('b')),
            KeyCmd::StartNotePick,
            Group::Pane,
            "b",
            "заметка",
        ));
        v.push(bar(
            KeyPattern::new(KeyCode::Char('v')),
            KeyCmd::StartSelect,
            Group::Pane,
            "v",
            "выд",
        ));
    } else {
        v.push(bar(
            KeyPattern::new(KeyCode::Char('t')),
            KeyCmd::NextLang,
            Group::Pane,
            "t",
            "язык",
        ));
        v.push(bar(
            KeyPattern::new(KeyCode::Char('b')),
            KeyCmd::StartNotePick,
            Group::Pane,
            "b",
            "заметка",
        ));
        v.push(bar(
            KeyPattern::new(KeyCode::Char('v')),
            KeyCmd::StartSelect,
            Group::Pane,
            "v",
            "выд",
        ));
    }
    v.push(bind(KeyPattern::new(KeyCode::Char(':')), KeyCmd::StartCommand, Group::Pane));
    v.push(bind(KeyPattern::new(KeyCode::Char('5')), KeyCmd::StartCommand, Group::Pane));
    // `?` — как `h`, только в окне текста (в баре не показывается).
    v.push(bind(KeyPattern::new(KeyCode::Char('?')), KeyCmd::Help, Group::Pane));
    if !search_active {
        v.push(bind(KeyPattern::new(KeyCode::Char('n')), KeyCmd::JumpBookmarkNext, Group::Pane));
        v.push(bind(KeyPattern::new(KeyCode::Char('p')), KeyCmd::JumpBookmarkPrev, Group::Pane));
    }
    v.push(bind(KeyPattern::new(KeyCode::Char('[')), KeyCmd::NarrowColumn, Group::Pane));
    v.push(bind(KeyPattern::new(KeyCode::Char(']')), KeyCmd::WidenColumn, Group::Pane));
    v
}

fn toc_pane() -> Vec<Binding> {
    vec![
        bar_no_desc(KeyPattern::new(KeyCode::Char('j')), KeyCmd::TocCursorDown, Group::Pane, "j/k"),
        bind(KeyPattern::new(KeyCode::Down), KeyCmd::TocCursorDown, Group::Pane),
        bar_no_desc(KeyPattern::new(KeyCode::Char('k')), KeyCmd::TocCursorUp, Group::Pane, "j/k"),
        bind(KeyPattern::new(KeyCode::Up), KeyCmd::TocCursorUp, Group::Pane),
        bar(KeyPattern::new(KeyCode::Enter), KeyCmd::TocEnter, Group::Pane, "Enter", "— к разделу"),
        bind(KeyPattern::new(KeyCode::Esc), KeyCmd::FocusText, Group::Pane),
    ]
}

fn bookmarks_pane() -> Vec<Binding> {
    vec![
        bar_no_desc(
            KeyPattern::new(KeyCode::Char('j')),
            KeyCmd::BookmarkCursorDown,
            Group::Pane,
            "j/k",
        ),
        bind(KeyPattern::new(KeyCode::Down), KeyCmd::BookmarkCursorDown, Group::Pane),
        bar_no_desc(
            KeyPattern::new(KeyCode::Char('k')),
            KeyCmd::BookmarkCursorUp,
            Group::Pane,
            "j/k",
        ),
        bind(KeyPattern::new(KeyCode::Up), KeyCmd::BookmarkCursorUp, Group::Pane),
        bar(
            KeyPattern::new(KeyCode::Enter),
            KeyCmd::BookmarkEnter,
            Group::Pane,
            "Enter",
            "— к заметке",
        ),
        bar(
            KeyPattern::new(KeyCode::Char('r')),
            KeyCmd::RenameBookmark,
            Group::Pane,
            "r",
            "— ред.",
        ),
        bar(KeyPattern::new(KeyCode::Char('c')), KeyCmd::RecolorNext, Group::Pane, "c/C", "цвет"),
        bind(KeyPattern::new(KeyCode::Char('C')), KeyCmd::RecolorPrev, Group::Pane),
        bar(
            KeyPattern::new(KeyCode::Char('D')),
            KeyCmd::DeleteBookmark,
            Group::Pane,
            "D",
            "удалить",
        ),
        bind(KeyPattern::new(KeyCode::Esc), KeyCmd::FocusText, Group::Pane),
    ]
}

fn commands_pane() -> Vec<Binding> {
    vec![
        bar_no_desc(
            KeyPattern::new(KeyCode::Char('j')),
            KeyCmd::CommandCursorDown,
            Group::Pane,
            "j/k",
        ),
        bind(KeyPattern::new(KeyCode::Down), KeyCmd::CommandCursorDown, Group::Pane),
        bar_no_desc(
            KeyPattern::new(KeyCode::Char('k')),
            KeyCmd::CommandCursorUp,
            Group::Pane,
            "j/k",
        ),
        bind(KeyPattern::new(KeyCode::Up), KeyCmd::CommandCursorUp, Group::Pane),
        bar(
            KeyPattern::new(KeyCode::Enter),
            KeyCmd::CommandEnter,
            Group::Pane,
            "Enter",
            "— подставить",
        ),
        bind(KeyPattern::new(KeyCode::Esc), KeyCmd::FocusText, Group::Pane),
    ]
}

fn search_pane() -> Vec<Binding> {
    vec![
        bar_no_desc(
            KeyPattern::new(KeyCode::Char('j')),
            KeyCmd::SearchCursorDown,
            Group::Pane,
            "j/k",
        ),
        bind(KeyPattern::new(KeyCode::Down), KeyCmd::SearchCursorDown, Group::Pane),
        bar_no_desc(
            KeyPattern::new(KeyCode::Char('k')),
            KeyCmd::SearchCursorUp,
            Group::Pane,
            "j/k",
        ),
        bind(KeyPattern::new(KeyCode::Up), KeyCmd::SearchCursorUp, Group::Pane),
        bar(
            KeyPattern::new(KeyCode::Enter),
            KeyCmd::SearchEnter,
            Group::Pane,
            "Enter",
            "— к тексту",
        ),
    ]
}
