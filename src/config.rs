//! Конфигурация: цвета (тема), настройки по умолчанию, пресеты и загрузка
//! из TOML. Файл опционален: нет его — действуют встроенные значения.

use std::path::{Path, PathBuf};

use ratatui::style::Color;
use serde::{Deserialize, Serialize};

/// Доступные пресеты: их принимает `--theme`.
pub const PRESETS: &[&str] = &["btop", "mono", "light"];

/// Ошибка разбора конфигурации — читаем fail-fast, не молчим о опечатках.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("не удалось прочитать {path}: {source}")]
    Io { path: PathBuf, source: std::io::Error },
    #[error("{path}: {source}")]
    Toml { path: PathBuf, source: toml::de::Error },
    #[error("{path}: не удалось записать: {source}")]
    Write { path: PathBuf, source: std::io::Error },
    #[error("{path}: не удалось сериализовать: {source}")]
    Encode { path: PathBuf, source: toml::ser::Error },
    #[error("{path}: неизвестный цвет «{value}» — имя (yellow, darkgray…) или #rrggbb")]
    Color { path: PathBuf, value: String },
    #[error("неизвестный пресет «{0}»: доступны btop, mono, light")]
    Preset(String),
    #[error("путь к конфигу не задан ($XDG_CONFIG_HOME)")]
    NoPath,
}

/// Обратное преобразование цвета: имя для именованной палитры, hex для RGB.
fn color_str(color: &Color) -> String {
    let named = match color {
        Color::Black => "black",
        Color::Red => "red",
        Color::Green => "green",
        Color::Yellow => "yellow",
        Color::Blue => "blue",
        Color::Magenta => "magenta",
        Color::Cyan => "cyan",
        Color::White => "white",
        Color::Gray => "gray",
        Color::DarkGray => "darkgray",
        Color::LightRed => "lightred",
        Color::LightGreen => "lightgreen",
        Color::LightYellow => "lightyellow",
        Color::LightBlue => "lightblue",
        Color::LightMagenta => "lightmagenta",
        Color::LightCyan => "lightcyan",
        Color::Reset => "reset",
        _ => {
            let (r, g, b) = to_rgb(*color);
            return format!("#{r:02x}{g:02x}{b:02x}");
        }
    };
    named.to_owned()
}

/// Имя цвета или hex-код в конфиге: `yellow`, `darkgrey`, `#rrggbb`, `#rgb`.
pub fn parse_color(raw: &str) -> Option<Color> {
    let value = raw.trim();
    let named = match value.to_ascii_lowercase().as_str() {
        "black" => Color::Black,
        "red" => Color::Red,
        "green" => Color::Green,
        "yellow" => Color::Yellow,
        "blue" => Color::Blue,
        "magenta" => Color::Magenta,
        "cyan" => Color::Cyan,
        "white" => Color::White,
        "gray" | "grey" => Color::Gray,
        "darkgray" | "darkgrey" => Color::DarkGray,
        "lightred" => Color::LightRed,
        "lightgreen" => Color::LightGreen,
        "lightyellow" => Color::LightYellow,
        "lightblue" => Color::LightBlue,
        "lightmagenta" => Color::LightMagenta,
        "lightcyan" => Color::LightCyan,
        "reset" => Color::Reset,
        _ => return parse_hex(value),
    };
    Some(named)
}

fn parse_hex(value: &str) -> Option<Color> {
    let digits = value.strip_prefix('#')?;
    let expanded = match digits.len() {
        3 => digits.chars().flat_map(|c| [c, c]).collect::<String>(),
        6 => digits.to_owned(),
        _ => return None,
    };
    if !expanded.is_ascii() || !expanded.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let channel = |range: std::ops::Range<usize>| u8::from_str_radix(&expanded[range], 16).ok();
    Some(Color::Rgb(channel(0..2)?, channel(2..4)?, channel(4..6)?))
}

/// Компоненты RGB — так градиент получает цвета темы.
pub type Rgb = (u8, u8, u8);

