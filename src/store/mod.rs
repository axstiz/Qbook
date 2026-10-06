//! SQLite-хранилище: книги, варианты, прогресс, закладки, кэш выравнивания.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, params};
use thiserror::Error;

use crate::model::{Anchor, BlockKind, Document};

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
/// Одна миграция на шаг `user_version`; индекс 0 отвечает за версию 1.
const MIGRATIONS: &[&str] = &[
    "
    CREATE TABLE IF NOT EXISTS books (
        id INTEGER PRIMARY KEY,
        path TEXT NOT NULL UNIQUE,
        title TEXT NOT NULL,
        base_lang TEXT NOT NULL,
        added_at INTEGER NOT NULL,
        last_opened_at INTEGER NOT NULL,
        mtime INTEGER NOT NULL,
        size INTEGER NOT NULL
    );
    CREATE TABLE IF NOT EXISTS variants (
        id INTEGER PRIMARY KEY,
        book_id INTEGER NOT NULL REFERENCES books(id) ON DELETE CASCADE,
        lang TEXT NOT NULL,
        path TEXT NOT NULL,
        kind TEXT NOT NULL,
        UNIQUE(book_id, lang)
    );
    CREATE TABLE IF NOT EXISTS progress (
        book_id INTEGER PRIMARY KEY REFERENCES books(id) ON DELETE CASCADE,
        anchor_block INTEGER NOT NULL,
        anchor_frac REAL NOT NULL,
        variant_lang TEXT NOT NULL,
        updated_at INTEGER NOT NULL
    );
    CREATE TABLE IF NOT EXISTS bookmarks (
        id INTEGER PRIMARY KEY,
        book_id INTEGER NOT NULL REFERENCES books(id) ON DELETE CASCADE,
        anchor_block INTEGER NOT NULL,
        anchor_frac REAL NOT NULL,
        label TEXT NOT NULL,
        created_at INTEGER NOT NULL
    );
    CREATE TABLE IF NOT EXISTS alignments (
        book_id INTEGER NOT NULL REFERENCES books(id) ON DELETE CASCADE,
        lang TEXT NOT NULL,
        base_hash INTEGER NOT NULL,
        var_hash INTEGER NOT NULL,
        map BLOB NOT NULL,
        coverage REAL NOT NULL,
        PRIMARY KEY (book_id, lang)
    );
",
    "ALTER TABLE progress ADD COLUMN percent REAL NOT NULL DEFAULT 0.0",
];

