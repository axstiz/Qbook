//! EPUB: zip-архив, `container.xml` → OPF → spine → XHTML.
//!
//! Текст берётся строго в порядке spine — иначе главы перемешиваются. Разбор
//! XHTML — потоковый на `quick-xml`: inline-стили едут в `InlineStyle`, ссылки
//! индексируются в `Document.hrefs`, картинки становятся `BlockKind::Image`.
//! Spine-файлы разбираются параллельно; ломаный XML одного файла пропускается.

use std::collections::HashMap;
use std::ops::Range;
use std::path::Path;

use quick_xml::Reader;
use quick_xml::escape::{resolve_predefined_entity, unescape};
use quick_xml::name::QName;
use quick_xml::{
    XmlVersion,
    events::{BytesRef, BytesText, Event},
};
use zip::ZipArchive;

use crate::model::{Block, BlockKind, Document, ImageRef, InlineStyle, Span, TocItem};
use crate::parse::{ParseError, normalize_newlines};

pub fn load(path: &Path) -> Result<Document, ParseError> {
    let shown = path.display().to_string();
    let file = std::fs::File::open(path)
        .map_err(|source| ParseError::Io { path: shown.clone(), source })?;
    let mut zip = ZipArchive::new(file)
        .map_err(|source| ParseError::Zip { path: shown.clone(), problem: source.to_string() })?;
    parse_archive(&mut zip, &shown)
}

/// Язык из метаданных OPF (`dc:language`), если он указан.
pub fn detect_lang(path: &Path) -> Option<String> {
    let mut zip = ZipArchive::new(std::fs::File::open(path).ok()?).ok()?;
    let opf_path = read_container(&mut zip).ok()?;
    let opf = read_entry(&mut zip, &opf_path).ok()?;
    let package = Package::parse(&opf).ok()?;
    (!package.lang.is_empty()).then_some(package.lang)
}

/// Пути контент-документов книги: тело книги, оглавление `nav`, оглавление `ncx`.
#[cfg(feature = "translate")]
pub(crate) struct ContentPaths {
    pub body: Vec<String>,
    pub nav: Option<String>,
    pub ncx: Option<String>,
}

/// Читает OPF и возвращает пути документов, в которых живёт переводимый текст.
#[cfg(feature = "translate")]
pub(crate) fn content_documents<R: std::io::Read + std::io::Seek>(
    zip: &mut ZipArchive<R>,
) -> Result<ContentPaths, ParseError> {
    let opf_path = read_container(zip)?;
    let opf = read_entry(zip, &opf_path)?;
    let package = Package::parse(&opf)
        .map_err(|problem| ParseError::Malformed { path: opf_path.clone(), problem })?;
    let opf_dir = parent_dir(&opf_path);

    let body = package
        .spine
        .iter()
        .filter_map(|idref| {
            package
                .manifest
                .iter()
                .find(|item| item.id == *idref)
                .map(|item| join(&opf_dir, &percent_decode(&item.href)))
        })
        .collect();

    let doc_path = |doc: &TocDoc| join(&opf_dir, &percent_decode(doc.href()));
    let nav = package.toc_doc.as_ref().filter(|d| matches!(d, TocDoc::Nav(_))).map(doc_path);
    let ncx = package.toc_doc.as_ref().filter(|d| matches!(d, TocDoc::Ncx(_))).map(doc_path);

    Ok(ContentPaths { body, nav, ncx })
}