/// Цветовая роль бара, рамок и текста. Значения по умолчанию — палитра btop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Theme {
    /// Клавиши в баре, курсоры и маркеры списков (жёлтый).
    pub key: Color,
    /// Подсказки, бейджи и ошибки (жёлтый).
    pub accent: Color,
    /// Неактивные рамки и приглушённый текст (тёмно-серый).
    pub dim: Color,
    /// Активный блок, панель и цифра (белый).
    pub active: Color,
    /// Активная глава в оглавлении (голубой).
    pub heading: Color,
    /// Тост-успех (зелёный).
    pub notice: Color,
    /// Фон выделения строк и курсора выделения (тёмно-серый).
    pub selection_bg: Color,
    /// Линия-разделитель `---` (серый).
    pub rule: Color,
    /// Градиент прогресса: начало, конец, пустая клетка.
    pub gauge_start: Color,
    pub gauge_end: Color,
    pub gauge_empty: Color,
    /// Семь цветов заметки в порядке индексов хранилища.
    pub notes: [Color; 7],
}

impl Theme {
    /// Палитра по умолчанию: жёлтые клавиши, серые неактивные, пурпурный
    /// градиент прогресса — то, что рисуется без конфига.
    pub fn btop() -> Self {
        Self {
            key: Color::Yellow,
            accent: Color::Yellow,
            dim: Color::DarkGray,
            active: Color::White,
            heading: Color::Cyan,
            notice: Color::Green,
            selection_bg: Color::DarkGray,
            rule: Color::Gray,
            gauge_start: Color::Rgb(90, 30, 140),
            gauge_end: Color::Rgb(210, 90, 255),
            gauge_empty: Color::Rgb(50, 50, 50),
            notes: [
                Color::Red,
                Color::Green,
                Color::Yellow,
                Color::Blue,
                Color::Magenta,
                Color::Cyan,
                Color::White,
            ],
        }
    }

    /// Монохром для терминалов без палитры: всё серое, градиент — от
    /// тёмно-серого к белому.
    pub fn mono() -> Self {
        Self {
            key: Color::White,
            accent: Color::White,
            dim: Color::DarkGray,
            active: Color::White,
            heading: Color::White,
            notice: Color::White,
            selection_bg: Color::DarkGray,
            rule: Color::Gray,
            gauge_start: Color::Rgb(90, 90, 90),
            gauge_end: Color::Rgb(245, 245, 245),
            gauge_empty: Color::Rgb(50, 50, 50),
            notes: [
                Color::Rgb(255, 255, 255),
                Color::Rgb(215, 215, 215),
                Color::Rgb(185, 185, 185),
                Color::Rgb(155, 155, 155),
                Color::Rgb(125, 125, 125),
                Color::Rgb(95, 95, 95),
                Color::Rgb(65, 65, 65),
            ],
        }
    }

    /// Для светлых фонов: тёмные клавиши и текст, голубой акцент,
    /// тёплый градиент, белая заметка заменяется серой.
    pub fn light() -> Self {
        Self {
            key: Color::Blue,
            accent: Color::Red,
            dim: Color::Gray,
            active: Color::Black,
            heading: Color::Blue,
            notice: Color::Green,
            selection_bg: Color::Gray,
            rule: Color::Gray,
            gauge_start: Color::Rgb(0, 70, 150),
            gauge_end: Color::Rgb(0, 150, 255),
            gauge_empty: Color::Rgb(190, 190, 190),
            notes: [
                Color::Red,
                Color::Green,
                Color::Rgb(200, 120, 0),
                Color::Blue,
                Color::Magenta,
                Color::Cyan,
                Color::Gray,
            ],
        }
    }

    /// Градиент и пустая клетка в компонентах RGB — интерполяция их ищет.
    pub fn gauge_colors(&self) -> (Rgb, Rgb, Rgb) {
        (to_rgb(self.gauge_start), to_rgb(self.gauge_end), to_rgb(self.gauge_empty))
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::btop()
    }
}

