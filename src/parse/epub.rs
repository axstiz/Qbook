//! EPUB: zip-архив, `container.xml` -> OPF -> spine -> XHTML.
//!
//! Текст берётся строго в порядке spine — иначе главы перемешиваются.
//! Из разметки остаются только блочные элементы; иллюстрации и сноски отбрасываются,
//! но сноска-обёртка `<a>` не должна рвать текст на части.

use std::ops::Range;
use std::path::Path;

use ego_tree::NodeRef;
use scraper::{Html, Node, Selector};
use zip::ZipArchive;

use crate::model::{Block, BlockKind, Document, TocItem};
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
    let (_, lang) = metadata(&Html::parse_document(&opf));
    (!lang.is_empty()).then_some(lang)
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

    let mut blocks: Vec<Block> = Vec::new();
    // Диапазоны блоков, занятые каждым spine-пунктом: оглавление ссылается на файл
    // из spine, а не на индекс блока.
    let mut spans: Vec<(String, Range<usize>)> = Vec::new();

    for idref in &package.spine {
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
        let Ok(xhtml) = read_entry(zip, &full) else { continue };

        let start = blocks.len();
        extract_blocks(&xhtml, &mut blocks);
        if blocks.len() > start {
            spans.push((full, start..blocks.len()));
        }
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
        Ok(Document::new(package.lang, package.title, blocks))
    } else {
        Ok(Document::with_toc(package.lang, package.title, blocks, toc))
    }
}

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

// --- container.xml ---------------------------------------------------------

const CONTAINER_PATH: &str = "META-INF/container.xml";

