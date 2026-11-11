//! Медиа: декодирование картинок книги и отрисовка в терминальные ячейки.
//!
//! Tier 1 (все терминалы): полублок `▀` с fg/bg-цветом — пиксельная пара
//! сверху/снизу. Кэш декодированных RGBA-картинок ограничен по памяти (LRU).

#[cfg(feature = "svg")]
pub mod svg;

use std::collections::HashMap;
use std::path::Path;

use image::DynamicImage;

/// Стандартная ёмкость кэша декодированных картинок, байт RGBA.
pub const CACHE_CAP_BYTES: usize = 32 * 1024 * 1024;

/// Ресурсы книги: байты картинок, лежащие внутри контейнера.
pub trait Resources {
    fn read_bytes(&mut self, name: &str) -> Option<Vec<u8>>;
}

/// Ресурсы EPUB: zip-архив книги.
pub struct ZipResources {
    zip: zip::ZipArchive<std::fs::File>,
}

impl ZipResources {
    /// Открывает zip контейнера; сбой возвращает `None` — книга читается
    /// без картинок, alt-текст остаётся.
    pub fn open(path: &Path) -> Option<Self> {
        let file = std::fs::File::open(path).ok()?;
        let zip = zip::ZipArchive::new(file).ok()?;
        Some(Self { zip })
    }
}

impl Resources for ZipResources {
    fn read_bytes(&mut self, name: &str) -> Option<Vec<u8>> {
        let mut entry = self.zip.by_name(name).ok()?;
        let mut out = Vec::new();
        std::io::Read::read_to_end(&mut entry, &mut out).ok()?;
        Some(out)
    }
}

/// Декодированная картинка в RGBA, готовая к масштабу терминала.
#[derive(Debug, Clone)]
pub struct ImageData {
    pub width: u32,
    pub height: u32,
    pub rgba: image::RgbaImage,
}

/// Кэш декодированных картинок и уже переведённых в ячейки половинок.
pub struct Media {
    images: HashMap<String, ImageData>,
    /// Порядок LRU: свежий ресурс в хвост, вытеснение — из головы.
    order: std::collections::VecDeque<String>,
    cells: HashMap<(String, u32, u32), Vec<Vec<HalfCell>>>,
    cap_bytes: usize,
    used_bytes: usize,
}

impl Media {
    pub fn new(cap_bytes: usize) -> Self {
        Self {
            images: HashMap::new(),
            order: std::collections::VecDeque::new(),
            cells: HashMap::new(),
            cap_bytes,
            used_bytes: 0,
        }
    }

    pub fn used_bytes(&self) -> usize {
        self.used_bytes
    }

    /// Декодированная картинка из кэша; при промахе — декодирует и кэширует.
    pub fn image(
        &mut self,
        resource: &str,
        source: &mut dyn FnMut(&str) -> Option<Vec<u8>>,
    ) -> Option<&ImageData> {
        if !self.images.contains_key(resource) {
            let bytes = source(resource)?;
            let image = decode(&bytes)?;
            self.used_bytes += image.rgba.as_raw().len();
            self.images.insert(resource.to_owned(), image);
            self.land(resource);
        }
        self.touch(resource);
        self.images.get(resource)
    }

    /// Ячейки half-block. Кэш по triple «ресурс × колонки × строки»:
    /// ресайз каждого кадра — дорого, поэтому его тоже не делаем лишний раз.
    pub fn cells(
        &mut self,
        resource: &str,
        cols: u32,
        rows: u32,
        source: &mut dyn FnMut(&str) -> Option<Vec<u8>>,
    ) -> Option<&[Vec<HalfCell>]> {
        let key = (resource.to_owned(), cols, rows);
        if !self.cells.contains_key(&key) {
            let bytes = source(resource)?;
            let image = decode(&bytes)?;
            let grid = half_cells(&image, cols, rows);
            self.used_bytes += image.rgba.as_raw().len();
            self.images.insert(resource.to_owned(), image);
            self.land(resource);
            self.cells.insert(key.clone(), grid);
        }
        let grid = self.cells.get(&key)?;
        Some(grid.as_slice())
    }