fn to_rgb(color: Color) -> (u8, u8, u8) {
    match color {
        Color::Rgb(r, g, b) => (r, g, b),
        Color::Black => (0, 0, 0),
        Color::Red => (205, 0, 0),
        Color::Green => (0, 205, 0),
        Color::Yellow => (205, 205, 0),
        Color::Blue => (0, 0, 238),
        Color::Magenta => (205, 0, 205),
        Color::Cyan => (0, 205, 205),
        Color::White => (229, 229, 229),
        Color::Gray => (128, 128, 128),
        Color::DarkGray => (69, 69, 69),
        Color::LightRed => (255, 0, 0),
        Color::LightGreen => (0, 255, 0),
        Color::LightYellow => (255, 255, 0),
        Color::LightBlue => (92, 92, 255),
        Color::LightMagenta => (255, 0, 255),
        Color::LightCyan => (0, 255, 255),
        Color::Reset | Color::Indexed(_) => (128, 128, 128),
    }
}

/// Стартовые значения поведения: панели, шаг колеса, ширина колонки, градиент.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Defaults {
    /// Строк на прокрутку колесом мыши.
    pub wheel_lines: isize,
    /// Видны ли колонки при первом открытии.
    pub toc: bool,
    pub bookmarks: bool,
    pub commands: bool,
    /// Смещение ширины текстовой колонки от автоширины (`[`/`]`-сдвиг).
    pub column_extra: i16,
    /// Гасить ли текст у нижнего края.
    pub fade_text: bool,
}

impl Default for Defaults {
    fn default() -> Self {
        Self {
            wheel_lines: 3,
            toc: true,
            bookmarks: true,
            commands: true,
            column_extra: 0,
            fade_text: true,
        }
    }
}

/// Готовая конфигурация: тема плюс настройки по умолчанию.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Config {
    pub theme: Theme,
    pub defaults: Defaults,
}