fn read_container<R: std::io::Read + std::io::Seek>(
    zip: &mut ZipArchive<R>,
) -> Result<String, ParseError> {
    let xml = read_entry(zip, CONTAINER_PATH)?;
    let html = Html::parse_document(&xml);
    let selector = Selector::parse("rootfile").map_err(|e| ParseError::Malformed {
        path: CONTAINER_PATH.to_owned(),
        problem: e.to_string(),
    })?;
    html.select(&selector)
        .next()
        .and_then(|el| el.value().attr("full-path"))
        .map(str::to_owned)
        .ok_or_else(|| ParseError::Malformed {
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
        let html = Html::parse_document(opf);
        let sel = |q: &str| Selector::parse(q).map_err(|e| e.to_string());

        let (title, lang) = metadata(&html);

        let manifest_sel = sel("item").map_err(|e| e.to_string())?;
        let manifest: Vec<ManifestItem> = html
            .select(&manifest_sel)
            .map(|el| {
                let id = el.value().attr("id").unwrap_or_default().to_owned();
                let href = el.value().attr("href").unwrap_or_default().to_owned();
                let is_nav = el
                    .value()
                    .attr("properties")
                    .is_some_and(|props| props.split_whitespace().any(|p| p == "nav"));
                ManifestItem { id, href, is_nav }
            })
            .filter(|item| !item.id.is_empty() && !item.href.is_empty())
            .collect();

        let spine_sel = sel("itemref").map_err(|e| e.to_string())?;
        // Оглавление не читается как текст: его пункты описывают книгу, но не являются ею.
        let skip: Vec<&str> =
            manifest.iter().filter(|item| item.is_nav).map(|item| item.id.as_str()).collect();
        let spine: Vec<String> = html
            .select(&spine_sel)
            .filter_map(|el| el.value().attr("idref"))
            .filter(|id| !skip.contains(id))
            .map(str::to_owned)
            .collect();

        if spine.is_empty() {
            return Err("в OPF нет элементов itemref".into());
        }

        // EPUB 2 прячет оглавление в атрибуте `spine toc="ncx"`, EPUB 3 — в `properties="nav"`.
        let ncx_id = sel("spine")
            .ok()
            .and_then(|s| html.select(&s).next())
            .and_then(|spine| spine.value().attr("toc"))
            .map(str::to_owned);
        let toc_doc = manifest
            .iter()
            .find(|item| item.is_nav)
            .map(|item| TocDoc::Nav(item.href.clone()))
            .or_else(|| {
                let id = ncx_id.as_deref()?;
                manifest
                    .iter()
                    .find(|item| item.id == id)
                    .map(|item| TocDoc::Ncx(item.href.clone()))
            });

        Ok(Self { title, lang, manifest, spine, toc_doc })
    }
}

/// Имена элементов OPF содержат двоеточие (`dc:title`), а html5ever теряет префиксы
/// пространств имён, поэтому метаданные читаем обходом дерева, а не CSS-селектором.
/// Безымянные файлы без `dc:title` остаются с пустым заголовком — это норма.
fn collect_text(node: NodeRef<'_, Node>) -> String {
    let mut out = String::new();
    for child in node.children() {
        if let Some(text) = child.value().as_text() {
            out.push_str(text);
        }
    }
    out
}

fn metadata(opf: &Html) -> (String, String) {
    let mut title = String::new();
    let mut lang = String::new();
    for node in opf.tree.nodes() {
        let Some(el) = node.value().as_element() else { continue };
        let text = collect_text(node).trim().to_owned();
        match el.name() {
            // Префикс пространства имён приходит частью имени элемента.
            "dc:title" | "title" => title = text,
            "dc:language" | "language" => lang = text,
            _ => {}
        }
    }
    (title, lang)
}

/// `nav[type=toc]` из `nav.xhtml`. Уровень вложенности даёт число `ol`-списков,
/// внутри которых лежит ссылка. Панель оглавления в v1 не показывается,
/// но собираем данные сразу: они нужны для перехода и для проверки структуры.
fn nav_toc(nav_xhtml: &str) -> Vec<TocEntry> {
    let html = Html::parse_document(nav_xhtml);
    let Ok(nav) = Selector::parse("nav") else { return Vec::new() };
    let Ok(anchor) = Selector::parse("a") else { return Vec::new() };

    html.select(&nav)
        .filter(|el| el.value().attr("type").is_some_and(|t| t == "toc"))
        .flat_map(|nav| nav.select(&anchor))
        .filter_map(|a| {
            let title = a.text().collect::<String>();
            let href = a.value().attr("href")?.to_owned();
            let level = a
                .ancestors()
                .filter(|node| node.value().as_element().is_some_and(|el| el.name() == "ol"))
                .count();
            // У каждого пункта есть свой `ol`, поэтому число вложенных списков и есть уровень.
            let level = u8::try_from(level).unwrap_or(u8::MAX).max(1);
            (!title.trim().is_empty()).then_some(TocEntry { title, href, level })
        })
        .collect()
}

/// `navMap` из `toc.ncx` — оглавление EPUB 2. Уровень задаёт вложенность `navPoint`.
fn ncx_toc(ncx_xml: &str) -> Vec<TocEntry> {
    let html = Html::parse_document(ncx_xml);
    let Ok(point) = Selector::parse("navpoint") else { return Vec::new() };
    let Ok(label) = Selector::parse("text") else { return Vec::new() };
    let Ok(src) = Selector::parse("content") else { return Vec::new() };

    html.select(&point)
        .filter_map(|point| {
            let title = point.select(&label).next()?.text().collect::<String>();
            let href = point.select(&src).next()?.value().attr("src")?.to_owned();
            let level = point
                .ancestors()
                .filter(|node| node.value().as_element().is_some_and(|el| el.name() == "navpoint"))
                .count();
            // `ancestors()` не содержит сам элемент, поэтому верхний `navPoint` — это 1.
            let level = u8::try_from(level).unwrap_or(u8::MAX).saturating_add(1);
            (!title.trim().is_empty()).then_some(TocEntry { title, href, level })
        })
        .collect()
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
    // Сначала заголовок с совпадающим текстом: у него точная привязка к смыслу главы.
    span.clone()
        .find(|&i| blocks[i].kind.is_heading() && normalize_title(&blocks[i].text) == needle)
        .or_else(|| span.clone().find(|&i| normalize_title(&blocks[i].text) == needle))
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

// --- XHTML -> блоки --------------------------------------------------------

/// Блочные элементы, которые дают самостоятельный блок документа.
fn block_kind(tag: &str) -> Option<BlockKind> {
    match tag {
        "p" => Some(BlockKind::Paragraph),
        "blockquote" => Some(BlockKind::Quote),
        "li" => Some(BlockKind::ListItem),
        "pre" => Some(BlockKind::Code),
        "td" => Some(BlockKind::Verse),
        "h1" => Some(BlockKind::Heading(1)),
        "h2" => Some(BlockKind::Heading(2)),
        "h3" => Some(BlockKind::Heading(3)),
        "h4" => Some(BlockKind::Heading(4)),
        "h5" => Some(BlockKind::Heading(5)),
        "h6" => Some(BlockKind::Heading(6)),
        "hr" => Some(BlockKind::Rule),
        _ => None,
    }
}

/// Элементы, содержимое которых не является текстом книги.
fn is_skipped(tag: &str) -> bool {
    matches!(tag, "script" | "style" | "head" | "svg" | "img" | "image")
}

fn extract_blocks(xhtml: &str, blocks: &mut Vec<Block>) {
    let html = Html::parse_document(xhtml);
    // `head` пропускается сам по себе (он в списке is_skipped), но обход от `body`
    // заодно отсекает Doctype и комментарии.
    let root = html
        .tree
        .nodes()
        .find(|node| node.value().as_element().is_some_and(|el| el.name() == "body"))
        .unwrap_or_else(|| html.tree.root());

    let mut sink = Sink::new();
    walk(root, &mut sink);
    blocks.extend(sink.finish());
}

/// Накопитель блоков. Держит ровно один открытый блок: вложенные блочные элементы
/// либо прозрачны для него, либо закрывают его и открывают следующий.
struct Sink {
    blocks: Vec<Block>,
    /// Индекс блока, в который идёт текст. `None` — текст вне блочного элемента.
    open: Option<usize>,
}

impl Sink {
    fn new() -> Self {
        Self { blocks: Vec::new(), open: None }
    }

    /// XHTML pretty-printed, поэтому пробелы в текстовых узлах — это отступы разметки,
    /// а не содержание. Каждую серию пробелов схлопываем в один пробел, ничего не обрезая:
    /// нужный пробел между `a` и `<em>b</em>` отличает «a b» от «ab».
    fn collapse(text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        let mut space = false;
        for c in text.chars() {
            if c.is_whitespace() {
                space = true;
                continue;
            }
            if space {
                out.push(' ');
                space = false;
            }
            out.push(c);
        }
        if space {
            out.push(' ');
        }
        out
    }

    fn push_text(&mut self, text: &str) {
        let Some(index) = self.open else { return };
        // Отступы pretty-print у первого текста блока — шум, между словами — содержание.
        let text = if self.blocks[index].text.is_empty() { text.trim_start() } else { text };
        if text.is_empty() {
            return;
        }
        append(&mut self.blocks[index], text);
    }

    fn break_line(&mut self) {
        if let Some(index) = self.open {
            append(&mut self.blocks[index], "\n");
        }
    }

    /// Открывает блок. Блочный элемент, вложенный в ещё не начатый блок, прозрачен:
    /// `<blockquote><p>цитата</p></blockquote>` остаётся цитатой, а не абзацем.
    /// Вложенный блок внутри непустого — уже самостоятельный блок.
    fn open_block(&mut self, kind: BlockKind) {
        if self.open.is_some_and(|i| self.blocks[i].text.is_empty()) {
            return;
        }
        self.blocks.push(Block::new(kind, ""));
        self.open = Some(self.blocks.len() - 1);
    }

    /// Разделитель: не накапливает текст, но и не закрывает текущий блок.
    fn push_filler(&mut self, kind: BlockKind) {
        self.blocks.push(Block::new(kind, ""));
    }

    fn finish(self) -> Vec<Block> {
        // Пустой последний блок — артефакт вложенности, а не текст.
        self.blocks
            .into_iter()
            .filter(|b| !b.text.is_empty() || b.kind.is_filler())
            .map(|mut b| {
                // Текст дописывался по текстовым узлам, между которыми остались одиночные
                // пробелы: `узел "…убрать. "` + узел `" "` + `узел "Рис."`. В `Code` и `Verse`
                // отступ значим, поэтому там пробелы не трогаем. Новую строку даёт `<br>`,
                // её сохраняем — иначе теряется разбивка стихов и адресов.
                if !matches!(b.kind, BlockKind::Code | BlockKind::Verse) {
                    let lines: Vec<String> = b
                        .text
                        .split('\n')
                        .map(|line| {
                            Sink::collapse(line).split_whitespace().collect::<Vec<_>>().join(" ")
                        })
                        .filter(|line| !line.is_empty())
                        .collect();
                    b.text = lines.join("\n");
                }
                // Текст дописывался в обход конструктора, поэтому приводим блок к виду,
                // который гарантирует `Document::validate`.
                b = Block::new(b.kind, std::mem::take(&mut b.text));
                b
            })
            .collect()
    }
}

/// Дописывает текст в блок и держит кэш символьной длины в согласованном состоянии.
fn append(block: &mut Block, text: &str) {
    let chars = block.char_len() + text.chars().count();
    block.text.push_str(text);
    block.set_char_len(chars);
}

fn walk(node: NodeRef<'_, Node>, sink: &mut Sink) {
    for child in node.children() {
        if let Some(el) = child.value().as_element() {
            let name = el.name();

            if is_skipped(name) {
                continue;
            }
            if name == "br" {
                sink.break_line();
                continue;
            }
            // Инлайн-усиление `em`/`i` и `strong`/`b` сохраняется маркерами
            // markdown — их снимает и красит рендер, как в .txt книгах.
            if matches!(name, "em" | "i" | "strong" | "b") {
                let mark = if matches!(name, "strong" | "b") { "**" } else { "*" };
                sink.push_text(mark);
                walk(child, sink);
                sink.push_text(mark);
                continue;
            }
            match block_kind(name) {
                Some(kind) if kind.is_filler() => sink.push_filler(kind),
                Some(kind) => sink.open_block(kind),
                None => {}
            }
            walk(child, sink);
            continue;
        }

        if let Some(text) = child.value().as_text() {
            let collapsed = Sink::collapse(text.as_ref());
            if !collapsed.is_empty() {
                sink.push_text(&collapsed);
            }
        }
    }
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
mod tests {
    use std::io::Write;

    use super::*;
    use crate::model::BlockKind;

    /// Собирает EPUB прямо в тесте: внешние фикстуры не нужны и не устаревают.
    struct EpubBuilder {
        files: Vec<(String, String)>,
    }

    impl EpubBuilder {
        fn new() -> Self {
            Self { files: Vec::new() }
        }

        fn add(mut self, name: &str, body: &str) -> Self {
            self.files.push((name.to_owned(), body.to_owned()));
            self
        }

        fn container(opf: &str) -> Self {
            Self::new().add(
                "META-INF/container.xml",
                &format!(
                    r#"<?xml version="1.0"?>
<container xmlns="urn:oasis:names:tc:opendocument:xmlns:container" version="1.0">
  <rootfiles><rootfile full-path="{opf}" media-type="application/oebps-package+xml"/></rootfiles>
</container>"#
                ),
            )
        }

        fn write(self, dir: &Path) -> std::path::PathBuf {
            let path = dir.join("book.epub");
            let file = std::fs::File::create(&path).expect("создать архив");
            let mut zip = zip::ZipWriter::new(file);
            let options: zip::write::FileOptions<()> = zip::write::FileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            for (name, body) in self.files {
                zip.start_file(name, options).expect("начать запись");
                zip.write_all(body.as_bytes()).expect("записать содержимое");
            }
            zip.finish().expect("закрыть архив");
            path
        }
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

    fn parse_str(parts: Vec<(String, String)>) -> Result<Document, ParseError> {
        let buf = archive(parts);
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(buf)).expect("открыть архив");
        parse_archive(&mut zip, "book.epub")
    }

    /// Запаковывает набор файлов в zip целиком в памяти — тесты не трогают диск.
    fn archive(parts: Vec<(String, String)>) -> Vec<u8> {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options: zip::write::FileOptions<()> = zip::write::FileOptions::default();
        for (name, body) in parts {
            zip.start_file(name, options).expect("начать запись");
            zip.write_all(body.as_bytes()).expect("записать");
        }
        zip.finish().expect("закрыть архив").into_inner()
    }

    const CONTAINER: &str = r#"<?xml version="1.0"?>
<container xmlns="urn:oasis:names:tc:opendocument:xmlns:container" version="1.0">
<rootfiles><rootfile full-path="{PATH}"/></rootfiles></container>"#;

    fn container_for(opf_path: &str) -> String {
        CONTAINER.replace("{PATH}", opf_path)
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
        let texts: Vec<&str> = doc.blocks().iter().map(|b| b.text.as_str()).collect();
        assert_eq!(texts, ["One", "alpha", "Two", "beta"]);
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
    fn skips_script_and_style_content() {
        let xhtml = page(
            r#"<p>keep</p><script>var x = "drop";</script><style>.a{color:red}</style><p>also</p>"#,
        );
        let blocks = extracted(&xhtml);
        let texts: Vec<&str> = blocks.iter().map(|b| b.text.as_str()).collect();
        assert_eq!(texts, ["keep", "also"]);
    }

    #[test]
    fn br_inside_paragraph_becomes_newline() {
        let blocks = extracted(&page("<p>line one<br/>line two</p>"));
        assert_eq!(blocks[0].text, "line one\nline two");
    }

    #[test]
    fn inline_tags_do_not_split_paragraph() {
        let blocks = extracted(&page(r##"<p>a <em>b</em> c <a href="#fn1">d</a> e</p>"##));
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].text, "a *b* c d e");
    }

    #[test]
    fn em_and_strong_survive_as_inline_markers() {
        let blocks = extracted(&page(
            "<p>Обычный <em>курсив</em> и <strong>жирный</strong>, <b>b</b> и <i>i</i>.</p>",
        ));
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].text, "Обычный *курсив* и **жирный**, **b** и *i*.");
    }

    #[test]
    fn nested_block_continues_paragraph() {
        let blocks = extracted(&page("<div><p>outer <span>still outer</span></p></div>"));
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].text, "outer still outer");
    }

    #[test]
    fn block_tags_map_to_kinds() {
        let blocks = extracted(&page(
            "<h2>H</h2><p>p</p><blockquote>q</blockquote><li>l</li><pre>c</pre><hr/>",
        ));
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
        let blocks = extracted(&page("<p>a &amp; b &lt;tag&gt; &#8212; c</p>"));
        assert_eq!(blocks[0].text, "a & b <tag> — c");
    }

    #[test]
    fn unicode_survives_intact() {
        let blocks = extracted(&page("<p>Привет, мир! 日本語もね</p>"));
        assert_eq!(blocks[0].text, "Привет, мир! 日本語もね");
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
        let path = EpubBuilder::container("OEBPS/content.opf")
            .add("OEBPS/content.opf", &opf(&["c1"], r#"<item id="c1" href="c.xhtml"/>"#, "en"))
            .add("OEBPS/c.xhtml", &page("<h1>T</h1><p>body</p>"))
            .write(&dir);
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
    fn titles_normalize_for_matching() {
        assert_eq!(normalize_title("Chapter Two!"), "chapter two");
        assert_eq!(normalize_title("Chapter  Two"), "chapter two");
    }

    fn extracted(xhtml: &str) -> Vec<Block> {
        let mut blocks = Vec::new();
        extract_blocks(xhtml, &mut blocks);
        blocks
    }

    const NAV: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><body>
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
        let texts: Vec<&str> = doc.blocks().iter().map(|b| b.text.as_str()).collect();
        assert_eq!(texts, ["One", "a", "One A", "b", "Two", "c"]);
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
}
