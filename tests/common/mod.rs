//! Общие хелперы интеграционных тестов v2.

/// Собирает мини-EPUB `mimetype → container.xml → book.opf → spine-документы`.
/// Nav-оглавления нет — итоговый TOC выводится из блоков-заголовков.
/// Каждый элемент `chapters` — цельный XHTML-документ.
#[allow(dead_code)]
pub fn write_mini_epub(path: &std::path::Path, chapters: &[String]) {
    use std::io::Write;
    use zip::write::SimpleFileOptions;
    use zip::{CompressionMethod, ZipWriter};

    let opf = {
        let mut items = String::new();
        let mut refs = String::new();
        for (i, _) in chapters.iter().enumerate() {
            items.push_str(&format!(
                "<item id=\"c{i}\" href=\"chapter{i}.xhtml\" media-type=\"application/xhtml+xml\"/>\n\
                 "
            ));
            refs.push_str(&format!("<itemref idref=\"c{i}\"/>"));
        }
        format!(
            "<?xml version=\"1.0\"?>\
             <package xmlns=\"http://www.idpf.org/2007/opf\" version=\"3.0\" unique-identifier=\"uid\">\
             <metadata xmlns:dc=\"http://purl.org/dc/elements/1.1/\">\
             <dc:identifier id=\"uid\">qbook-test</dc:identifier>\
             <dc:title>Test</dc:title><dc:language>en</dc:language>\
             </metadata>\
             <manifest>{items}</manifest><spine>{refs}</spine></package>"
        )
    };
    let container = "<?xml version=\"1.0\"?><container version=\"1.0\" xmlns=\"urn:oasis:names:tc:opendocument:xmlns:container\">\
         <rootfiles><rootfile full-path=\"book.opf\" media-type=\"application/oebps-package+xml\"/></rootfiles></container>";

    let file = std::fs::File::create(path).expect("создать epub");
    let mut zip = ZipWriter::new(file);
    let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    let deflate = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    let seed = |z: &mut ZipWriter<std::fs::File>, name: &str, body: &str, opts| {
        z.start_file(name, opts).expect("запись-заголовок");
        z.write_all(body.as_bytes()).expect("запись-тело");
    };
    seed(&mut zip, "mimetype", "application/epub+zip", stored);
    seed(&mut zip, "META-INF/container.xml", container, deflate);
    seed(&mut zip, "book.opf", &opf, deflate);
    for (i, body) in chapters.iter().enumerate() {
        seed(&mut zip, &format!("chapter{i}.xhtml"), body, deflate);
    }
    zip.finish().expect("закрыть epub");
}
