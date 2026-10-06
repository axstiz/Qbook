//! Хранилище: схема, CRUD и кэш выравнивания.

use qbook::model::{Anchor, Block, BlockKind, Document};
use qbook::store::{Book, Bookmark, Progress, Store, Variant, document_hash};

fn doc(texts: &[&str]) -> Document {
    Document::new("ru", "T", texts.iter().map(|t| Block::new(BlockKind::Paragraph, *t)).collect())
}

#[test]
fn fresh_database_migrates_and_survives_reopen() {
    let dir = tempfile::tempdir().expect("временный каталог");
    let path = dir.path().join("qbook.db");
    {
        let store = Store::open(&path).expect("открытие");
        assert_eq!(store.user_version().expect("версия"), 2);
        assert!(store.list_books().expect("список пуст").is_empty());
    }
    {
        let store = Store::open(&path).expect("повторное открытие");
        assert_eq!(store.user_version().expect("версия"), 2);
        assert!(store.list_books().expect("список пуст").is_empty());
    }
}

#[test]
fn open_in_creates_the_directory_layout() {
    let dir = tempfile::tempdir().expect("временный каталог");
    let store = Store::open_in(dir.path()).expect("открытие");
    assert!(dir.path().join("qbook.db").exists(), "файл БД создан");
    drop(store);
}

#[test]
fn book_crud_round_trip() {
    let dir = tempfile::tempdir().expect("временный каталог");
    let store = Store::open(dir.path().join("qbook.db")).expect("открытие");

    let id = store.add_book("/books/a.epub", "Книга", "ru", 1000, 2048).expect("добавление");
    let again =
        store.add_book("/books/a.epub", "Книга", "ru", 1000, 2048).expect("повторное добавление");
    assert_eq!(again, id, "добавление по пути идемпотентно");
    assert_eq!(store.book_id("/books/a.epub").expect("поиск"), Some(id));
    assert_eq!(store.book_id("/books/нет.epub").expect("поиск"), None);

    let books: Vec<Book> = store.list_books().expect("список");
    assert_eq!(books.len(), 1);
    assert_eq!(books[0].title, "Книга");
    assert_eq!(books[0].base_lang, "ru");
    assert_eq!(books[0].size, 2048);

    store.touch_book(id).expect("отметка");
    let touched = store.get_book(id).expect("книга").expect("есть");
    assert!(touched.last_opened_at >= touched.added_at);

    store.set_variant(id, "en", "/books/a.en.epub", "epub").expect("вариант");
    store.set_variant(id, "en", "/books/a.en.v2.epub", "epub").expect("вариант");
    let variants: Vec<Variant> = store.variants(id).expect("варианты");
    assert_eq!(variants.len(), 1, "UNIQUE(book_id, lang) перезаписывает путь");
    assert_eq!(variants[0].lang, "en");
    assert_eq!(variants[0].path, "/books/a.en.v2.epub");
}

#[test]
fn progress_round_trip_upserts() {
    let dir = tempfile::tempdir().expect("временный каталог");
    let store = Store::open(dir.path().join("qbook.db")).expect("открытие");
    let id = store.add_book("/b.epub", "T", "ru", 1, 1).expect("книга");

    assert!(store.get_progress(id).expect("чтение").is_none());
    store.set_progress(id, Anchor::new(42, 0.25), "en", 12.5).expect("запись");
    store.set_progress(id, Anchor::new(7, 0.5), "ru", 63.0).expect("перезапись");

    let p: Progress = store.get_progress(id).expect("чтение").expect("есть");
    assert_eq!(p.anchor, Anchor::new(7, 0.5));
    assert_eq!(p.variant_lang, "ru");
    assert!((p.percent - 63.0).abs() < 1e-6);
}