/// Кэшированная карта: сама карта и покрытие.
pub type CachedAlignment = (Vec<Option<usize>>, f32);

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("хранилище: {0}")]
    Sql(#[from] rusqlite::Error),
    #[error("каталог данных недоступен")]
    NoDataDir,
    #[error("непригодная карта выравнивания")]
    CorruptAlignment,
    #[error("{0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Book {
    pub id: i64,
    pub path: String,
    pub title: String,
    pub base_lang: String,
    pub added_at: i64,
    pub last_opened_at: i64,
    pub mtime: i64,
    pub size: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Variant {
    pub id: i64,
    pub book_id: i64,
    pub lang: String,
    pub path: String,
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Progress {
    pub anchor: Anchor,
    pub variant_lang: String,
    /// Прогресс чтения в процентах — его показывает полка без знания документа.
    pub percent: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Bookmark {
    pub id: i64,
    pub book_id: i64,
    pub anchor: Anchor,
    pub label: String,
    pub created_at: i64,
}

/// Открытое хранилище. Один экземпляр = одно соединение SQLite.
pub struct Store {
    conn: Connection,
    path: PathBuf,
}

impl Store {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let path = path.as_ref().to_path_buf();
        let conn = Connection::open(&path)?;
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        migrate(&conn)?;
        Ok(Self { conn, path })
    }

    /// Новое соединение с тем же файлом — для попыток, которые не должны
    /// терять основное соединение при ошибке.
    pub fn reopen(&self) -> Result<Self, StoreError> {
        Self::open(&self.path)
    }

    /// Открыть (создав каталог) хранилище `qbook.db` внутри директории.
    pub fn open_in(dir: impl AsRef<Path>) -> Result<Self, StoreError> {
        std::fs::create_dir_all(dir.as_ref())?;
        Self::open(dir.as_ref().join("qbook.db"))
    }

    /// Стандартное место: XDG data dir (`~/.local/share/qbook`).
    pub fn open_default() -> Result<Self, StoreError> {
        let dir = dirs::data_dir().ok_or(StoreError::NoDataDir)?;
        Self::open_in(dir.join("qbook"))
    }

    pub fn user_version(&self) -> Result<i64, StoreError> {
        Ok(self.conn.query_row("PRAGMA user_version", [], |row| row.get(0))?)
    }

    /// Добавить книгу. Повторное обращение к тому же пути возвращает прежний id.
    pub fn add_book(
        &self,
        path: &str,
        title: &str,
        base_lang: &str,
        mtime: i64,
        size: i64,
    ) -> Result<i64, StoreError> {
        let stamp = now();
        self.conn.execute(
            "INSERT INTO books (path, title, base_lang, added_at, last_opened_at, mtime, size)
             VALUES (?1, ?2, ?3, ?4, 0, ?5, ?6)
             ON CONFLICT(path) DO NOTHING",
            params![path, title, base_lang, stamp, mtime, size],
        )?;
        self.book_id(path).map(|id| id.expect("книга существует после вставки"))
    }

    pub fn book_id(&self, path: &str) -> Result<Option<i64>, StoreError> {
        Ok(self
            .conn
            .query_row("SELECT id FROM books WHERE path = ?1", [path], |row| row.get(0))
            .optional()?)
    }

    pub fn get_book(&self, id: i64) -> Result<Option<Book>, StoreError> {
        Ok(self
            .conn
            .query_row(
                "SELECT id, path, title, base_lang, added_at, last_opened_at, mtime, size
                 FROM books WHERE id = ?1",
                [id],
                book_from_row,
            )
            .optional()?)
    }

    pub fn list_books(&self) -> Result<Vec<Book>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, path, title, base_lang, added_at, last_opened_at, mtime, size
             FROM books ORDER BY id",
        )?;
        let books = stmt.query_map([], book_from_row)?.collect::<Result<Vec<_>, _>>()?;
        Ok(books)
    }

    pub fn touch_book(&self, id: i64) -> Result<(), StoreError> {
        self.conn
            .execute("UPDATE books SET last_opened_at = ?1 WHERE id = ?2", params![now(), id])?;
        Ok(())
    }

    pub fn delete_book(&self, id: i64) -> Result<(), StoreError> {
        self.conn.execute("DELETE FROM books WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn set_variant(
        &self,
        book_id: i64,
        lang: &str,
        path: &str,
        kind: &str,
    ) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO variants (book_id, lang, path, kind) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(book_id, lang) DO UPDATE SET path = ?3, kind = ?4",
            params![book_id, lang, path, kind],
        )?;
        Ok(())
    }

    pub fn variants(&self, book_id: i64) -> Result<Vec<Variant>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, book_id, lang, path, kind FROM variants WHERE book_id = ?1 ORDER BY id",
        )?;
        let variants = stmt
            .query_map([book_id], |row| {
                Ok(Variant {
                    id: row.get(0)?,
                    book_id: row.get(1)?,
                    lang: row.get(2)?,
                    path: row.get(3)?,
                    kind: row.get(4)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(variants)
    }

    pub fn set_progress(
        &self,
        book_id: i64,
        anchor: Anchor,
        variant_lang: &str,
        percent: f32,
    ) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO progress (book_id, anchor_block, anchor_frac, variant_lang, updated_at, percent)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(book_id) DO UPDATE SET
                anchor_block = ?2, anchor_frac = ?3, variant_lang = ?4, updated_at = ?5, percent = ?6",
            params![book_id, anchor.block as i64, anchor.frac as f64, variant_lang, now(), percent],
        )?;
        Ok(())
    }

    pub fn get_progress(&self, book_id: i64) -> Result<Option<Progress>, StoreError> {
        let progress = self
            .conn
            .query_row(
                "SELECT anchor_block, anchor_frac, variant_lang, percent
                 FROM progress WHERE book_id = ?1",
                [book_id],
                |row| {
                    let block: i64 = row.get(0)?;
                    let frac: f64 = row.get(1)?;
                    Ok(Progress {
                        anchor: Anchor::new(block as usize, frac as f32),
                        variant_lang: row.get(2)?,
                        percent: row.get(3)?,
                    })
                },
            )
            .optional()?;
        Ok(progress)
    }

    pub fn add_bookmark(
        &self,
        book_id: i64,
        anchor: Anchor,
        label: &str,
    ) -> Result<i64, StoreError> {
        self.conn.execute(
            "INSERT INTO bookmarks (book_id, anchor_block, anchor_frac, label, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![book_id, anchor.block as i64, anchor.frac as f64, label, now()],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn list_bookmarks(&self, book_id: i64) -> Result<Vec<Bookmark>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, book_id, anchor_block, anchor_frac, label, created_at
             FROM bookmarks WHERE book_id = ?1 ORDER BY id",
        )?;
        let marks = stmt
            .query_map([book_id], |row| {
                let block: i64 = row.get(2)?;
                let frac: f64 = row.get(3)?;
                Ok(Bookmark {
                    id: row.get(0)?,
                    book_id: row.get(1)?,
                    anchor: Anchor::new(block as usize, frac as f32),
                    label: row.get(4)?,
                    created_at: row.get(5)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(marks)
    }

    pub fn rename_bookmark(&self, id: i64, label: &str) -> Result<(), StoreError> {
        self.conn.execute("UPDATE bookmarks SET label = ?1 WHERE id = ?2", params![label, id])?;
        Ok(())
    }

    pub fn delete_bookmark(&self, id: i64) -> Result<(), StoreError> {
        self.conn.execute("DELETE FROM bookmarks WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Положить карту выравнивания в кэш под ключ `(book_id, lang, хэши)`.
    pub fn save_alignment(
        &self,
        book_id: i64,
        lang: &str,
        base_hash: u64,
        var_hash: u64,
        map: &[Option<usize>],
        coverage: f32,
    ) -> Result<(), StoreError> {
        let bytes = encode_map(map)?;
        self.conn.execute(
            "INSERT INTO alignments (book_id, lang, base_hash, var_hash, map, coverage)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(book_id, lang) DO UPDATE SET
                base_hash = ?3, var_hash = ?4, map = ?5, coverage = ?6",
            params![book_id, lang, base_hash as i64, var_hash as i64, bytes, coverage],
        )?;
        Ok(())
    }

    /// Взять карту из кэша; любое расхождение ключа (включая хэши) — промах.
    pub fn load_alignment(
        &self,
        book_id: i64,
        lang: &str,
        base_hash: u64,
        var_hash: u64,
    ) -> Result<Option<CachedAlignment>, StoreError> {
        let cached = self
            .conn
            .query_row(
                "SELECT map, coverage FROM alignments
                 WHERE book_id = ?1 AND lang = ?2 AND base_hash = ?3 AND var_hash = ?4",
                params![book_id, lang, base_hash as i64, var_hash as i64],
                |row| {
                    let bytes: Vec<u8> = row.get(0)?;
                    let coverage: f32 = row.get(1)?;
                    Ok((bytes, coverage))
                },
            )
            .optional()?;
        let (bytes, coverage) = match cached {
            Some(cached) => cached,
            None => return Ok(None),
        };
        Ok(Some((decode_map(&bytes)?, coverage)))
    }
}

/// FNV-1a по сигнатурам блоков: меняется при любой правке текста или типа блока.
pub fn document_hash(doc: &Document) -> u64 {
    let mut hash = FNV_OFFSET;
    for block in doc.blocks() {
        hash = fnv1a(&[kind_tag(block.kind)], hash);
        hash = fnv1a(block.text.as_bytes(), hash);
        hash = fnv1a(&[0xff], hash);
    }
    hash
}

fn fnv1a(bytes: &[u8], seed: u64) -> u64 {
    let mut hash = seed;
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

fn kind_tag(kind: BlockKind) -> u8 {
    match kind {
        BlockKind::Heading(level) => level.saturating_add(10),
        BlockKind::Paragraph => 1,
        BlockKind::Quote => 2,
        BlockKind::ListItem => 3,
        BlockKind::Code => 4,
        BlockKind::Verse => 5,
        BlockKind::Rule => 6,
        BlockKind::Blank => 7,
    }
}

fn book_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Book> {
    Ok(Book {
        id: row.get(0)?,
        path: row.get(1)?,
        title: row.get(2)?,
        base_lang: row.get(3)?,
        added_at: row.get(4)?,
        last_opened_at: row.get(5)?,
        mtime: row.get(6)?,
        size: row.get(7)?,
    })
}

fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

fn migrate(conn: &Connection) -> Result<(), StoreError> {
    let current: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    for (index, sql) in MIGRATIONS.iter().enumerate().skip(current as usize) {
        conn.execute_batch(sql)?;
        conn.pragma_update(None, "user_version", index as i64 + 1)?;
    }
    Ok(())
}

/// Карта в BLOB: длина + `i + 1` на блок, ноль — несопоставленный блок.
fn encode_map(map: &[Option<usize>]) -> Result<Vec<u8>, StoreError> {
    let mut out = Vec::with_capacity(4 + map.len() * 4);
    out.extend_from_slice(&(map.len() as u32).to_le_bytes());
    for slot in map {
        let value = match slot {
            None => 0,
            &Some(i) => u32::try_from(i)
                .ok()
                .and_then(|i| i.checked_add(1))
                .ok_or(StoreError::CorruptAlignment)?,
        };
        out.extend_from_slice(&value.to_le_bytes());
    }
    Ok(out)
}

fn decode_map(bytes: &[u8]) -> Result<Vec<Option<usize>>, StoreError> {
    let (len_bytes, rest) = bytes.split_at_checked(4).ok_or(StoreError::CorruptAlignment)?;
    let len = u32::from_le_bytes(len_bytes.try_into().expect("ровно 4 байта")) as usize;
    if rest.len() != len * 4 {
        return Err(StoreError::CorruptAlignment);
    }
    let (cells, _) = rest.as_chunks::<4>();
    cells
        .iter()
        .map(|cell| {
            let value = u32::from_le_bytes(*cell);
            Ok(if value == 0 { None } else { Some(value as usize - 1) })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_blob_round_trip() {
        let map = vec![None, Some(0), Some(42), Some(1_000_000)];
        let bytes = encode_map(&map).expect("кодирование");
        assert_eq!(decode_map(&bytes).expect("декодирование"), map);
        assert_eq!(decode_map(&encode_map(&[]).expect("кодирование")).expect("декодирование"), []);
    }

    #[test]
    fn corrupt_blob_is_rejected() {
        assert!(matches!(decode_map(&[]), Err(StoreError::CorruptAlignment)));
        assert!(matches!(decode_map(&[1, 0, 0]), Err(StoreError::CorruptAlignment)));
        let mut bytes = encode_map(&[Some(0)]).expect("кодирование");
        bytes.push(0);
        assert!(matches!(decode_map(&bytes), Err(StoreError::CorruptAlignment)));
        assert!(matches!(
            encode_map(&[Some(u32::MAX as usize)]),
            Err(StoreError::CorruptAlignment)
        ));
    }
}
