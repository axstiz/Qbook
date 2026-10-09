//! Шаблонизатор XHTML: переводим текстовые узлы, разметку сохраняем байт-в-байт.
//!
//! Блок-элемент (`p`, `h1`, `li`, …) считается единицей перевода. Внутри единицы
//! текст превращается в шаблон, а каждый тег — в плейсхолдер `[[n]]`; движок
//! переводит шаблон целиком, после чего разметка возвращается на места. Если
//! движок сломал плейсхолдеры, вызывающий использует фолбэк по текстовым узлам.

use std::ops::Range;

use quick_xml::Reader;
use quick_xml::escape::unescape;
use quick_xml::events::{BytesRef, Event};

/// Тип документа эпилога: от него зависит набор единиц перевода.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocKind {
    /// Документ из spine: единицы — блочные элементы тела.
    Body,
    /// `nav.xhtml`: единицы — ссылки оглавления.
    Nav,
    /// `toc.ncx`: единицы — `text` внутри `navLabel`.
    Ncx,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Part {
    Text(String),
    Raw(String),
}

/// Единица перевода: диапазон внутреннего содержимого элемента и его части.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unit {
    /// Байтовый диапазон внутреннего содержимого в исходном документе.
    pub range: Range<usize>,
    parts: Vec<Part>,
}

impl Unit {
    /// Шаблон для движка: текст с плейсхолдерами вместо разметки.
    pub fn template(&self) -> String {
        let mut out = String::new();
        let mut i = 0usize;
        for part in &self.parts {
            match part {
                Part::Text(text) => out.push_str(text),
                Part::Raw(_) => {
                    out.push_str("[[");
                    out.push_str(&i.to_string());
                    out.push_str("]]");
                    i += 1;
                }
            }
        }
        out
    }