#[test]
fn bookmark_crud() {
    let dir = tempfile::tempdir().expect("временный каталог");
    let store = Store::open(dir.path().join("qbook.db")).expect("открытие");
    let id = store.add_book("/b.epub", "T", "ru", 1, 1).expect("книга");

    let first = store.add_bookmark(id, Anchor::new(10, 0.5), "глава 1").expect("закладка");
    store.add_bookmark(id, Anchor::new(99, 0.0), "").expect("закладка");

    let marks: Vec<Bookmark> = store.list_bookmarks(id).expect("список");
    assert_eq!(marks.len(), 2);
    assert_eq!(marks[0].id, first);
    assert_eq!(marks[0].anchor, Anchor::new(10, 0.5));
    assert_eq!(marks[0].label, "глава 1");

    store.rename_bookmark(first, "новая метка").expect("переименование");
    let marks: Vec<Bookmark> = store.list_bookmarks(id).expect("список");
    assert_eq!(marks[0].label, "новая метка");

    store.delete_bookmark(first).expect("удаление");
    let marks: Vec<Bookmark> = store.list_bookmarks(id).expect("список");
    assert_eq!(marks.len(), 1);
    assert_eq!(marks[0].anchor, Anchor::new(99, 0.0));
}

#[test]
fn deleting_a_book_cascades() {
    let dir = tempfile::tempdir().expect("временный каталог");
    let store = Store::open(dir.path().join("qbook.db")).expect("открытие");
    let id = store.add_book("/b.epub", "T", "ru", 1, 1).expect("книга");
    store.set_variant(id, "en", "/b.en.epub", "epub").expect("вариант");
    store.set_progress(id, Anchor::START, "ru", 0.0).expect("прогресс");
    store.add_bookmark(id, Anchor::START, "").expect("закладка");
    store.save_alignment(id, "en", 1, 2, &[Some(0)], 1.0).expect("кэш");

    store.delete_book(id).expect("удаление");

    assert!(store.list_books().expect("книги").is_empty());
    assert!(store.variants(id).expect("варианты").is_empty());
    assert!(store.get_progress(id).expect("прогресс").is_none());
    assert!(store.list_bookmarks(id).expect("закладки").is_empty());
    assert!(store.load_alignment(id, "en", 1, 2).expect("кэш").is_none());
}

#[test]
fn alignment_cache_keys_on_hashes_and_language() {
    let dir = tempfile::tempdir().expect("временный каталог");
    let store = Store::open(dir.path().join("qbook.db")).expect("открытие");
    let id = store.add_book("/b.epub", "T", "ru", 1, 1).expect("книга");

    store.save_alignment(id, "en", 111, 222, &[Some(0), None, Some(1)], 0.66).expect("сохранение");
    let (map, coverage) = store.load_alignment(id, "en", 111, 222).expect("чтение").expect("есть");
    assert_eq!(map, vec![Some(0), None, Some(1)]);
    assert!((coverage - 0.66).abs() < 1e-6);

    assert!(
        store.load_alignment(id, "en", 111, 999).expect("чтение").is_none(),
        "иной хэш варианта"
    );
    assert!(store.load_alignment(id, "fr", 111, 222).expect("чтение").is_none(), "иной язык");

    store.save_alignment(id, "en", 333, 444, &[Some(0)], 1.0).expect("перезапись");
    assert!(
        store.load_alignment(id, "en", 111, 222).expect("чтение").is_none(),
        "старый ключ затёрт"
    );
    assert!(store.load_alignment(id, "en", 333, 444).expect("чтение").is_some());
}

#[test]
fn document_hash_changes_when_content_changes() {
    let original = doc(&["Первый абзац.", "Второй абзац."]);
    let edited = doc(&["Первый абзац!", "Второй абзац."]);
    let retyped = Document::new(
        "ru",
        "T",
        vec![
            Block::new(BlockKind::Paragraph, "Первый абзац."),
            Block::new(BlockKind::Quote, "Второй абзац."),
        ],
    );

    assert_eq!(document_hash(&original), document_hash(&original), "хэш детерминирован");
    assert_ne!(document_hash(&original), document_hash(&edited), "правка текста меняет хэш");
    assert_ne!(document_hash(&original), document_hash(&retyped), "смена типа меняет хэш");
}
