//! Копирование в системный буфер через escape-последовательность OSC 52:
//! работает в kitty, alacritty, wezterm, foot, tmux и большинстве терминалов.
pub fn osc52(text: &str) -> String {
    format!("\x1b]52;c;{}\x07", base64(text.as_bytes()))
}

/// Классический base64 без зависимостей.
pub fn base64(data: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let hi = u32::from(chunk[0]) << 16
            | u32::from(chunk.get(1).copied().unwrap_or(0)) << 8
            | u32::from(chunk.get(2).copied().unwrap_or(0));
        out.push(TABLE[(hi >> 18 & 63) as usize] as char);
        out.push(TABLE[(hi >> 12 & 63) as usize] as char);
        if chunk.len() > 1 {
            out.push(TABLE[(hi >> 6 & 63) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(TABLE[(hi & 63) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}