    /// Непустые текстовые части — для фолбэка по отдельным узлам.
    pub fn text_parts(&self) -> Vec<&str> {
        self.parts
            .iter()
            .filter_map(|part| match part {
                Part::Text(text) if !text.is_empty() => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }

    /// Нет текста для перевода (только разметка/пробелы) — единицу можно пропустить.
    pub fn is_blank(&self) -> bool {
        self.text_parts().is_empty()
    }

    /// Собирает содержимое из переведённого шаблона. `None`, если плейсхолдеры
    /// потеряны или переставлены — тогда нужен фолбэк.
    pub fn rebuild(&self, translated: &str) -> Option<String> {
        let (segments, tokens) = split_tokens(translated);
        let raw_count = self.parts.iter().filter(|p| matches!(p, Part::Raw(_))).count();
        let text_count = self.parts.iter().filter(|p| matches!(p, Part::Text(_))).count();
        if tokens.len() != raw_count || segments.len() != text_count {
            return None;
        }
        if tokens.iter().enumerate().any(|(i, &t)| t != i) {
            return None;
        }
        let mut out = String::new();
        let mut seg = 0usize;
        for part in &self.parts {
            match part {
                Part::Text(_) => {
                    out.push_str(&escape_text(segments[seg]));
                    seg += 1;
                }
                Part::Raw(raw) => out.push_str(raw),
            }
        }
        Some(out)
    }

    /// Сборка из переводов по отдельным текстовым узлам (фолбэк).
    pub fn rebuild_parts(&self, translations: &[String]) -> String {
        let mut out = String::new();
        let mut i = 0usize;
        for part in &self.parts {
            match part {
                Part::Text(text) if text.is_empty() => {}
                Part::Text(_) => {
                    if let Some(translated) = translations.get(i) {
                        out.push_str(&escape_text(translated));
                    }
                    i += 1;
                }
                Part::Raw(raw) => out.push_str(raw),
            }
        }
        out
    }
}

/// Находит единицы перевода в документе: внешние блочные элементы с текстом.
pub fn find_units(xhtml: &str, kind: DocKind) -> Vec<Unit> {
    let mut reader = Reader::from_str(xhtml);
    reader.config_mut().trim_text(false);
    let mut units = Vec::new();
    let mut open: Option<String> = None;
    let mut depth = 0usize;
    let mut inner_start = 0usize;
    loop {
        let start = reader.buffer_position() as usize;
        let Ok(event) = reader.read_event() else { break };
        let end = reader.buffer_position() as usize;
        match event {
            Event::Eof => break,
            Event::Start(e) => {
                let qname = e.name();
                let name = local_name(qname.as_ref());
                if open.is_none() {
                    if is_unit(kind, name) {
                        open = Some(name.to_owned());
                        depth = 1;
                        inner_start = end;
                    }
                } else if open.as_deref() == Some(name) {
                    depth += 1;
                }
            }
            Event::End(e) => {
                let qname = e.name();
                let name = local_name(qname.as_ref());
                if open.as_deref() == Some(name) {
                    depth -= 1;
                    if depth == 0 {
                        let inner = &xhtml[inner_start..start];
                        units.push(build_unit(inner, inner_start..start));
                        open = None;
                    }
                }
            }
            _ => {}
        }
    }
    units
}

/// Разбирает внутреннее содержимое единицы на чередующиеся текстовые и raw-части.
fn build_unit(inner: &str, range: Range<usize>) -> Unit {
    let mut reader = Reader::from_str(inner);
    reader.config_mut().trim_text(false);
    let mut parts = vec![Part::Text(String::new())];
    let mut skip_depth = 0usize;
    let mut skip_buf = String::new();
    loop {
        let start = reader.buffer_position() as usize;
        let Ok(event) = reader.read_event() else { break };
        let end = reader.buffer_position() as usize;
        let raw = inner.get(start..end).unwrap_or("");
        match event {
            Event::Eof => break,
            Event::Text(text) => {
                if skip_depth > 0 {
                    skip_buf.push_str(raw);
                    continue;
                }
                let content = text.into_inner();
                match unescape(&content) {
                    Ok(decoded) if decoded.trim().is_empty() => push_raw(&mut parts, raw),
                    Ok(decoded) => append_text(&mut parts, &decoded),
                    Err(_) => push_raw(&mut parts, raw),
                }
            }
            Event::Start(e) => {
                let qname = e.name();
                let name = local_name(qname.as_ref());
                if skip_depth > 0 || is_skip(name) {
                    skip_depth += 1;
                    skip_buf.push_str(raw);
                } else {
                    push_raw(&mut parts, raw);
                }
            }
            Event::GeneralRef(reference) => {
                if skip_depth > 0 {
                    skip_buf.push_str(raw);
                } else if let Some(text) = resolve_ref(&reference) {
                    append_text(&mut parts, &text);
                } else {
                    push_raw(&mut parts, raw);
                }
            }
            Event::End(_) => {
                if skip_depth > 0 {
                    skip_buf.push_str(raw);
                    skip_depth -= 1;
                    if skip_depth == 0 {
                        let buf = std::mem::take(&mut skip_buf);
                        push_raw(&mut parts, &buf);
                    }
                } else {
                    push_raw(&mut parts, raw);
                }
            }
            _ => {
                if skip_depth > 0 {
                    skip_buf.push_str(raw);
                } else {
                    push_raw(&mut parts, raw);
                }
            }
        }
    }
    if skip_depth > 0 {
        let buf = std::mem::take(&mut skip_buf);
        push_raw(&mut parts, &buf);
    }
    Unit { range, parts }
}

/// Дописывает текст в текущую текстовую часть (она всегда последняя).
fn append_text(parts: &mut [Part], text: &str) {
    if let Some(Part::Text(last)) = parts.last_mut() {
        last.push_str(text);
    }
}

/// Закрывает текущую текстовую часть разметкой и открывает следующую.
fn push_raw(parts: &mut Vec<Part>, raw: &str) {
    parts.push(Part::Raw(raw.to_owned()));
    parts.push(Part::Text(String::new()));
}

/// Разбивает переведённый шаблон на текстовые сегменты и номера плейсхолдеров.
fn split_tokens(text: &str) -> (Vec<&str>, Vec<usize>) {
    let bytes = text.as_bytes();
    let mut segments = Vec::new();
    let mut tokens = Vec::new();
    let mut last = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'[' && bytes.get(i + 1) == Some(&b'[') {
            let mut j = i + 2;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                j += 1;
            }
            if j > i + 2 && bytes.get(j) == Some(&b']') && bytes.get(j + 1) == Some(&b']') {
                segments.push(&text[last..i]);
                tokens.push(text[i + 2..j].parse().unwrap_or(usize::MAX));
                i = j + 2;
                last = i;
                continue;
            }
        }
        i += 1;
    }
    segments.push(&text[last..]);
    (segments, tokens)
}

