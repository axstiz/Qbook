//! kitty graphics protocol: передача картинок и Unicode placeholders.
//!
//! Картинка передаётся терминалу один раз (`a=t,U=1,c=,r=` — transmit плюс
//! виртуальный placement, `f=100` PNG, `q=2` без ответов), а в ячейки текста
//! пишется `U+10EEEE` с диакритиками строки/колонки; image id кодируется
//! в цвете переднего плана (24 бита). Скролл и позиционирование выполняет
//! сам терминал.

/// Символ-плейсхолдер kitty.
pub const PLACEHOLDER: char = '\u{10EEEE}';

/// Диакритики из `rowcolumn-diacritics.txt`: индекс = число (0-based row/col).
/// В kitty декодер даёт «позиция + 1», ноль означает «не задано».
pub const DIACRITICS: [char; 297] = [
    '\u{0305}',
    '\u{030d}',
    '\u{030e}',
    '\u{0310}',
    '\u{0312}',
    '\u{033d}',
    '\u{033e}',
    '\u{033f}',
    '\u{0346}',
    '\u{034a}',
    '\u{034b}',
    '\u{034c}',
    '\u{0350}',
    '\u{0351}',
    '\u{0352}',
    '\u{0357}',
    '\u{035b}',
    '\u{0363}',
    '\u{0364}',
    '\u{0365}',
    '\u{0366}',
    '\u{0367}',
    '\u{0368}',
    '\u{0369}',
    '\u{036a}',
    '\u{036b}',
    '\u{036c}',
    '\u{036d}',
    '\u{036e}',
    '\u{036f}',
    '\u{0483}',
    '\u{0484}',
    '\u{0485}',
    '\u{0486}',
    '\u{0487}',
    '\u{0592}',
    '\u{0593}',
    '\u{0594}',
    '\u{0595}',
    '\u{0597}',
    '\u{0598}',
    '\u{0599}',
    '\u{059c}',
    '\u{059d}',
    '\u{059e}',
    '\u{059f}',
    '\u{05a0}',
    '\u{05a1}',
    '\u{05a8}',
    '\u{05a9}',
    '\u{05ab}',
    '\u{05ac}',
    '\u{05af}',
    '\u{05c4}',
    '\u{0610}',
    '\u{0611}',
    '\u{0612}',
    '\u{0613}',
    '\u{0614}',
    '\u{0615}',
    '\u{0616}',
    '\u{0617}',
    '\u{0657}',
    '\u{0658}',
    '\u{0659}',
    '\u{065a}',
    '\u{065b}',
    '\u{065d}',
    '\u{065e}',
    '\u{06d6}',
    '\u{06d7}',
    '\u{06d8}',
    '\u{06d9}',
    '\u{06da}',
    '\u{06db}',
    '\u{06dc}',
    '\u{06df}',
    '\u{06e0}',
    '\u{06e1}',
    '\u{06e2}',
    '\u{06e4}',
    '\u{06e7}',
    '\u{06e8}',
    '\u{06eb}',
    '\u{06ec}',
    '\u{0730}',
    '\u{0732}',
    '\u{0733}',
    '\u{0735}',
    '\u{0736}',
    '\u{073a}',
    '\u{073d}',
    '\u{073f}',
    '\u{0740}',
    '\u{0741}',
    '\u{0743}',
    '\u{0745}',
    '\u{0747}',
    '\u{0749}',
    '\u{074a}',
    '\u{07eb}',
    '\u{07ec}',
    '\u{07ed}',
    '\u{07ee}',
    '\u{07ef}',
    '\u{07f0}',
    '\u{07f1}',
    '\u{07f3}',
    '\u{0816}',
    '\u{0817}',
    '\u{0818}',
    '\u{0819}',
    '\u{081b}',
    '\u{081c}',
    '\u{081d}',
    '\u{081e}',
    '\u{081f}',
    '\u{0820}',
    '\u{0821}',
    '\u{0822}',
    '\u{0823}',
    '\u{0825}',
    '\u{0826}',
    '\u{0827}',
    '\u{0829}',
    '\u{082a}',
    '\u{082b}',
    '\u{082c}',
    '\u{082d}',
    '\u{0951}',
    '\u{0953}',
    '\u{0954}',
    '\u{0f82}',
    '\u{0f83}',
    '\u{0f86}',
    '\u{0f87}',
    '\u{135d}',
    '\u{135e}',
    '\u{135f}',
    '\u{17dd}',
    '\u{193a}',
    '\u{1a17}',
    '\u{1a75}',
    '\u{1a76}',
    '\u{1a77}',
    '\u{1a78}',
    '\u{1a79}',
    '\u{1a7a}',
    '\u{1a7b}',
    '\u{1a7c}',
    '\u{1b6b}',
    '\u{1b6d}',
    '\u{1b6e}',
    '\u{1b6f}',
    '\u{1b70}',
    '\u{1b71}',
    '\u{1b72}',
    '\u{1b73}',
    '\u{1cd0}',
    '\u{1cd1}',
    '\u{1cd2}',
    '\u{1cda}',
    '\u{1cdb}',
    '\u{1ce0}',
    '\u{1dc0}',
    '\u{1dc1}',
    '\u{1dc3}',
    '\u{1dc4}',
    '\u{1dc5}',
    '\u{1dc6}',
    '\u{1dc7}',
    '\u{1dc8}',
    '\u{1dc9}',
    '\u{1dcb}',
    '\u{1dcc}',
    '\u{1dd1}',
    '\u{1dd2}',
    '\u{1dd3}',
    '\u{1dd4}',
    '\u{1dd5}',
    '\u{1dd6}',
    '\u{1dd7}',
    '\u{1dd8}',
    '\u{1dd9}',
    '\u{1dda}',
    '\u{1ddb}',
    '\u{1ddc}',
    '\u{1ddd}',
    '\u{1dde}',
    '\u{1ddf}',
    '\u{1de0}',
    '\u{1de1}',
    '\u{1de2}',
    '\u{1de3}',
    '\u{1de4}',
    '\u{1de5}',
    '\u{1de6}',
    '\u{1dfe}',
    '\u{20d0}',
    '\u{20d1}',
    '\u{20d4}',
    '\u{20d5}',
    '\u{20d6}',
    '\u{20d7}',
    '\u{20db}',
    '\u{20dc}',
    '\u{20e1}',
    '\u{20e7}',
    '\u{20e9}',
    '\u{20f0}',
    '\u{2cef}',
    '\u{2cf0}',
    '\u{2cf1}',
    '\u{2de0}',
    '\u{2de1}',
    '\u{2de2}',
    '\u{2de3}',
    '\u{2de4}',
    '\u{2de5}',
    '\u{2de6}',
    '\u{2de7}',
    '\u{2de8}',
    '\u{2de9}',
    '\u{2dea}',
    '\u{2deb}',
    '\u{2dec}',
    '\u{2ded}',
    '\u{2dee}',
    '\u{2def}',
    '\u{2df0}',
    '\u{2df1}',
    '\u{2df2}',
    '\u{2df3}',
    '\u{2df4}',
    '\u{2df5}',
    '\u{2df6}',
    '\u{2df7}',
    '\u{2df8}',
    '\u{2df9}',
    '\u{2dfa}',
    '\u{2dfb}',
    '\u{2dfc}',
    '\u{2dfd}',
    '\u{2dfe}',
    '\u{2dff}',
    '\u{a66f}',
    '\u{a67c}',
    '\u{a67d}',
    '\u{a6f0}',
    '\u{a6f1}',
    '\u{a8e0}',
    '\u{a8e1}',
    '\u{a8e2}',
    '\u{a8e3}',
    '\u{a8e4}',
    '\u{a8e5}',
    '\u{a8e6}',
    '\u{a8e7}',
    '\u{a8e8}',
    '\u{a8e9}',
    '\u{a8ea}',
    '\u{a8eb}',
    '\u{a8ec}',
    '\u{a8ed}',
    '\u{a8ee}',
    '\u{a8ef}',
    '\u{a8f0}',
    '\u{a8f1}',
    '\u{aab0}',
    '\u{aab2}',
    '\u{aab3}',
    '\u{aab7}',
    '\u{aab8}',
    '\u{aabe}',
    '\u{aabf}',
    '\u{aac1}',
    '\u{fe20}',
    '\u{fe21}',
    '\u{fe22}',
    '\u{fe23}',
    '\u{fe24}',
    '\u{fe25}',
    '\u{fe26}',
    '\u{10a0f}',
    '\u{10a38}',
    '\u{1d185}',
    '\u{1d186}',
    '\u{1d187}',
    '\u{1d188}',
    '\u{1d189}',
    '\u{1d1aa}',
    '\u{1d1ab}',
    '\u{1d1ac}',
    '\u{1d1ad}',
    '\u{1d242}',
    '\u{1d243}',
    '\u{1d244}',
];