/// Файл конфига: все поля опциональны, заданные перекрывают базу (пресет).
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ConfigFile {
    theme: ThemePatch,
    defaults: DefaultsPatch,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ThemePatch {
    key: Option<String>,
    accent: Option<String>,
    dim: Option<String>,
    active: Option<String>,
    heading: Option<String>,
    notice: Option<String>,
    selection_bg: Option<String>,
    rule: Option<String>,
    gauge_start: Option<String>,
    gauge_end: Option<String>,
    gauge_empty: Option<String>,
    notes: Option<Vec<String>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct DefaultsPatch {
    wheel_lines: Option<isize>,
    toc: Option<bool>,
    bookmarks: Option<bool>,
    commands: Option<bool>,
    column_extra: Option<i16>,
    fade_text: Option<bool>,
}

/// Полная запись конфига для `save()`: все поля, цвета строками.
#[derive(Serialize)]
struct ConfigOut {
    theme: ThemeOut,
    defaults: Defaults,
}

#[derive(Serialize)]
struct ThemeOut {
    key: String,
    accent: String,
    dim: String,
    active: String,
    heading: String,
    notice: String,
    selection_bg: String,
    rule: String,
    gauge_start: String,
    gauge_end: String,
    gauge_empty: String,
    notes: Vec<String>,
}

impl From<&Config> for ConfigOut {
    fn from(config: &Config) -> Self {
        let theme = &config.theme;
        Self {
            theme: ThemeOut {
                key: color_str(&theme.key),
                accent: color_str(&theme.accent),
                dim: color_str(&theme.dim),
                active: color_str(&theme.active),
                heading: color_str(&theme.heading),
                notice: color_str(&theme.notice),
                selection_bg: color_str(&theme.selection_bg),
                rule: color_str(&theme.rule),
                gauge_start: color_str(&theme.gauge_start),
                gauge_end: color_str(&theme.gauge_end),
                gauge_empty: color_str(&theme.gauge_empty),
                notes: theme.notes.iter().map(color_str).collect(),
            },
            defaults: config.defaults.clone(),
        }
    }
}

impl ConfigFile {
    fn apply(self, base: Config, path: &Path) -> Result<Config, ConfigError> {
        let mut config = base;
        let color = |raw: String| -> Result<Color, ConfigError> {
            parse_color(&raw)
                .ok_or_else(|| ConfigError::Color { path: path.to_owned(), value: raw })
        };
        let theme = &mut config.theme;
        let patch = self.theme;
        if let Some(raw) = patch.key {
            theme.key = color(raw)?;
        }
        if let Some(raw) = patch.accent {
            theme.accent = color(raw)?;
        }
        if let Some(raw) = patch.dim {
            theme.dim = color(raw)?;
        }
        if let Some(raw) = patch.active {
            theme.active = color(raw)?;
        }
        if let Some(raw) = patch.heading {
            theme.heading = color(raw)?;
        }
        if let Some(raw) = patch.notice {
            theme.notice = color(raw)?;
        }
        if let Some(raw) = patch.selection_bg {
            theme.selection_bg = color(raw)?;
        }
        if let Some(raw) = patch.rule {
            theme.rule = color(raw)?;
        }
        if let Some(raw) = patch.gauge_start {
            theme.gauge_start = color(raw)?;
        }
        if let Some(raw) = patch.gauge_end {
            theme.gauge_end = color(raw)?;
        }
        if let Some(raw) = patch.gauge_empty {
            theme.gauge_empty = color(raw)?;
        }
        if let Some(raw) = patch.notes {
            if raw.len() != theme.notes.len() {
                return Err(ConfigError::Color {
                    path: path.to_owned(),
                    value: format!(
                        "notes: ожидалось {} цветов, получено {}",
                        theme.notes.len(),
                        raw.len()
                    ),
                });
            }
            for (slot, value) in theme.notes.iter_mut().zip(raw) {
                *slot = color(value)?;
            }
        }
        let defaults = &mut config.defaults;
        if let Some(value) = self.defaults.wheel_lines {
            defaults.wheel_lines = value;
        }
        if let Some(value) = self.defaults.toc {
            defaults.toc = value;
        }
        if let Some(value) = self.defaults.bookmarks {
            defaults.bookmarks = value;
        }
        if let Some(value) = self.defaults.commands {
            defaults.commands = value;
        }
        if let Some(value) = self.defaults.column_extra {
            defaults.column_extra = value;
        }
        if let Some(value) = self.defaults.fade_text {
            defaults.fade_text = value;
        }
        Ok(config)
    }
}

/// Записать текущую конфигурацию в файл (`:theme save`). Все поля
/// сериализуются целиком, чтобы файл читался без базового пресета.
pub fn save(config: &Config) -> Result<PathBuf, ConfigError> {
    let path = path().ok_or(ConfigError::NoPath)?;
    save_to(&path, config)?;
    Ok(path)
}

/// Записать конфигурацию в явный путь, создавая каталог при необходимости.
pub fn save_to(path: &Path, config: &Config) -> Result<(), ConfigError> {
    let file = ConfigOut::from(config);
    let text = toml::to_string_pretty(&file)
        .map_err(|source| ConfigError::Encode { path: path.to_owned(), source })?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|source| ConfigError::Write { path: path.to_owned(), source })?;
    }
    std::fs::write(path, text)
        .map_err(|source| ConfigError::Write { path: path.to_owned(), source })?;
    Ok(())
}

/// Тема пресета; неизвестное имя — ошибка.
pub fn preset_theme(name: &str) -> Result<Theme, ConfigError> {
    match name {
        "btop" => Ok(Theme::btop()),
        "mono" => Ok(Theme::mono()),
        "light" => Ok(Theme::light()),
        other => Err(ConfigError::Preset(other.to_owned())),
    }
}

/// Путь к файлу конфига: `QBOOK_CONFIG` (явно заданный путь к файлу)
/// либо `$XDG_CONFIG_HOME/qbook/config.toml`.
pub fn path() -> Option<PathBuf> {
    if let Some(raw) = std::env::var_os("QBOOK_CONFIG") {
        return Some(PathBuf::from(raw));
    }
    dirs::config_dir().map(|dir| dir.join("qbook").join("config.toml"))
}

/// Загрузить конфигурацию: пресет, затем файл поверх него. Файл в
/// стандартном месте опционален; явный `QBOOK_CONFIG` обязан существовать.
pub fn load(preset: Option<&str>) -> Result<Config, ConfigError> {
    let base = Config { theme: preset_theme(preset.unwrap_or("btop"))?, ..Config::default() };
    let Some(path) = path() else { return Ok(base) };
    let required = std::env::var_os("QBOOK_CONFIG").is_some();
    load_from(&path, base, required)
}