/// Раскрывает ссылку на символ/сущность в текст; неизвестные сущности — не трогаем.
fn resolve_ref(reference: &BytesRef<'_>) -> Option<String> {
    if reference.is_char_ref()
        && let Ok(Some(c)) = reference.resolve_char_ref()
    {
        return Some(c.to_string());
    }
    match reference.as_ref() {
        "amp" => Some("&".to_owned()),
        "lt" => Some("<".to_owned()),
        "gt" => Some(">".to_owned()),
        "apos" => Some("'".to_owned()),
        "quot" => Some("\"".to_owned()),
        _ => None,
    }
}

/// Экранирование текста для вставки обратно в XML.
fn escape_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(c),
        }
    }
    out
}

/// Локальное имя элемента без префикса пространства имён.
fn local_name(name: &str) -> &str {
    name.rsplit_once(':').map_or(name, |(_, local)| local)
}

/// Блочные элементы, дающие единицу перевода в теле книги.
fn is_unit(kind: DocKind, name: &str) -> bool {
    match kind {
        DocKind::Nav => name == "a",
        DocKind::Ncx => name == "text",
        DocKind::Body => matches!(
            name,
            "p" | "h1"
                | "h2"
                | "h3"
                | "h4"
                | "h5"
                | "h6"
                | "li"
                | "blockquote"
                | "td"
                | "dt"
                | "dd"
                | "figcaption"
        ),
    }
}

