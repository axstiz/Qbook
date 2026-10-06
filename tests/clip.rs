use qbook::clip::{base64, osc52};

#[test]
fn base64_encodes_classic_examples() {
    assert_eq!(base64(b""), "");
    assert_eq!(base64(b"f"), "Zg==");
    assert_eq!(base64(b"fo"), "Zm8=");
    assert_eq!(base64(b"foo"), "Zm9v");
    assert_eq!(base64(b"foob"), "Zm9vYg==");
    assert_eq!(base64(b"fooba"), "Zm9vYmE=");
    assert_eq!(base64(b"foobar"), "Zm9vYmFy");
}

#[test]
fn base64_handles_utf8() {
    let decoded = rfc_decode(&base64("привет".as_bytes()));
    assert_eq!(String::from_utf8(decoded).expect("utf8"), "привет");
}

/// Референс-декодер для проверки энкодера.
fn rfc_decode(text: &str) -> Vec<u8> {
    let table = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let value = |c: char| table.iter().position(|&t| t as char == c).expect("символ base64");
    let mut out = Vec::new();
    let mut acc = 0u32;
    let mut bits = 0u32;
    for byte in text.bytes() {
        if byte == b'=' {
            break;
        }
        acc = (acc << 6) | value(byte as char) as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((acc >> bits) & 0xff) as u8);
        }
    }
    out
}

#[test]
fn osc52_wraps_text_for_the_terminal() {
    assert_eq!(osc52("abc"), "\x1b]52;c;YWJj\x07");
}
