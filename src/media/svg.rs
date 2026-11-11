//! Растеризация SVG через `resvg`: размер из документа, рендер в RGBA.
//!
//! Текст внутри SVG рендерится только если нашлись системные шрифты; формы
//! (линии, прямоугольники, стрелки) рисуются всегда.

use std::sync::{Arc, OnceLock};

use resvg::tiny_skia::{Pixmap, Transform};
use resvg::usvg;

use super::ImageData;

/// Начинаются ли байты с SVG-документа (пробелы и BOM пропускаются).
pub fn is_svg(bytes: &[u8]) -> bool {
    let text = strip_bom(bytes);
    let text = text.trim_ascii_start();
    text.starts_with(b"<svg") || text.starts_with(b"<?xml")
}

fn strip_bom(bytes: &[u8]) -> &[u8] {
    bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes)
}

/// Размер документа: `viewBox` или `width`/`height`.
pub fn size(bytes: &[u8]) -> Option<(u32, u32)> {
    let tree = tree(bytes)?;
    let int = tree.size().to_int_size();
    Some((int.width(), int.height()))
}

/// Рендер SVG в RGBA естественного размера документа.
pub fn render(bytes: &[u8]) -> Option<ImageData> {
    let tree = tree(bytes)?;
    let int = tree.size().to_int_size();
    let (width, height) = (int.width(), int.height());
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > 64_000_000 {
        return None;
    }
    let mut pixmap = Pixmap::new(width, height)?;
    resvg::render(&tree, Transform::identity(), &mut pixmap.as_mut());
    Some(ImageData { width, height, rgba: demultiply(pixmap) })
}

fn tree(bytes: &[u8]) -> Option<usvg::Tree> {
    let options = usvg::Options { fontdb: Arc::clone(font_database()), ..usvg::Options::default() };
    usvg::Tree::from_data(bytes, &options).ok()
}

/// Системные шрифты кэшируются один раз на процесс.
fn font_database() -> &'static Arc<usvg::fontdb::Database> {
    static DB: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();
    DB.get_or_init(|| {
        let mut db = usvg::fontdb::Database::new();
        db.load_system_fonts();
        Arc::new(db)
    })
}

/// `resvg` рисует premultiplied RGBA; приводим к обычной альфе.
fn demultiply(pixmap: Pixmap) -> image::RgbaImage {
    let (width, height) = (pixmap.width(), pixmap.height());
    let mut out = image::RgbaImage::new(width, height);
    let data = pixmap.take();
    let (chunks, _tail) = data.as_chunks::<4>();
    for (pixel, chunk) in out.pixels_mut().zip(chunks) {
        let (r, g, b, a) = (chunk[0], chunk[1], chunk[2], chunk[3]);
        let (r, g, b, a) = (u32::from(r), u32::from(g), u32::from(b), u32::from(a));
        if a == 0 {
            *pixel = image::Rgba([0, 0, 0, 0]);
        } else if a == 255 {
            *pixel = image::Rgba([r as u8, g as u8, b as u8, 255]);
        } else {
            *pixel = image::Rgba([
                ((r * 255 + a / 2) / a) as u8,
                ((g * 255 + a / 2) / a) as u8,
                ((b * 255 + a / 2) / a) as u8,
                a as u8,
            ]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const RECT: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100">
<rect x="10" y="10" width="80" height="80" fill="#ff0000"/></svg>"##;

    const TEXT: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 200 60">
<rect x="0" y="0" width="200" height="60" fill="#ffffff"/>
<text x="10" y="40" font-size="30" fill="#000000">схема 1-2</text></svg>"##;

    #[test]
    fn detects_svg_with_bom_and_xml_prolog() {
        assert!(is_svg(RECT.as_bytes()));
        assert!(is_svg(format!("<?xml version=\"1.0\"?>\n{RECT}").as_bytes()));
        assert!(!is_svg(b"\x89PNG\r\n\x1a\n"));
        assert!(!is_svg(b"GIF89a"));
    }

    #[test]
    fn size_comes_from_viewbox() {
        assert_eq!(size(RECT.as_bytes()), Some((100, 100)));
        assert_eq!(size(TEXT.as_bytes()), Some((200, 60)));
    }

    #[test]
    fn renders_shapes() {
        let image = render(RECT.as_bytes()).expect("рендер");
        assert_eq!(image.rgba.dimensions(), (100, 100));
        let center = image.rgba.get_pixel(50, 50);
        assert!(center.0[0] > 200, "красный в центре: {center:?}");
        assert_eq!(center.0[3], 255);
        // Фон прозрачен.
        assert_eq!(image.rgba.get_pixel(5, 5).0[3], 0);
    }

    #[test]
    fn text_is_rendered_when_fonts_exist_or_dropped_gracefully() {
        let image = render(TEXT.as_bytes()).expect("рендер");
        assert_eq!(image.rgba.dimensions(), (200, 60));
        // Фон белый — непрозрачный пиксель есть в любом случае.
        assert_eq!(image.rgba.get_pixel(100, 10).0[3], 255);
    }

    #[test]
    fn garbage_is_none() {
        assert!(render(b"not svg at all").is_none());
        assert!(size(b"<svg xmlns=\"http://www.w3.org/2000/svg\"").is_none());
    }
}