    /// Регистрирует свежий ресурс и гоняет вытеснение.
    fn land(&mut self, resource: &str) {
        self.touch(resource);
        self.evict();
    }

    /// Свежий доступ: ресурс переносится в хвост очереди.
    fn touch(&mut self, resource: &str) {
        if let Some(index) = self.order.iter().position(|key| key.as_str() == resource) {
            self.order.remove(index);
        }
        self.order.push_back(resource.to_owned());
    }

    /// Вытесняет самые старые картинки (голова очереди LRU), пока не влезем.
    fn evict(&mut self) {
        while self.used_bytes > self.cap_bytes {
            let Some(oldest) = self.order.front().cloned() else { break };
            self.order.pop_front();
            if let Some(image) = self.images.remove(&oldest) {
                self.used_bytes = self.used_bytes.saturating_sub(image.rgba.as_raw().len());
            }
            let keys: Vec<(String, u32, u32)> =
                self.cells.keys().filter(|(res, _, _)| res == &oldest).cloned().collect();
            for key in keys {
                self.cells.remove(&key);
            }
        }
    }

    pub fn clear(&mut self) {
        self.images.clear();
        self.order.clear();
        self.cells.clear();
        self.used_bytes = 0;
    }
}

/// Декодирует PNG/JPEG/GIF/WebP (и SVG при feature `svg`) байты в RGBA.
pub fn decode(bytes: &[u8]) -> Option<ImageData> {
    #[cfg(feature = "svg")]
    if svg::is_svg(bytes) {
        return svg::render(bytes);
    }
    let image = image::load_from_memory(bytes).ok()?;
    match image {
        DynamicImage::ImageRgba8(rgba) => {
            Some(ImageData { width: rgba.width(), height: rgba.height(), rgba })
        }
        image => {
            let rgba = image.to_rgba8();
            Some(ImageData { width: rgba.width(), height: rgba.height(), rgba })
        }
    }
}

/// Цвет одной половины ячейки: непрозрачность переводится в Reset (фон).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellColor {
    Reset,
    Rgb(u8, u8, u8),
}

/// Точки ячейки half-block: верх и низ, каждый — с полупрозрачным сбросом.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HalfCell {
    pub top: CellColor,
    pub bottom: CellColor,
}

/// Масштаб RGBA в ячейки half-block: `(колонки, строки) → ячейки`.
/// Пиксельные размеры на выходе: ширина ровно колонок, высота — строки.
pub fn half_cells(image: &ImageData, cols: u32, rows: u32) -> Vec<Vec<HalfCell>> {
    let cols = cols.max(1);
    let rows = rows.max(1);
    let resized = image::imageops::resize(
        &image.rgba,
        cols * 2,
        rows * 2,
        image::imageops::FilterType::Triangle,
    );
    let mut out = Vec::with_capacity(rows as usize);
    for cy in 0..rows {
        let mut line = Vec::with_capacity(cols as usize);
        for cx in 0..cols {
            let px = |x: u32, y: u32| -> CellColor {
                let pixel = resized.get_pixel(x, y);
                if pixel.0[3] >= 128 {
                    CellColor::Rgb(pixel.0[0], pixel.0[1], pixel.0[2])
                } else {
                    CellColor::Reset
                }
            };
            let x = cx * 2;
            let top = px(x, cy * 2);
            let bottom = px(x, cy * 2 + 1);
            line.push(HalfCell { top, bottom });
        }
        out.push(line);
    }
    out
}

