// Image sniffing: the format comes from the bytes, never from the file's name.

use std::io::Read as _;
use std::path::Path;

use anyhow::{Context as _, Result};

// The format these bytes open with, as the data-URL subtype the wire takes.
pub(crate) fn known(head: &[u8]) -> Option<&'static str> {
    let at_least = |n: usize| head.len() >= n;
    if at_least(8) && head[..8] == [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A] {
        Some("png")
    } else if at_least(3) && head[..3] == [0xFF, 0xD8, 0xFF] {
        Some("jpeg")
    } else if at_least(6) && (&head[..6] == b"GIF87a" || &head[..6] == b"GIF89a") {
        Some("gif")
    } else if at_least(12) && &head[..4] == b"RIFF" && &head[8..12] == b"WEBP" {
        Some("webp")
    } else {
        None
    }
}

// The format a file's head names.
pub(crate) fn detect(path: &Path) -> Result<Option<&'static str>> {
    let mut head = [0u8; 12];
    let mut file = std::fs::File::open(path).context("failed to open file")?;
    let read = file.read(&mut head).context("failed to read file head")?;
    Ok(known(&head[..read]))
}