/// Элементы, содержимое которых не переводим (код, скрипты, разметка).
fn is_skip(name: &str) -> bool {
    matches!(
        name,
        "script"
            | "style"
            | "head"
            | "title"
            | "svg"
            | "math"
            | "pre"
            | "code"
            | "rt"
            | "rp"
            | "noscript"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn units(xhtml: &str) -> Vec<Unit> {
        find_units(xhtml, DocKind::Body)
    }

    fn one(xhtml: &str) -> Unit {
        let mut found = units(xhtml);
        assert_eq!(found.len(), 1, "ожидалась одна единица в {xhtml}");
        found.pop().expect("единица")
    }

    #[test]
    fn plain_paragraph_translated() {
        let unit = one("<p>Hello world</p>");
        assert_eq!(unit.template(), "Hello world");
        assert_eq!(unit.rebuild("Привет мир").as_deref(), Some("Привет мир"));
        assert!(unit.rebuild("Привет мир").is_some());
    }

    #[test]
    fn inline_emphasis_becomes_placeholders() {
        let unit = one("<p>a <em>b</em> c</p>");
        assert_eq!(unit.template(), "a [[0]]b[[1]] c");
        assert_eq!(unit.rebuild("a [[0]]b[[1]] c").as_deref(), Some("a <em>b</em> c"));
        assert_eq!(unit.text_parts(), ["a ", "b", " c"]);
    }

    #[test]
    fn attributes_survive_verbatim() {
        let unit = one(r#"<p>see <a href="x.xhtml#n" class="lnk">note</a>!</p>"#);
        assert_eq!(unit.template(), "see [[0]]note[[1]]!");
        let rebuilt = unit.rebuild("see [[0]]заметка[[1]]!").expect("собрано");
        assert_eq!(rebuilt, r#"see <a href="x.xhtml#n" class="lnk">заметка</a>!"#);
    }

    #[test]
    fn entities_decode_and_reencode() {
        let unit = one("<p>a &amp; b &lt;tag&gt;</p>");
        assert_eq!(unit.template(), "a & b <tag>");
        assert_eq!(unit.rebuild("a & b <tag>").as_deref(), Some("a &amp; b &lt;tag&gt;"));
    }

    #[test]
    fn void_elements_are_tokens() {
        let unit = one("<p>line<br/>two</p>");
        assert_eq!(unit.template(), "line[[0]]two");
        assert_eq!(unit.rebuild("line[[0]]two").as_deref(), Some("line<br/>two"));
    }

    #[test]
    fn script_and_code_content_is_not_translated() {
        let unit = one("<p>keep <code>x = 1</code> more</p>");
        assert_eq!(unit.template(), "keep [[0]] more");
        assert_eq!(unit.text_parts(), ["keep ", " more"]);
    }

    #[test]
    fn cdata_is_preserved() {
        let unit = one("<p>a<![CDATA[ <b>raw</b> ]]>b</p>");
        assert_eq!(unit.template(), "a[[0]]b");
        assert_eq!(unit.rebuild("a[[0]]b").as_deref(), Some("a<![CDATA[ <b>raw</b> ]]>b"));
    }

    #[test]
    fn cyrillic_round_trip() {
        let unit = one("<p>Привет, мир!</p>");
        assert_eq!(unit.template(), "Привет, мир!");
        assert_eq!(unit.rebuild("Здравствуй, мир!").as_deref(), Some("Здравствуй, мир!"));
    }

    #[test]
    fn broken_tokens_detected() {
        let unit = one("<p>a <em>b</em> c</p>");
        assert_eq!(unit.rebuild("a b c"), None);
        assert_eq!(unit.rebuild("a [[1]]b[[0]] c"), None);
        assert_eq!(unit.rebuild("a [[0]]b c"), None);
    }

    #[test]
    fn fallback_translates_each_node() {
        let unit = one("<p>a <em>b</em> c</p>");
        let translations = vec!["A ".to_owned(), "B".to_owned(), " C".to_owned()];
        assert_eq!(unit.rebuild_parts(&translations), "A <em>B</em> C");
    }

    #[test]
    fn finds_each_top_level_block() {
        let found = units("<div><p>one</p><p>two</p><h2>three</h2></div>");
        assert_eq!(found.len(), 3);
        assert_eq!(found[0].template(), "one");
        assert_eq!(found[1].template(), "two");
        assert_eq!(found[2].template(), "three");
    }

    #[test]
    fn nested_block_stays_one_unit() {
        let unit = one("<blockquote><p>quote</p></blockquote>");
        assert_eq!(unit.template(), "[[0]]quote[[1]]");
        assert_eq!(unit.rebuild("[[0]]цитата[[1]]").as_deref(), Some("<p>цитата</p>"));
    }

    #[test]
    fn whitespace_only_unit_is_blank() {
        assert!(one("<p>   </p>").is_blank());
        assert!(!one("<p>x</p>").is_blank());
    }

    #[test]
    fn nav_units_are_links() {
        let found =
            find_units("<nav><ol><li><a href=\"c.xhtml\">One</a></li></ol></nav>", DocKind::Nav);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].template(), "One");
    }

    #[test]
    fn ncx_units_are_text() {
        let found = find_units("<navLabel><text>One</text></navLabel>", DocKind::Ncx);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].template(), "One");
    }

    #[test]
    fn range_covers_inner_content() {
        let unit = one("<p>Hello</p>");
        let source = "<p>Hello</p>";
        assert_eq!(&source[unit.range.clone()], "Hello");
    }
}