fn parse_archive<R: std::io::Read + std::io::Seek>(
    zip: &mut ZipArchive<R>,
    path: &str,
) -> Result<Document, ParseError> {
    // Внутренние функции уже знают имя проблемного файла, поэтому переносим только путь книги.
    let rewrap = |e: ParseError| -> ParseError {
        match e {
            ParseError::Malformed { problem, .. } | ParseError::Zip { problem, .. } => {
                ParseError::Malformed { path: path.to_owned(), problem }
            }
            other => other,
        }
    };

    let opf_path = read_container(zip).map_err(rewrap)?;
    let opf = read_entry(zip, &opf_path).map_err(rewrap)?;

    let package = Package::parse(&opf)
        .map_err(|problem| ParseError::Malformed { path: path.to_owned(), problem })?;
    let opf_dir = parent_dir(&opf_path);

    // Оглавление `nav` есть в manifest, но его пункты — не текст книги.
    let skip: Vec<&str> =
        package.manifest.iter().filter(|item| item.is_nav).map(|item| item.id.as_str()).collect();

    // Чтение архива последовательное (ZipArchive одному владельцу), разбор — параллельный.
    let mut bodies: Vec<(String, String)> = Vec::new();
    for idref in &package.spine {
        if skip.contains(&idref.as_str()) {
            continue;
        }
        let href = package
            .manifest
            .iter()
            .find(|item| item.id == *idref)
            .map(|item| item.href.as_str())
            .ok_or_else(|| ParseError::Malformed {
                path: path.to_owned(),
                problem: format!("spine ссылается на неизвестный idref {idref}"),
            })?;
        let full = join(&opf_dir, &percent_decode(href));
        if let Ok(xhtml) = read_entry(zip, &full) {
            bodies.push((full, xhtml));
        }
    }

    // Чтение архива последовательное (ZipArchive одному владельцу), разбор — параллельный.
    type RawSection = Result<Vec<RawBlock>, ParseError>;
    let sections: Vec<(String, RawSection)> = std::thread::scope(|scope| {
        let mut handles: Vec<(String, std::thread::ScopedJoinHandle<'_, RawSection>)> = Vec::new();
        for (full, xhtml) in bodies {
            let dir = parent_dir(&full);
            let handle = scope.spawn(move || parse_xhtml(&xhtml, &dir));
            handles.push((full, handle));
        }
        handles
            .into_iter()
            .map(|(full, handle)| (full, handle.join().expect("поток разбора")))
            .collect()
    });

    let mut blocks: Vec<Block> = Vec::new();
    // Диапазоны блоков, занятые каждым spine-пунктом: оглавление ссылается на файл
    // из spine, а не на индекс блока.
    let mut spans: Vec<(String, Range<usize>)> = Vec::new();
    let mut hrefs: Vec<String> = Vec::new();
    let mut href_index: HashMap<String, usize> = HashMap::new();
    for (full, section) in sections {
        let Ok(raw_blocks) = section else { continue };
        let start = blocks.len();
        for raw in raw_blocks {
            blocks.push(materialize(raw, &mut hrefs, &mut href_index));
        }
        if blocks.len() > start {
            spans.push((full, start..blocks.len()));
        }
    }

    // Пиксельные размеры картинок — для высоты ячеек в раскладке; сбой чтения
    // ресурса не валит книгу, картинка останется с alt-текстом.
    for block in &mut blocks {
        let Some(image) = block.image.as_ref() else { continue };
        block.image =
            Some(ImageRef { size: image_dimensions(zip, &image.resource), ..image.clone() });
    }

    if blocks.is_empty() {
        return Err(ParseError::Malformed {
            path: path.to_owned(),
            problem: "в spine не найдено ни одного текстового блока".to_owned(),
        });
    }
    let toc: Vec<TocItem> = read_toc(zip, &package, &opf_dir)
        .iter()
        .filter_map(|item| {
            let block = toc_target(item, &opf_dir, &spans, &blocks)?;
            Some(TocItem { title: item.title.clone(), block, level: item.level })
        })
        .collect();

    // Оглавление есть не у всех книг: без него выводим его из блоков-заголовков,
    // как это делает разбор текста. Тогда обе ветки дают одинаковый результат.
    if toc.is_empty() {
        Ok(Document::with_hrefs(package.lang, package.title, blocks, hrefs))
    } else {
        Ok(Document::with_hrefs_toc(package.lang, package.title, blocks, hrefs, toc))
    }
}

/// Сводит «сырые» блоки секции в модельные: локальные адреса ссылок получают
/// единый индекс в списке `Document.hrefs` (с дедупликацией по адресу).
fn materialize(
    raw: RawBlock,
    hrefs: &mut Vec<String>,
    index: &mut HashMap<String, usize>,
) -> Block {
    let RawBlock { kind, spans: raw_spans, indent, marker, image } = raw;
    for span in &raw_spans {
        let Some(href) = &span.href else { continue };
        if !index.contains_key(href) {
            index.insert(href.clone(), hrefs.len());
            hrefs.push(href.clone());
        }
    }

    if let Some(image) = image {
        let mut block = Block::new(BlockKind::Image, image.alt.clone()).with_image(image);
        block.indent = indent;
        return block;
    }
    let model_spans = raw_spans
        .into_iter()
        .map(|s| {
            let RawSpan { text, style, href } = s;
            let style = href
                .and_then(|href| index.get(&href).copied())
                .map_or(style, |link| InlineStyle { link: Some(link), ..style });
            Span::styled(text, style)
        })
        .collect();
    let mut block = Block::styled(kind, model_spans);
    block.indent = indent;
    block.marker = marker;
    block
}

// --- сырые блоки одного XHTML-файла ----------------------------------------

struct RawSpan {
    text: String,
    style: InlineStyle,
    /// Локальный адрес ссылки; глобальный индекс присваивается при слиянии секций.
    href: Option<String>,
}

struct RawBlock {
    kind: BlockKind,
    spans: Vec<RawSpan>,
    indent: u8,
    marker: Option<String>,
    image: Option<ImageRef>,
}

/// Открытый блок в состоянии набора.
struct Draft {
    kind: BlockKind,
    spans: Vec<RawSpan>,
    indent: u8,
    marker: Option<String>,
    /// Перед каждым непробельным символом решает, нужен ли пробел-разделитель:
    /// текстовые узлы XHTML рвутся произвольными отступами разметки.
    pending_space: bool,
    /// Открыт `<pre>`: пробелы и `\n` значимы — collapse на лету не делается.
    pre: bool,
}

impl Draft {
    fn new(kind: BlockKind, indent: u8, marker: Option<String>) -> Self {
        Self { kind, spans: Vec::new(), indent, marker, pending_space: false, pre: false }
    }

    /// Аккумулирует сырой текст: серия пробелов → один пробел на лету, отступы
    /// pretty-print у первого текста блока подавляются. Стиль применён к тому
    /// диапазону, на который он действовал.
    fn push_chunks(&mut self, text: &str, style: InlineStyle, href: Option<String>) {
        let mut plain = String::new();
        for ch in text.chars() {
            if ch.is_whitespace() {
                if !plain.is_empty() {
                    let piece = std::mem::take(&mut plain);
                    self.push_span(&piece, style, href.clone());
                }
                self.pending_space = true;
                continue;
            }
            if self.pending_space && !self.spans.is_empty() {
                self.push_span(" ", style, href.clone());
            }
            self.pending_space = false;
            plain.push(ch);
        }
        if !plain.is_empty() {
            self.push_span(&plain, style, href);
        }
    }

    fn push_span(&mut self, text: &str, style: InlineStyle, href: Option<String>) {
        if text.is_empty() {
            return;
        }
        if let Some(last) = self.spans.last_mut()
            && last.style == style
            && last.href == href
        {
            last.text.push_str(text);
            return;
        }
        self.spans.push(RawSpan { text: text.to_owned(), style, href });
    }
}

/// Дополнения стиля от одного открытого inline-элемента (`em`, `strong`, …).
struct StyleCtx {
    bold: bool,
    italic: bool,
    underline: bool,
    strike: bool,
    sup: bool,
    sub: bool,
    href: Option<String>,
}

impl StyleCtx {
    fn plain() -> Self {
        Self {
            bold: false,
            italic: false,
            underline: false,
            strike: false,
            sup: false,
            sub: false,
            href: None,
        }
    }
}

/// Текущий эффективный стиль — объединение открытых inline-элементов; href —
/// самый внутренний открытый якорь.
fn current_style(stack: &[StyleCtx]) -> (InlineStyle, Option<String>) {
    let mut style = InlineStyle::PLAIN;
    let mut href = None;
    for ctx in stack {
        style.bold |= ctx.bold;
        style.italic |= ctx.italic;
        style.underline |= ctx.underline;
        style.strike |= ctx.strike;
        style.sup |= ctx.sup;
        style.sub |= ctx.sub;
        if ctx.href.is_some() {
            href = ctx.href.clone();
        }
    }
    (style, href)
}

/// Вложенный блочный элемент в ещё не начавшемся блоке прозрачен:
/// `<blockquote><p>цитата</p></blockquote>` остаётся цитатой. Внутри непустого
/// блока — уже самостоятельный, поэтому предыдущий запечатывается.
fn switch_block(
    blocks: &mut Vec<RawBlock>,
    draft: Option<Draft>,
    kind: BlockKind,
    indent: u8,
    marker: Option<String>,
) -> Option<Draft> {
    if let Some(open) = draft {
        if open.spans.is_empty() {
            // Прозрачность: продолжаем накопление в прежнем блоке, kind не меняется.
            return Some(open);
        }
        blocks.push(finish_draft(open));
    }
    Some(Draft::new(kind, indent, marker))
}

/// Переводит блок из состояния набора в сырой: хвостовые чисто-пробельные спаны
/// сбрасываются — их абзац всё равно подрежет рендер.
fn finish_draft(mut draft: Draft) -> RawBlock {
    draft.spans.retain(|s| !s.text.chars().all(char::is_whitespace));
    RawBlock {
        kind: draft.kind,
        spans: draft.spans,
        indent: draft.indent,
        marker: draft.marker,
        image: None,
    }
}

/// Разбирает один XHTML-документ spine в сырые rich-блоки.
fn parse_xhtml(xhtml: &str, base_dir: &str) -> Result<Vec<RawBlock>, ParseError> {
    let mut reader = Reader::from_str(xhtml);

    let mut blocks: Vec<RawBlock> = Vec::new();
    let mut draft: Option<Draft> = None;
    let mut inline_stack: Vec<StyleCtx> = Vec::new();
    let mut skip = 0usize;
    let mut list_depth = 0usize;
    // `usize::MAX` — неупорядоченный список; прочая цифра — счётчик очередного `ol li`.
    let mut list_counters: Vec<usize> = Vec::new();
    let mut pre_depth = 0usize;

    loop {
        let event = reader.read_event().map_err(|e| ParseError::Malformed {
            path: base_dir.to_owned(),
            problem: format!("ломаный XHTML: {e}"),
        })?;
        let empty = matches!(&event, Event::Empty(_));
        match event {
            Event::Start(element) | Event::Empty(element) => {
                let tag = local_name(&element.name());

                // `image` — картинка и вне svg-контекста, и внутри skip.
                if tag == "image"
                    && let Some(href) = element_attr(&element, "href")
                {
                    if let Some(open) = draft.take() {
                        blocks.push(finish_draft(open));
                    }
                    blocks.push(RawBlock {
                        kind: BlockKind::Image,
                        spans: Vec::new(),
                        indent: 0,
                        marker: None,
                        image: Some(ImageRef {
                            resource: resolve_href(base_dir, &href),
                            alt: element_attr(&element, "alt").unwrap_or_default(),
                            size: None,
                        }),
                    });
                    draft = None;
                    continue;
                }
                if skip > 0 {
                    if !empty {
                        skip += 1;
                    }
                    continue;
                }
                match tag.as_str() {
                    "head" | "script" | "style" | "title" | "template" | "svg" => {
                        skip += usize::from(!empty);
                        continue;
                    }
                    "br" => {
                        if let Some(open) = draft.as_mut() {
                            let (style, href) = current_style(&inline_stack);
                            open.push_span("\n", style, href);
                            if !open.pre {
                                open.pending_space = false;
                            }
                        }
                        continue;
                    }
                    "hr" => {
                        if let Some(open) = draft.take() {
                            blocks.push(finish_draft(open));
                        }
                        blocks.push(RawBlock {
                            kind: BlockKind::Rule,
                            spans: Vec::new(),
                            indent: 0,
                            marker: None,
                            image: None,
                        });
                        continue;
                    }
                    "img" => {
                        // Картинка — отдельный блок: открытый текстовый закрывается.
                        if let Some(open) = draft.take() {
                            blocks.push(finish_draft(open));
                        }
                        let src = element_attr(&element, "src");
                        let alt = element_attr(&element, "alt").unwrap_or_default();
                        if let Some(src) = src {
                            blocks.push(RawBlock {
                                kind: BlockKind::Image,
                                spans: Vec::new(),
                                indent: 0,
                                marker: None,
                                image: Some(ImageRef {
                                    resource: resolve_href(base_dir, &src),
                                    alt,
                                    size: None,
                                }),
                            });
                            draft = None;
                        }
                        continue;
                    }
                    "ol" | "ul" => {
                        list_depth += 1;
                        list_counters.push(if tag == "ol" { 0 } else { usize::MAX });
                        continue;
                    }
                    "li" => {
                        let indent = list_depth.saturating_sub(1).min(u8::MAX as usize) as u8;
                        let marker = list_counters.last_mut().map(|slot| {
                            if *slot == usize::MAX {
                                "•".to_owned()
                            } else {
                                *slot += 1;
                                format!("{}.", slot)
                            }
                        });
                        draft = switch_block(
                            &mut blocks,
                            draft.take(),
                            BlockKind::ListItem,
                            indent,
                            marker,
                        );
                        continue;
                    }
                    "pre" => {
                        draft = switch_block(&mut blocks, draft.take(), BlockKind::Code, 0, None);
                        if let Some(open) = draft.as_mut() {
                            open.pre = true;
                        }
                        pre_depth += 1;
                        continue;
                    }
                    "td" => {
                        draft = switch_block(&mut blocks, draft.take(), BlockKind::Verse, 0, None);
                        continue;
                    }
                    "dt" | "dd" | "figcaption" => {
                        draft =
                            switch_block(&mut blocks, draft.take(), BlockKind::Paragraph, 0, None);
                        continue;
                    }
                    "p" => {
                        draft =
                            switch_block(&mut blocks, draft.take(), BlockKind::Paragraph, 0, None);
                        continue;
                    }
                    "blockquote" => {
                        draft = switch_block(&mut blocks, draft.take(), BlockKind::Quote, 0, None);
                        continue;
                    }
                    "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                        let level = tag.as_bytes()[1] - b'0';
                        draft = switch_block(
                            &mut blocks,
                            draft.take(),
                            BlockKind::Heading(level),
                            0,
                            None,
                        );
                        continue;
                    }
                    "div" | "figure" | "table" | "tr" | "tbody" | "thead" | "body" | "html" => {
                        continue;
                    }
                    _ => {}
                }

                // Inline-элементы: дополнить стиль либо пронести ссылку.
                let ctx = match tag.as_str() {
                    "strong" | "b" => StyleCtx { bold: true, ..StyleCtx::plain() },
                    "em" | "i" | "dfn" | "cite" => StyleCtx { italic: true, ..StyleCtx::plain() },
                    "u" | "ins" => StyleCtx { underline: true, ..StyleCtx::plain() },
                    "s" | "del" | "strike" => StyleCtx { strike: true, ..StyleCtx::plain() },
                    "sup" => StyleCtx { sup: true, ..StyleCtx::plain() },
                    "sub" => StyleCtx { sub: true, ..StyleCtx::plain() },
                    "a" => StyleCtx {
                        href: element_attr(&element, "href")
                            .map(|href| resolve_href(base_dir, &href)),
                        ..StyleCtx::plain()
                    },
                    _ => StyleCtx::plain(),
                };
                if !empty {
                    inline_stack.push(ctx);
                }
            }
            Event::End(element) => {
                let tag = local_name(&element.name());
                if skip > 0 {
                    skip -= 1;
                    continue;
                }
                match tag.as_str() {
                    "ol" | "ul" => {
                        list_depth = list_depth.saturating_sub(1);
                        list_counters.pop();
                    }
                    "pre" => {
                        pre_depth = pre_depth.saturating_sub(1);
                        if pre_depth == 0
                            && let Some(open) = draft.as_mut()
                        {
                            open.pre = false;
                        }
                    }
                    "strong" | "b" | "em" | "i" | "dfn" | "cite" | "u" | "ins" | "s" | "del"
                    | "strike" | "sup" | "sub" | "a" => {
                        inline_stack.pop();
                    }
                    _ => {}
                }
            }
            Event::Text(content) => {
                if skip > 0 {
                    continue;
                }
                let text = text_of(content).unwrap_or_default();
                let (style, href) = current_style(&inline_stack);
                if let Some(open) = draft.as_mut() {
                    if open.pre {
                        open.push_span(&text, style, href);
                    } else {
                        open.push_chunks(&text, style, href);
                    }
                }
            }
            Event::GeneralRef(reference) => {
                if skip > 0 {
                    continue;
                }
                let text = resolve_entity(&reference).to_owned();
                let (style, href) = current_style(&inline_stack);
                if let Some(open) = draft.as_mut() {
                    if open.pre {
                        open.push_span(&text, style, href);
                    } else {
                        open.push_chunks(&text, style, href);
                    }
                }
            }
            Event::CData(content) => {
                if skip > 0 {
                    continue;
                }
                let (style, href) = current_style(&inline_stack);
                if let Some(open) = draft.as_mut() {
                    open.push_span(&content.xml10_content(), style, href);
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }

    if let Some(open) = draft.take() {
        blocks.push(finish_draft(open));
    }
    Ok(blocks)
}

/// Текст события: сущности разрешаются через `unescape`.
fn text_of(content: BytesText<'_>) -> Result<String, String> {
    unescape(content.as_ref()).map(|cow| cow.into_owned()).map_err(|e| e.to_string())
}

/// Тело события `&entity;`: именованные сущности — по таблице XML, числовые —
/// сами; незнакомое оставляем как есть (это литерал, не сущность).
fn resolve_entity(reference: &BytesRef<'_>) -> String {
    let raw: &str = reference.as_ref();
    if let Some(predefined) = resolve_predefined_entity(raw) {
        return predefined.to_owned();
    }
    let numeric = raw
        .strip_prefix("#x")
        .map(|rest| (rest, 16u32))
        .or_else(|| raw.strip_prefix("#X").map(|rest| (rest, 16u32)))
        .or_else(|| raw.strip_prefix('#').map(|rest| (rest, 10u32)));
    if let Some((text, radix)) = numeric
        && let Ok(code) = u32::from_str_radix(text, radix)
        && let Some(ch) = char::from_u32(code)
    {
        return ch.to_string();
    }
    raw.to_owned()
}

/// Значение атрибута с разрешением сущностей и нормализацией (XML 1.0).
fn attr_value(attr: &quick_xml::events::attributes::Attribute<'_>) -> Option<String> {
    attr.normalized_value(XmlVersion::Implicit1_0).map(|value| value.into_owned()).ok()
}

fn element_attr(element: &quick_xml::events::BytesStart<'_>, key: &str) -> Option<String> {
    for attr in element.attributes() {
        let Ok(attr) = attr else { continue };
        if attr.key.local_name().as_ref() == key
            && let Some(value) = attr_value(&attr)
        {
            return Some(value);
        }
    }
    None
}

fn local_name(name: &QName<'_>) -> String {
    name.local_name().as_ref().to_owned()
}

/// Относительный href секции → абсолютный путь внутри zip. Абсолютные схемы и
/// голые якоря остаются как есть (в тексте книги их всё равно резолвить некуда).
fn resolve_href(base_dir: &str, href: &str) -> String {
    if href.is_empty() || href.starts_with('#') || href.starts_with("mailto:") {
        return href.to_owned();
    }
    if let Some(at) = href.find("://")
        && at >= 2
    {
        return href.to_owned();
    }
    collapse_path(&format!("{base_dir}{}", percent_decode(href)))
}

/// Убирает `.`/`..`-сегменты и дубли слешей, сохраняя ведущий `/` если был.
fn collapse_path(path: &str) -> String {
    let mut segments: Vec<&str> = Vec::new();
    for segment in path.split('/') {
        match segment {
            "" | "." => continue,
            ".." => {
                segments.pop();
            }
            _ => segments.push(segment),
        }
    }
    let mut out = String::with_capacity(path.len());
    if path.starts_with('/') {
        out.push('/');
    }
    out.push_str(&segments.join("/"));
    out
}

// --- контейнер -------------------------------------------------------------

const CONTAINER_PATH: &str = "META-INF/container.xml";

fn read_container<R: std::io::Read + std::io::Seek>(
    zip: &mut ZipArchive<R>,
) -> Result<String, ParseError> {
    let xml = read_entry(zip, CONTAINER_PATH)?;
    let mut reader = Reader::from_str(&xml);
    loop {
        let event = reader.read_event().map_err(|e| ParseError::Malformed {
            path: CONTAINER_PATH.to_owned(),
            problem: e.to_string(),
        })?;
        match event {
            Event::Start(element) | Event::Empty(element) => {
                if element.name().local_name().as_ref() == "rootfile"
                    && let Some(path) = element_attr(&element, "full-path")
                {
                    return Ok(path);
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Err(ParseError::Malformed {
        path: CONTAINER_PATH.to_owned(),
        problem: "нет элемента rootfile с атрибутом full-path".into(),
    })
}

// --- OPF -------------------------------------------------------------------

struct ManifestItem {
    id: String,
    href: String,
    /// `properties="nav"`: документ с оглавлением. Он есть в spine, но его пункты
    /// оглавления — не текст книги.
    is_nav: bool,
}

struct TocEntry {
    title: String,
    href: String,
    level: u8,
}

/// Источник оглавления: `nav.xhtml` в EPUB 3, `toc.ncx` в EPUB 2.
enum TocDoc {
    Nav(String),
    Ncx(String),
}

struct Package {
    title: String,
    lang: String,
    manifest: Vec<ManifestItem>,
    spine: Vec<String>,
    toc_doc: Option<TocDoc>,
}

impl Package {
    fn parse(opf: &str) -> Result<Self, String> {
        #[derive(Copy, Clone, PartialEq, Eq)]
        enum Meta {
            Title,
            Lang,
        }
        let mut title = String::new();
        let mut lang = String::new();
        let mut manifest: Vec<ManifestItem> = Vec::new();
        let mut spine: Vec<String> = Vec::new();
        let mut spine_toc: Option<String> = None;
        let mut open_meta: Option<Meta> = None;
        let mut buffer = String::new();

        let mut reader = Reader::from_str(opf);
        loop {
            let event = reader.read_event().map_err(|e| e.to_string())?;
            let empty = matches!(&event, Event::Empty(_));
            match event {
                Event::Start(element) | Event::Empty(element) => {
                    match element.name().local_name().as_ref() {
                        "title" => {
                            if !empty && title.is_empty() {
                                open_meta = Some(Meta::Title);
                                buffer.clear();
                            }
                        }
                        "language" => {
                            if !empty && lang.is_empty() {
                                open_meta = Some(Meta::Lang);
                                buffer.clear();
                            }
                        }
                        "item" => {
                            let mut id = String::new();
                            let mut href = String::new();
                            let mut is_nav = false;
                            for attr in element.attributes() {
                                let Ok(attr) = attr else { continue };
                                let Some(value) = attr_value(&attr) else { continue };
                                match attr.key.local_name().as_ref() {
                                    "id" => id = value,
                                    "href" => href = percent_decode(&value),
                                    "properties" => {
                                        is_nav = value.split_whitespace().any(|p| p == "nav");
                                    }
                                    _ => {}
                                }
                            }
                            if !id.is_empty() && !href.is_empty() {
                                manifest.push(ManifestItem { id, href, is_nav });
                            }
                        }
                        "itemref" => {
                            if let Some(idref) = element_attr(&element, "idref") {
                                spine.push(idref);
                            }
                        }
                        "spine" => spine_toc = element_attr(&element, "toc"),
                        _ => {}
                    }
                }
                Event::Text(content) => {
                    if let Some(_) = open_meta
                        && let Ok(text) = text_of(content)
                    {
                        buffer.push_str(&text);
                    }
                }
                Event::End(element) => {
                    let tag = element.name().local_name().as_ref().to_owned();
                    if (tag == "title" || tag == "language")
                        && let Some(meta) = open_meta.take()
                    {
                        let text = buffer.trim().to_owned();
                        match meta {
                            Meta::Title => {
                                if title.is_empty() {
                                    title = text;
                                }
                            }
                            Meta::Lang => {
                                if lang.is_empty() {
                                    lang = text;
                                }
                            }
                        }
                        buffer.clear();
                    }
                }
                Event::Eof => break,
                _ => {}
            }
        }

        // Пустой spine не ошибкой: `detect_lang` требует только метаданные;
        // обязательность spine проверяет `parse_archive` после слияния секций.

        // EPUB 2 прячет оглавление в атрибуте `spine toc="ncx"`, EPUB 3 — в `properties="nav"`.
        let toc_doc = manifest
            .iter()
            .find(|item| item.is_nav)
            .map(|item| TocDoc::Nav(item.href.clone()))
            .or_else(|| {
                let id = spine_toc.as_deref()?;
                manifest
                    .iter()
                    .find(|item| item.id == id)
                    .map(|item| TocDoc::Ncx(item.href.clone()))
            });

        Ok(Self { title, lang, manifest, spine, toc_doc })
    }
}

// --- оглавление ------------------------------------------------------------

fn read_toc<R: std::io::Read + std::io::Seek>(
    zip: &mut ZipArchive<R>,
    package: &Package,
    opf_dir: &str,
) -> Vec<TocEntry> {
    let Some(doc) = &package.toc_doc else { return Vec::new() };
    let href = join(opf_dir, &percent_decode(doc.href()));
    // Оглавление — необязательная часть книги: если файла нет, разбор не падает.
    let Ok(text) = read_entry(zip, &href) else { return Vec::new() };
    match doc {
        TocDoc::Nav(_) => nav_toc(&text),
        TocDoc::Ncx(_) => ncx_toc(&text),
    }
}

impl TocDoc {
    fn href(&self) -> &str {
        match self {
            TocDoc::Nav(href) | TocDoc::Ncx(href) => href,
        }
    }
}

/// `nav[type=toc]` из `nav.xhtml`. Уровень вложенности — число открытых `ol`
/// на момент ссылки (внешний список — уровень 1).
fn nav_toc(nav_xhtml: &str) -> Vec<TocEntry> {
    let mut reader = Reader::from_str(nav_xhtml);
    let mut entries: Vec<TocEntry> = Vec::new();
    let mut ol_depth = 0usize;
    let mut in_toc = false;
    // Текст копится только внутри `<a>`: заголовки и прочее в nav — не названия глав.
    let mut in_anchor = false;
    let mut current_href: Option<String> = None;
    let mut text = String::new();

    while let Ok(event) = reader.read_event() {
        match event {
            Event::Start(element) => match element.name().local_name().as_ref() {
                "nav" => {
                    in_toc = element.attributes().any(|a| {
                        a.is_ok_and(|a| {
                            a.key.local_name().as_ref() == "type"
                                && attr_value(&a).as_deref() == Some("toc")
                        })
                    }) || element_attr(&element, "id").is_some_and(|id| id == "toc");
                }
                "ol" if in_toc => ol_depth += 1,
                "a" if in_toc => {
                    in_anchor = true;
                    current_href = element_attr(&element, "href");
                }
                _ => {}
            },
            Event::Empty(element) => {
                if in_toc && element.name().local_name().as_ref() == "a" {
                    in_anchor = true;
                    current_href = element_attr(&element, "href");
                }
            }
            Event::Text(content) => {
                if in_toc
                    && in_anchor
                    && let Ok(value) = text_of(content)
                {
                    let fragment = value.trim();
                    if !fragment.is_empty() {
                        text.push_str(fragment);
                    }
                }
            }
            Event::End(element) => match element.name().local_name().as_ref() {
                "ol" if in_toc => ol_depth = ol_depth.saturating_sub(1),
                "a" if in_toc => {
                    in_anchor = false;
                    if let Some(href) = current_href.take()
                        && !text.trim().is_empty()
                    {
                        entries.push(TocEntry {
                            title: text.clone(),
                            href,
                            level: ol_depth.max(1) as u8,
                        });
                    }
                    text.clear();
                }
                "nav" => in_toc = false,
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
    }
    entries
}

/// `navMap` из `toc.ncx` — оглавление EPUB 2. Уровень задаёт глубина вложенности
/// текущего `navPoint` (внешний пункт — уровень 1).
fn ncx_toc(ncx_xml: &str) -> Vec<TocEntry> {
    let mut reader = Reader::from_str(ncx_xml);
    let mut entries: Vec<TocEntry> = Vec::new();
    // Стек открытых `navPoint`: внешний пункт ждёт `text` и `content` по очереди.
    let mut points: Vec<Option<(String, Option<String>)>> = Vec::new();

    while let Ok(event) = reader.read_event() {
        match event {
            Event::Start(element) => {
                if element.name().local_name().as_ref() == "navPoint" {
                    points.push(None);
                }
            }
            Event::Empty(element) => {
                if element.name().local_name().as_ref() == "content"
                    && let Some(src) = element_attr(&element, "src")
                    && let Some(top) = points.last_mut()
                    && top.is_none()
                {
                    *top = Some((String::new(), Some(src)));
                }
            }
            Event::Text(content) => {
                if let Ok(value) = text_of(content) {
                    let fragment = value.trim();
                    if fragment.is_empty() {
                        continue;
                    }
                    if let Some(state) = points.last_mut()
                        && let Some((title, _)) = state.as_mut()
                    {
                        if !title.is_empty() {
                            title.push(' ');
                        }
                        title.push_str(fragment);
                    }
                }
            }
            Event::End(element) => {
                if element.name().local_name().as_ref() == "navPoint"
                    && let Some(Some((title, href))) = points.pop()
                    && let Some(href) = href
                    && !title.trim().is_empty()
                {
                    entries.push(TocEntry { title, href, level: (points.len() + 1) as u8 });
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    entries
}

/// Ссылка оглавления ведёт внутрь spine-пункта, поэтому сначала находим диапазон
/// блоков этого файла, а внутри — блок с подходящим заголовком.
fn toc_target(
    entry: &TocEntry,
    opf_dir: &str,
    spans: &[(String, Range<usize>)],
    blocks: &[Block],
) -> Option<usize> {
    let (file, _) = entry.href.split_once('#').unwrap_or((entry.href.as_str(), ""));
    let file = join(opf_dir, &percent_decode(file));
    let span = spans.iter().find(|(name, _)| *name == file).map(|(_, s)| s.clone())?;

    let needle = normalize_title(&entry.title);
    // Сначала заголовок с точно совпадающим текстом; затем заголовок, содержащий
    // иглу (в книгах заголовки начинаются с нумерации «1.1 Первая глава»).
    span.clone()
        .find(|&i| blocks[i].kind.is_heading() && normalize_title(&blocks[i].text) == needle)
        .or_else(|| {
            span.clone().find(|&i| {
                blocks[i].kind.is_heading() && normalize_title(&blocks[i].text).contains(&needle)
            })
        })
        .or_else(|| span.clone().find(|&i| normalize_title(&blocks[i].text) == needle))
        .or_else(|| span.clone().find(|&i| normalize_title(&blocks[i].text).contains(&needle)))
        .or(Some(span.start))
}

/// Заголовки в оглавлении и в тексте различаются пунктуацией, регистром и тире.
fn normalize_title(title: &str) -> String {
    title
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == ' ')
        .flat_map(char::to_lowercase)
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Пиксельный размер изображения из первых 64 КиБ ресурса.
fn image_dimensions<R: std::io::Read + std::io::Seek>(
    zip: &mut ZipArchive<R>,
    path: &str,
) -> Option<(u32, u32)> {
    use std::io::Read as _;
    let mut entry = zip.by_name(path).ok()?;
    let mut head = Vec::new();
    (&mut entry).take(64 * 1024).read_to_end(&mut head).ok()?;
    image::ImageReader::new(std::io::Cursor::new(&head))
        .with_guessed_format()
        .ok()?
        .into_dimensions()
        .ok()
}

// --- утилиты ---------------------------------------------------------------

fn read_entry<R: std::io::Read + std::io::Seek>(
    zip: &mut ZipArchive<R>,
    name: &str,
) -> Result<String, ParseError> {
    let missing = || ParseError::Malformed {
        path: name.to_owned(),
        problem: "нет такого файла в архиве".into(),
    };

    let mut entry = zip.by_name(name).map_err(|_| missing())?;
    let mut buf = String::new();
    std::io::Read::read_to_string(&mut entry, &mut buf)
        .map_err(|source| ParseError::Io { path: name.to_owned(), source })?;
    Ok(normalize_newlines(&buf))
}

fn parent_dir(path: &str) -> String {
    path.rsplit_once('/').map_or_else(String::new, |(dir, _)| format!("{dir}/"))
}

fn join(dir: &str, href: &str) -> String {
    if dir.is_empty() { href.to_owned() } else { format!("{dir}{href}") }
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
            if let Some(byte) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod scratch {
    #[test]
    fn debug_nav_levels() {
        let nav = r#"<html><nav epub:type="toc"><ol><li><a href="a">1</a><ol><li><a href="b">2</a><ol><li><a href="c">3</a></li></ol></li></ol></li></ol></nav></html>"#;
        let entries = super::nav_toc(nav);
        let levels: Vec<u8> = entries.iter().map(|t| t.level).collect();
        assert_eq!(
            levels,
            [1, 2, 3],
            "уровни: {:?}",
            entries.iter().map(|t| (t.level, &t.title)).collect::<Vec<_>>()
        );
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;
    use crate::model::BlockKind;
    use zip::write::SimpleFileOptions;

    /// Собирает EPUB прямо в памяти: внешние фикстуры не нужны и не устаревают.
    fn archive(parts: Vec<(String, String)>) -> Vec<u8> {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (name, body) in parts {
            zip.start_file(name, options).expect("начать запись");
            zip.write_all(body.as_bytes()).expect("записать");
        }
        zip.finish().expect("закрыть архив").into_inner()
    }

    fn parse_str(parts: Vec<(String, String)>) -> Result<Document, ParseError> {
        let buf = archive(parts);
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(buf)).expect("открыть архив");
        parse_archive(&mut zip, "book.epub")
    }

    fn container_for(opf_path: &str) -> String {
        format!(
            r#"<?xml version="1.0"?>
<container xmlns="urn:oasis:names:tc:opendocument:xmlns:container" version="1.0">
<rootfiles><rootfile full-path="{opf_path}"/></rootfiles></container>"#
        )
    }

    fn opf(spine: &[&str], manifest: &str, lang: &str) -> String {
        let items: String = spine.iter().map(|id| format!(r#"<itemref idref="{id}"/>"#)).collect();
        format!(
            r#"<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="uid">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>The Book</dc:title>
    <dc:language>{lang}</dc:language>
  </metadata>
  <manifest>{manifest}</manifest>
  <spine>{items}</spine>
</package>"#
        )
    }

    fn page(body: &str) -> String {
        format!(
            r#"<?xml version="1.0" encoding="utf-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><head><title>x</title></head><body>{body}</body></html>"#
        )
    }

    fn texts(doc: &Document) -> Vec<&str> {
        doc.blocks().iter().map(|b| b.text.as_str()).collect()
    }

    #[test]
    fn reads_blocks_in_spine_order() {
        let files = vec![
            ("META-INF/container.xml".to_owned(), container_for("OEBPS/content.opf")),
            (
                "OEBPS/content.opf".to_owned(),
                opf(
                    &["c1", "c2"],
                    r#"<item id="c1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
<item id="c2" href="ch2.xhtml" media-type="application/xhtml+xml"/>"#,
                    "en",
                ),
            ),
            ("OEBPS/ch1.xhtml".to_owned(), page("<h1>One</h1><p>alpha</p>")),
            ("OEBPS/ch2.xhtml".to_owned(), page("<h1>Two</h1><p>beta</p>")),
        ];
        let doc = parse_str(files).expect("разобрать");
        assert_eq!(texts(&doc), ["One", "alpha", "Two", "beta"]);
        assert_eq!(doc.title(), "The Book");
        assert_eq!(doc.lang(), "en");
    }

    #[test]
    fn manifest_order_does_not_override_spine() {
        let files = vec![
            ("META-INF/container.xml".to_owned(), container_for("content.opf")),
            (
                "content.opf".to_owned(),
                opf(
                    &["second", "first"],
                    r#"<item id="first" href="a.xhtml"/>
<item id="second" href="b.xhtml"/>"#,
                    "en",
                ),
            ),
            ("a.xhtml".to_owned(), page("<p>AAA</p>")),
            ("b.xhtml".to_owned(), page("<p>BBB</p>")),
        ];
        let doc = parse_str(files).expect("разобрать");
        assert_eq!(doc.blocks()[0].text, "BBB");
    }

    #[test]
    fn skips_script_style_head_title() {
        let blocks = parse_xhtml(
            &page(
                r#"<p>keep</p><script>var x = "drop";</script><style>.a{color:red}</style><p>also</p>"#,
            ),
            "",
        )
        .expect("разбор");
        let texts: Vec<String> = blocks.iter().map(|b| b.spans_text()).collect();
        assert_eq!(texts, ["keep", "also"]);
    }

    #[test]
    fn broken_xml_in_one_chapter_is_skipped_others_survive() {
        let files = vec![
            ("META-INF/container.xml".to_owned(), container_for("content.opf")),
            (
                "content.opf".to_owned(),
                opf(
                    &["a", "b"],
                    r#"<item id="a" href="a.xhtml"/>
<item id="b" href="b.xhtml"/>"#,
                    "en",
                ),
            ),
            ("a.xhtml".to_owned(), "<p>broken</c><p>unmatched end tag".to_owned()),
            ("b.xhtml".to_owned(), page("<p>intact</p>")),
        ];
        let doc = parse_str(files).expect("ломаная секция не рушит книгу");
        assert_eq!(texts(&doc), ["intact"]);
    }

    #[test]
    fn br_inside_paragraph_becomes_newline() {
        let blocks = parse_xhtml(&page("<p>line one<br/>line two</p>"), "").expect("разбор");
        assert_eq!(blocks[0].spans_text(), "line one\nline two");
    }

    #[test]
    fn inline_tags_do_not_split_paragraph() {
        let blocks = parse_xhtml(&page(r##"<p>a <em>b</em> c <a href="#fn1">d</a> e</p>"##), "")
            .expect("разбор");
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].spans_text(), "a b c d e");
        assert_eq!(blocks[0].spans.iter().filter(|s| s.href.is_some()).count(), 1);
    }

    #[test]
    fn em_and_strong_become_styled_spans() {
        let blocks = parse_xhtml(
            &page("<p>Обычный <em>курсив</em> и <strong>жирный</strong>, <b>b</b> и <i>i</i>.</p>"),
            "",
        )
        .expect("разбор");
        assert_eq!(blocks.len(), 1);
        let gold: Vec<(&str, bool, bool)> = vec![
            ("Обычный", false, false),
            (" курсив", true, false),
            (" и", false, false),
            (" жирный", false, true),
            (",", false, false),
            (" b", false, true),
            (" и", false, false),
            (" i", true, false),
            (".", false, false),
        ];
        for (span, (text, italic, bold)) in blocks[0].spans.iter().zip(&gold) {
            assert_eq!(&span.text.as_str(), text);
            assert_eq!(span.style.italic, *italic, "курсив в «{text}»");
            assert_eq!(span.style.bold, *bold, "жирный в «{text}»");
        }
    }

    #[test]
    fn nested_italic_inside_bold_keeps_both_flags() {
        let blocks =
            parse_xhtml(&page("<p><strong>жирный <em>и курсив</em></strong> конец</p>"), "")
                .expect("разбор");
        let spans = &blocks[0].spans;
        assert_eq!(spans.len(), 3);
        assert!(spans[0].style.bold && !spans[0].style.italic);
        assert!(spans[1].style.bold && spans[1].style.italic);
        assert!(!spans[2].style.bold && !spans[2].style.italic);
    }

    #[test]
    fn nested_block_continues_paragraph() {
        let blocks = parse_xhtml(&page("<div><p>outer <span>still outer</span></p></div>"), "")
            .expect("разбор");
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].spans_text(), "outer still outer");
    }

    #[test]
    fn block_tags_map_to_kinds() {
        let blocks = parse_xhtml(
            &page("<h2>H</h2><p>p</p><blockquote>q</blockquote><li>l</li><pre>c</pre><hr/>"),
            "",
        )
        .expect("разбор");
        let kinds: Vec<BlockKind> = blocks.iter().map(|b| b.kind).collect();
        assert_eq!(
            kinds,
            [
                BlockKind::Heading(2),
                BlockKind::Paragraph,
                BlockKind::Quote,
                BlockKind::ListItem,
                BlockKind::Code,
                BlockKind::Rule,
            ]
        );
    }

    #[test]
    fn entities_are_decoded() {
        let blocks =
            parse_xhtml(&page("<p>a &amp; b &lt;tag&gt; &#8212; c</p>"), "").expect("разбор");
        assert_eq!(blocks[0].spans_text(), "a & b <tag> — c", "амперсанд, тег и тире");
    }

    #[test]
    fn whitespace_runs_collapse_to_single_space() {
        let blocks =
            parse_xhtml(&page("<p>Обычный  <em>причём</em>   разделены    одним.</p>"), "")
                .expect("разбор");
        assert_eq!(blocks[0].spans_text(), "Обычный причём разделены одним.");
    }

    #[test]
    fn unicode_survives_intact() {
        let blocks = parse_xhtml(&page("<p>Привет, мир! 日本語もね</p>"), "").expect("разбор");
        assert_eq!(blocks[0].spans_text(), "Привет, мир! 日本語もね");
    }

    #[test]
    fn list_items_get_bullets_and_nested_indent() {
        let blocks = parse_xhtml(
            &page("<ol><li>первый<ul><li>внутри</li></ul></li><li>второй</li></ol>"),
            "",
        )
        .expect("разбор");
        assert_eq!(blocks.len(), 3);
        assert_eq!(blocks[0].spans_text(), "первый");
        assert_eq!(blocks[0].marker.as_deref(), Some("1."));
        assert_eq!(blocks[1].spans_text(), "внутри");
        assert_eq!(blocks[1].indent, 1, "вложенный список с отступом");
        assert_eq!(blocks[1].marker.as_deref(), Some("•"));
        assert_eq!(blocks[2].spans_text(), "второй");
        assert_eq!(blocks[2].marker.as_deref(), Some("2."));
    }

    #[test]
    fn images_become_image_blocks() {
        let blocks = parse_xhtml(
            &page(r#"<p>до</p><img src="img/pic.png" alt="подпись"/><p>после</p>"#),
            "base/",
        )
        .expect("разбор");
        assert_eq!(blocks.len(), 3);
        let image = blocks[1].image.as_ref().expect("ресурс картинки");
        assert_eq!(image.resource, "base/img/pic.png", "href — абсолютный путь в книге");
        assert_eq!(image.alt, "подпись");
        assert_eq!(blocks[0].spans_text(), "до");
        assert_eq!(blocks[2].spans_text(), "после");
    }

    #[test]
    fn pre_keep_raw_lines() {
        let blocks = parse_xhtml(&page("<pre>let x = 1;\nlet y = 2;</pre>"), "").expect("разбор");
        assert_eq!(blocks[0].kind, BlockKind::Code);
        assert_eq!(blocks[0].spans_text(), "let x = 1;\nlet y = 2;");
    }

    #[test]
    fn heading_blocks_feed_toc() {
        let files = vec![
            ("META-INF/container.xml".to_owned(), container_for("content.opf")),
            ("content.opf".to_owned(), opf(&["c1"], r#"<item id="c1" href="c.xhtml"/>"#, "en")),
            ("c.xhtml".to_owned(), page("<h1>Intro</h1><p>x</p><h1>Chapter Two!</h1><p>y</p>")),
        ];
        let doc = parse_str(files).expect("разобрать");
        let titles: Vec<&str> = doc.toc().iter().map(|t| t.title.as_str()).collect();
        assert_eq!(titles, ["Intro", "Chapter Two!"]);
        assert_eq!(doc.toc()[1].block, 2);
    }

    #[test]
    fn hrefs_across_chapters_are_global_and_deduplicated() {
        let files = vec![
            ("META-INF/container.xml".to_owned(), container_for("content.opf")),
            (
                "content.opf".to_owned(),
                opf(
                    &["a", "b"],
                    r#"<item id="a" href="a.xhtml"/>
<item id="b" href="b.xhtml"/>"#,
                    "en",
                ),
            ),
            ("a.xhtml".to_owned(), page(r#"<p><a href="sub/t.xhtml#x">первая</a></p>"#)),
            (
                "b.xhtml".to_owned(),
                page(
                    r#"<p><a href="../sub/t.xhtml#x">дубль</a> и <a href="else.html">новая</a></p>"#,
                ),
            ),
        ];
        let doc = parse_str(files).expect("разобрать");
        assert_eq!(doc.hrefs().len(), 2, "одинаковый адрес по дедуп");
        assert_eq!(doc.hrefs(), ["sub/t.xhtml#x", "else.html"]);
        // Обе ссылки ведут в одну запись hrefs.
        let links: Vec<usize> = doc
            .blocks()
            .iter()
            .flat_map(|b| b.spans.iter())
            .filter_map(|s| s.style().link)
            .collect();
        assert_eq!(links, [0, 0, 1]);
    }

    #[test]
    fn missing_spine_entry_is_reported() {
        let files = vec![
            ("META-INF/container.xml".to_owned(), container_for("content.opf")),
            ("content.opf".to_owned(), opf(&["ghost"], "", "en")),
        ];
        assert!(parse_str(files).is_err());
    }

    #[test]
    fn epub_without_container_is_rejected() {
        assert!(
            parse_str(vec![("mimetype".to_owned(), "application/epub+zip".to_owned())]).is_err()
        );
    }

    #[test]
    fn load_reads_archive_from_disk() {
        let dir = std::env::temp_dir().join("qbook-epub-on-disk");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("каталог");
        let path = dir.join("book.epub");
        std::fs::write(
            &path,
            archive(vec![
                ("META-INF/container.xml".to_owned(), container_for("OEBPS/content.opf")),
                (
                    "OEBPS/content.opf".to_owned(),
                    opf(&["c1"], r#"<item id="c1" href="c.xhtml"/>"#, "en"),
                ),
                ("OEBPS/c.xhtml".to_owned(), page("<h1>T</h1><p>body</p>")),
            ]),
        )
        .expect("записать");
        let doc = load(&path).expect("загрузить");
        assert_eq!(doc.len(), 2);
        assert_eq!(doc.validate(), Ok(()));
    }

    #[test]
    fn percent_encoded_hrefs_resolve() {
        assert_eq!(percent_decode("a%20b.xhtml"), "a b.xhtml");
        assert_eq!(percent_decode("plain.xhtml"), "plain.xhtml");
        assert_eq!(percent_decode("100%"), "100%");
    }

    #[test]
    fn parent_dir_and_join() {
        assert_eq!(parent_dir("OEBPS/content.opf"), "OEBPS/");
        assert_eq!(parent_dir("content.opf"), "");
        assert_eq!(join("OEBPS/", "c.xhtml"), "OEBPS/c.xhtml");
        assert_eq!(join("", "c.xhtml"), "c.xhtml");
    }

    #[test]
    fn resolve_href_collapses_relative_segments() {
        assert_eq!(resolve_href("OEBPS/", "ch1.xhtml"), "OEBPS/ch1.xhtml");
        assert_eq!(resolve_href("OEBPS/", "sub/../ch1.xhtml"), "OEBPS/ch1.xhtml");
        assert_eq!(resolve_href("", "./ch1.xhtml"), "ch1.xhtml");
        assert_eq!(resolve_href("OEBPS/", "http://example.org/page"), "http://example.org/page");
        assert_eq!(resolve_href("OEBPS/", "#anchor"), "#anchor");
    }

    #[test]
    fn titles_normalize_for_matching() {
        assert_eq!(normalize_title("Chapter Two!"), "chapter two");
        assert_eq!(normalize_title("Chapter  Two"), "chapter two");
    }

    const NAV: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops"><body>
  <nav epub:type="toc" id="toc"><h1>Оглавление</h1><ol>
    <li><a href="c.xhtml#one">One</a>
      <ol><li><a href="c.xhtml#one-a">One A</a></li></ol>
    </li>
    <li><a href="c.xhtml#two">Two</a></li>
  </ol></nav>
</body></html>"#;

    const NCX: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/"><navMap>
  <navPoint id="n1"><navLabel><text>One</text></navLabel><content src="c.xhtml#one"/>
    <navPoint id="n2"><navLabel><text>One A</text></navLabel><content src="c.xhtml#one-a"/></navPoint>
  </navPoint>
  <navPoint id="n3"><navLabel><text>Two</text></navLabel><content src="c.xhtml#two"/></navPoint>
</navMap></ncx>"#;

    const CHAPTER: &str = r#"<h1>One</h1><p>a</p><h2>One A</h2><p>b</p><h1>Two</h1><p>c</p>"#;

    #[test]
    fn nav_document_supplies_toc_with_levels() {
        let files = vec![
            ("META-INF/container.xml".to_owned(), container_for("OEBPS/content.opf")),
            (
                "OEBPS/content.opf".to_owned(),
                opf(
                    &["nav", "c1"],
                    r#"<item id="nav" href="nav.xhtml" properties="nav"/>
<item id="c1" href="c.xhtml"/>"#,
                    "en",
                ),
            ),
            ("OEBPS/nav.xhtml".to_owned(), NAV.to_owned()),
            ("OEBPS/c.xhtml".to_owned(), page(CHAPTER)),
        ];
        let doc = parse_str(files).expect("разобрать");
        let toc: Vec<(&str, u8, usize)> =
            doc.toc().iter().map(|t| (t.title.as_str(), t.level, t.block)).collect();
        assert_eq!(toc, [("One", 1, 0), ("One A", 2, 2), ("Two", 1, 4)]);
    }

    #[test]
    fn ncx_document_supplies_toc_when_there_is_no_nav() {
        let files = vec![
            ("META-INF/container.xml".to_owned(), container_for("OEBPS/content.opf")),
            (
                "OEBPS/content.opf".to_owned(),
                r#"<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf" version="2.0" unique-identifier="uid">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>The Book</dc:title>
    <dc:language>en</dc:language>
  </metadata>
  <manifest><item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
<item id="c1" href="c.xhtml"/></manifest>
  <spine toc="ncx"><itemref idref="c1"/></spine>
</package>"#
                    .to_owned(),
            ),
            ("OEBPS/toc.ncx".to_owned(), NCX.to_owned()),
            ("OEBPS/c.xhtml".to_owned(), page(CHAPTER)),
        ];
        let doc = parse_str(files).expect("разобрать");
        let toc: Vec<(&str, u8, usize)> =
            doc.toc().iter().map(|t| (t.title.as_str(), t.level, t.block)).collect();
        assert_eq!(toc, [("One", 1, 0), ("One A", 2, 2), ("Two", 1, 4)]);
    }

    #[test]
    fn nav_document_is_not_book_text() {
        let files = vec![
            ("META-INF/container.xml".to_owned(), container_for("OEBPS/content.opf")),
            (
                "OEBPS/content.opf".to_owned(),
                opf(
                    &["nav", "c1"],
                    r#"<item id="nav" href="nav.xhtml" properties="nav"/>
<item id="c1" href="c.xhtml"/>"#,
                    "en",
                ),
            ),
            ("OEBPS/nav.xhtml".to_owned(), NAV.to_owned()),
            ("OEBPS/c.xhtml".to_owned(), page(CHAPTER)),
        ];
        let doc = parse_str(files).expect("разобрать");
        assert_eq!(texts(&doc), ["One", "a", "One A", "b", "Two", "c"]);
    }

    #[test]
    fn missing_toc_document_falls_back_to_headings() {
        let files = vec![
            ("META-INF/container.xml".to_owned(), container_for("OEBPS/content.opf")),
            (
                "OEBPS/content.opf".to_owned(),
                opf(
                    &["nav", "c1"],
                    r#"<item id="nav" href="nav.xhtml" properties="nav"/>
<item id="c1" href="c.xhtml"/>"#,
                    "en",
                ),
            ),
            ("OEBPS/c.xhtml".to_owned(), page(CHAPTER)),
        ];
        let doc = parse_str(files).expect("разобрать");
        let titles: Vec<&str> = doc.toc().iter().map(|t| t.title.as_str()).collect();
        assert_eq!(titles, ["One", "One A", "Two"]);
    }

    #[test]
    fn detect_lang_reads_opf_metadata() {
        let files = vec![
            ("META-INF/container.xml".to_owned(), container_for("content.opf")),
            (
                "content.opf".to_owned(),
                r#"<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="uid">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>q</dc:title>
    <dc:language>ru</dc:language>
  </metadata>
  <manifest/>
  <spine/>
</package>"#
                    .to_owned(),
            ),
        ];
        let dir = std::env::temp_dir().join("qbook-epub-lang");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("каталог");
        let path = dir.join("book.epub");
        std::fs::write(&path, archive(files)).expect("записать");
        assert_eq!(detect_lang(&path).as_deref(), Some("ru"));
    }
}

#[cfg(test)]
impl RawBlock {
    /// Плоский текст сырого блока, как его увидит рендер: конкатенация спанов.
    fn spans_text(&self) -> String {
        self.spans.iter().map(|s| s.text.as_str()).collect()
    }
}