/// Прочитать файл поверх базовой конфигурации: отсутствующий файл, если он
/// не обязателен, оставляет базу без изменений.
pub fn load_from(path: &Path, base: Config, required: bool) -> Result<Config, ConfigError> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound && !required => {
            return Ok(base);
        }
        Err(source) => {
            return Err(ConfigError::Io { path: path.to_owned(), source });
        }
    };
    let file: ConfigFile = toml::from_str(&text)
        .map_err(|source| ConfigError::Toml { path: path.to_owned(), source })?;
    file.apply(base, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &tempfile::TempDir, text: &str) -> PathBuf {
        let path = dir.path().join("config.toml");
        std::fs::write(&path, text).expect("запись конфига");
        path
    }

    #[test]
    fn colors_accept_names_and_hex() {
        assert_eq!(parse_color("yellow"), Some(Color::Yellow));
        assert_eq!(parse_color(" DarkGrey "), Some(Color::DarkGray));
        assert_eq!(parse_color("lightcyan"), Some(Color::LightCyan));
        assert_eq!(parse_color("#ff8000"), Some(Color::Rgb(255, 128, 0)));
        assert_eq!(parse_color("#F80"), Some(Color::Rgb(255, 136, 0)));
        assert_eq!(parse_color("#12345"), None);
        assert_eq!(parse_color("#zzzzzz"), None);
        assert_eq!(parse_color("жёлтый"), None);
        assert_eq!(parse_color("redish"), None);
    }

    #[test]
    fn unknown_preset_is_an_error() {
        let err = preset_theme("neon").expect_err("пресета нет");
        assert!(err.to_string().contains("neon"));
        assert_eq!(preset_theme("mono").expect("mono"), Theme::mono());
    }

    #[test]
    fn partial_file_patches_the_preset() {
        let dir = tempfile::tempdir().expect("каталог");
        let path = write(
            &dir,
            "[theme]\nkey = \"cyan\"\ngauge_start = \"#010203\"\n[defaults]\nwheel_lines = 7\ntoc = false\n",
        );
        let base = Config { theme: Theme::mono(), ..Config::default() };
        let config = load_from(&path, base, true).expect("конфиг");
        assert_eq!(config.theme.key, Color::Cyan, "перекрытый цвет");
        assert_eq!(config.theme.gauge_start, Color::Rgb(1, 2, 3));
        assert_eq!(config.theme.accent, Color::White, "остальное осталось от пресета");
        assert_eq!(config.defaults.wheel_lines, 7);
        assert!(!config.defaults.toc);
        assert!(config.defaults.fade_text, "незаданные поля не тронуты");
    }

    #[test]
    fn bad_color_reports_path_and_value() {
        let dir = tempfile::tempdir().expect("каталог");
        let path = write(&dir, "[theme]\nrule = \"не-цвет\"\n");
        let err = load_from(&path, Config::default(), true).expect_err("цвет невалиден");
        let text = err.to_string();
        assert!(text.contains("не-цвет"), "{text}");
        assert!(text.contains("config.toml"), "{text}");
    }

    #[test]
    fn malformed_toml_is_an_error() {
        let dir = tempfile::tempdir().expect("каталог");
        let path = write(&dir, "[theme\nkey = \n");
        assert!(matches!(load_from(&path, Config::default(), true), Err(ConfigError::Toml { .. })));
    }

    #[test]
    fn missing_optional_file_keeps_base() {
        let dir = tempfile::tempdir().expect("каталог");
        let path = dir.path().join("нет.tomл");
        let base = Config { theme: Theme::light(), ..Config::default() };
        let config = load_from(&path, base.clone(), false).expect("файл опционален");
        assert_eq!(config, base);
        assert!(matches!(load_from(&path, base, true), Err(ConfigError::Io { .. })));
    }

    #[test]
    fn wrong_notes_count_is_rejected() {
        let dir = tempfile::tempdir().expect("каталог");
        let path = write(&dir, "[theme]\nnotes = [\"red\", \"green\"]\n");
        let err = load_from(&path, Config::default(), true).expect_err("должно упасть");
        assert!(err.to_string().contains("7"), "{err}");
    }
}
