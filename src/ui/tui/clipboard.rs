//! OSC 52: text to the terminal's clipboard, no dependency and no subprocess.

/// One escape sequence carrying `text` as the system clipboard.
pub(crate) fn osc52(text: &str) -> String {
    format!("\x1b]52;c;{}\x1b\\", base64(text.as_bytes()))
}

/// Standard base64, padded.
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b1 = chunk[0];
        let b2 = *chunk.get(1).unwrap_or(&0);
        let b3 = *chunk.get(2).unwrap_or(&0);
        out.push(ALPHABET[(b1 >> 2) as usize] as char);
        out.push(ALPHABET[(((b1 & 0x03) << 4) | (b2 >> 4)) as usize] as char);
        out.push(match chunk.len() > 1 {
            true => ALPHABET[(((b2 & 0x0f) << 2) | (b3 >> 6)) as usize] as char,
            false => '=',
        });
        out.push(match chunk.len() > 2 {
            true => ALPHABET[(b3 & 0x3f) as usize] as char,
            false => '=',
        });
    }
    out
}