/// Диакритика для числа; индекс списка = число.
fn diacritic(number: u32) -> Option<char> {
    DIACRITICS.get(usize::try_from(number).ok()?).copied()
}

/// Поддерживается ли kitty по окружению. Активные запросы не делаются.
pub fn detected() -> bool {
    detect(std::env::var("TERM").ok().as_deref(), std::env::var_os("KITTY_WINDOW_ID").is_some())
}

/// Чистая проверка признаков: `TERM` содержит kitty или задан `KITTY_WINDOW_ID`.
fn detect(term: Option<&str>, kitty_window: bool) -> bool {
    kitty_window || term.is_some_and(|term| term.contains("kitty"))
}

/// Стабильный ненулевой 24-битный id картинки: из книги и имени ресурса.
pub fn image_id(book: &str, resource: &str) -> u32 {
    let mut key = String::with_capacity(book.len() + resource.len() + 1);
    key.push_str(book);
    key.push('|');
    key.push_str(resource);
    (u32::try_from(fnv1a(key.as_bytes()) & 0xff_ff_ff).unwrap_or(1)).max(1)
}

fn fnv1a(data: &[u8]) -> u64 {
    let mut state = 0xcbf2_9ce4_8422_2325;
    for byte in data {
        state ^= u64::from(*byte);
        state = state.wrapping_mul(0x0000_0100_0000_01b3);
    }
    state
}