/// Высота картинки в терминальных строках: ячейка терминала примерно вдвое
/// выше своей ширины, поэтому делим высоту пикселей ещё и на два.
pub fn image_rows_pixels(width_px: u32, height_px: u32, cols: u32) -> u32 {
    if width_px == 0 || height_px == 0 || cols == 0 {
        return 0;
    }
    // Ячейка терминала ≈ 2:1 (высота/ширина), поэтому пиксели растягивать
    // сильно нельзя: строки = height * cols / width / 2.
    let rows = u64::from(height_px) * u64::from(cols) / (u64::from(width_px) * 2);
    rows.max(1) as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgba, RgbaImage};

    fn solid(w: u32, h: u32, red: u8, green: u8, blue: u8, alpha: u8) -> ImageData {
        let rgba = RgbaImage::from_pixel(w, h, Rgba([red, green, blue, alpha]));
        ImageData { width: w, height: h, rgba }
    }

    #[test]
    fn half_block_uses_top_bottom_colors() {
        // 2×2 пикселя: верхняя строка зелёная, нижняя прозрачная →
        // одна ячейка 1×1: сверху зелёный, снизу — фон терминала.
        let rgba = RgbaImage::from_fn(2, 2, |_, y| {
            if y == 0 { Rgba([0, 255, 0, 255]) } else { Rgba([0, 255, 0, 0]) }
        });
        let image = ImageData { width: 2, height: 2, rgba };
        let half = half_cells(&image, 1, 1);
        assert_eq!(half.len(), 1);
        assert_eq!(half[0][0].top, CellColor::Rgb(0, 255, 0));
        assert_eq!(half[0][0].bottom, CellColor::Reset, "нижний пиксель — прозрачный сброс");
    }

    #[test]
    fn resize_maps_full_imati_to_cells_grid() {
        let image = solid(4, 4, 255, 0, 0, 255);
        let grid = half_cells(&image, 2, 2);
        assert_eq!(grid.len(), 2);
        for line in &grid {
            assert_eq!(line.len(), 2);
            for cell in line {
                assert_eq!(cell.top, CellColor::Rgb(255, 0, 0));
                assert_eq!(cell.bottom, CellColor::Rgb(255, 0, 0));
            }
        }
    }

    #[test]
    fn row_height_respects_cell_aspect() {
        // 40×80 пикселей, колонка 20 → 20/40 * 80/2 = 20 строк?.. нет:
        // rows = height * cols / width / 2 = 80 * 20 / 40 / 2 = 20.
        assert_eq!(image_rows_pixels(40, 80, 20), 20);
        // Широкая картинка 160×40 при колонке 20 → 40*20/160/2 = 2.5 → 3? Округляем вниз?.
        assert_eq!(image_rows_pixels(160, 40, 20), 2, "округление вниз и минимум 1");
        assert_eq!(image_rows_pixels(0, 40, 20), 0);
    }

    #[test]
    fn lru_cache_evicts_oldest_when_over_capacity() {
        let mut media = Media::new(30);
        let mut source = |name: &str| -> Option<Vec<u8>> {
            // PNG картинки 2×2 — декодируется в 2×2 RGBA = 16 байт.
            let _ = name;
            let rgba = RgbaImage::from_pixel(2, 2, Rgba([255, 0, 0, 255]));
            let mut out = std::io::Cursor::new(Vec::new());
            image::DynamicImage::ImageRgba8(rgba)
                .write_to(&mut out, image::ImageFormat::Png)
                .expect("кодирование PNG");
            Some(out.into_inner())
        };
        for index in 0..4 {
            media.image(&format!("img{index}.png"), &mut source).expect("картинка");
        }
        assert!(media.used_bytes() <= 30, "кэш не режется: used = {}", media.used_bytes());
        // Последняя картинка остаётся, старые вытеснены.
        let mut second_source =
            |_name: &str| -> Option<Vec<u8>> { panic!("должна быть в кэше") };
        let maybe = media.image("img3.png", &mut second_source);
        assert!(maybe.is_some(), "последняя картинка остаётся в кэше");
    }

    #[test]
    fn cache_clear_releases_bytes() {
        let mut media = Media::new(CACHE_CAP_BYTES);
        let mut source = |name: &str| -> Option<Vec<u8>> {
            // Продуманный байт PNG: используйте один届中国 image.design курсе(…
            let _ = name;
            let image = solid(1, 1, 0, 0, 0, 255);
            let mut out = std::io::Cursor::new(Vec::new());
            image::DynamicImage::ImageRgba8(image.rgba)
                .write_to(&mut out, image::ImageFormat::Png)
                .ok()?;
            Some(out.into_inner())
        };
        media.image("a.png", &mut source).expect("картинка");
        assert!(media.used_bytes() > 0);
        media.clear();
        assert_eq!(media.used_bytes(), 0);
    }
}