/// Передача картинки: PNG payload chunked по 4096, первый кусок с заголовком.
pub fn transmit(id: u32, png: &[u8], cols: u32, rows: u32) -> String {
    let encoded = base64(png);
    let mut out = String::with_capacity(encoded.len() + 256);
    let mut offset = 0;
    let mut first = true;
    loop {
        let end = (offset + 4096).min(encoded.len());
        if first {
            out.push_str("\x1b_Gf=100,i=");
            out.push_str(&id.to_string());
            out.push_str(",U=1,c=");
            out.push_str(&cols.to_string());
            out.push_str(",r=");
            out.push_str(&rows.to_string());
            out.push_str(",q=2,m=1;");
        } else {
            out.push_str("\x1b_G");
            out.push_str(if end < encoded.len() { "m=1;" } else { "m=0;" });
        }
        if end == offset {
            out.push_str("\x1b\\");
            break;
        }
        out.push_str(&encoded[offset..end]);
        out.push_str("\x1b\\");
        first = false;
        if end == encoded.len() {
            break;
        }
        offset = end;
    }
    out
}

/// Удаление картинки и её виртуального placement.
pub fn delete(id: u32) -> String {
    format!("\x1b_Ga=d,d=i,i={id},q=2\x1b\\")
}

/// Символ ячейки-плейсхолдера: `U+10EEEE` + диакритики строки и колонки.
pub fn placeholder(row: u32, col: u32) -> Option<String> {
    let row_char = diacritic(row)?;
    let col_char = diacritic(col)?;
    Some(format!("{PLACEHOLDER}{row_char}{col_char}"))
}

/// Цвет переднего плана плейсхолдера: id в 24-битном RGB.
pub fn id_color(id: u32) -> (u8, u8, u8) {
    ((id >> 16) as u8, ((id >> 8) & 0xff) as u8, (id & 0xff) as u8)
}

/// Base64 без зависимостей: стандартный алфавит с паддингом.
fn base64(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = u32::from(chunk[0]);
        let b1 = chunk.get(1).map_or(0, |b| u32::from(*b));
        let b2 = chunk.get(2).map_or(0, |b| u32::from(*b));
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[(triple >> 18) as usize & 63] as char);
        out.push(ALPHABET[(triple >> 12) as usize & 63] as char);
        out.push(match chunk.len() {
            1 => '=',
            _ => ALPHABET[(triple >> 6) as usize & 63] as char,
        });
        out.push(match chunk.len() {
            3 => ALPHABET[triple as usize & 63] as char,
            _ => '=',
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detected_by_kitty_environment() {
        assert!(detect(Some("xterm-kitty"), false));
        assert!(detect(Some("kitty-direct"), false));
        assert!(detect(Some("xterm-256color"), true));
        assert!(!detect(Some("xterm-256color"), false));
        assert!(!detect(None, false));
    }

    #[test]
    fn image_ids_are_stable_nonzero_and_distinct() {
        let first = image_id("/books/a.epub", "file5.svg");
        assert_eq!(first, image_id("/books/a.epub", "file5.svg"));
        assert_ne!(first, 0);
        assert_ne!(first, image_id("/books/b.epub", "file5.svg"));
        assert_ne!(first, image_id("/books/a.epub", "file6.svg"));
        assert!(first <= 0xff_ff_ff);
    }

    #[test]
    fn placeholders_match_kitty_encoding() {
        // Из документации kitty: id 42, ячейка (0, 0) → U+0305 + U+0305,
        // (0, 1) → U+0305 + U+030D; (1, 1) → U+030D + U+030D.
        assert_eq!(placeholder(0, 0).as_deref(), Some("\u{10EEEE}\u{0305}\u{0305}"));
        assert_eq!(placeholder(0, 1).as_deref(), Some("\u{10EEEE}\u{0305}\u{030d}"));
        assert_eq!(placeholder(1, 1).as_deref(), Some("\u{10EEEE}\u{030d}\u{030d}"));
        // Число вне таблицы — без плейсхолдера.
        assert!(placeholder(297, 0).is_none());
        assert!(placeholder(0, 300).is_none());
    }

    #[test]
    fn id_color_packs_24_bits() {
        assert_eq!(id_color(0x42_7f_00), (0x42, 0x7f, 0x00));
        assert_eq!(id_color(1), (0, 0, 1));
        assert_eq!(id_color(0xff_ff_ff), (0xff, 0xff, 0xff));
    }

    #[test]
    fn transmit_is_chunked_png_with_virtual_placement() {
        let payload = "hello world".as_bytes();
        let escape = transmit(7, payload, 40, 9);
        assert!(escape.starts_with("\x1b_Gf=100,i=7,U=1,c=40,r=9,q=2,m=1;"), "{escape:?}");
        assert!(escape.ends_with("\x1b\\"));
        // Маленький payload — один кусок, никакого m в закрывающем.
        assert_eq!(escape.matches("\x1b_G").count(), 1);
        assert!(!escape.contains("m=0;"));
        // base64(payload) == aGVsbG8gd29ybGQ=
        assert!(escape.contains("aGVsbG8gd29ybGQ="));
    }

    #[test]
    fn transmit_splits_large_payloads() {
        let payload: Vec<u8> = (0..4096 * 2 + 11).map(|i| (i % 251) as u8).collect();
        let escape = transmit(3, &payload, 10, 2);
        let chunks: Vec<&str> = escape.split("\x1b\\").filter(|c| !c.is_empty()).collect();
        assert_eq!(chunks.len(), 3, "{chunks:?}");
        assert!(chunks[0].contains("f=100,i=3,U=1,c=10,r=2,q=2,m=1;"));
        assert!(chunks[1].starts_with("\x1b_Gm=1;"));
        assert!(chunks[2].starts_with("\x1b_Gm=0;"));
        // Собираем payload назад и сравниваем.
        let mut joined = String::new();
        for chunk in &chunks {
            let body = chunk.split(';').nth(1).expect("payload");
            joined.push_str(body);
        }
        assert_eq!(base64_decode(&joined), payload, "payload собран заново");
    }

    #[test]
    fn delete_targets_the_image_id() {
        assert_eq!(delete(42), "\x1b_Ga=d,d=i,i=42,q=2\x1b\\");
    }

    fn base64_decode(text: &str) -> Vec<u8> {
        const ALPHABET: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = Vec::new();
        let mut buffer = 0u32;
        let mut bits = 0u32;
        for c in text.bytes() {
            if c == b'=' {
                break;
            }
            let value = ALPHABET.iter().position(|a| *a == c).expect("символ") as u32;
            buffer = (buffer << 6) | value;
            bits += 6;
            if bits >= 8 {
                bits -= 8;
                out.push(((buffer >> bits) & 0xff) as u8);
            }
        }
        out
    }
}
