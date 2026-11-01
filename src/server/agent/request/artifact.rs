//! Oversized tool output: its citation, and the line the model reads.

use anyhow::Result;

use super::contract::{History, Reference};

/// The prefix lengths a citation may take, shortest first.
const LENGTHS: [usize; 7] = [4, 6, 8, 10, 12, 14, 16];

/// FNV-1a 64 as lowercase hex: sixteen digits, always.
pub fn hash(content: &str) -> String {
    let mut value: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in content.as_bytes() {
        value ^= u64::from(*byte);
        value = value.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{value:016x}")
}

/// The citation for one content: the same text always gets the same one, a taken citation lengthens.
pub fn resolve(
    content: &str,
    known: &mut Vec<Reference>,
    history: &dyn History,
) -> Result<(String, String)> {
    let hash = hash(content);
    if let Some(existing) = known.iter().find(|r| r.hash == hash) {
        return Ok((hash, existing.reference.clone()));
    }
    for length in LENGTHS {
        let candidate = hash[..length].to_string();
        let taken = match known.iter().find(|r| r.reference == candidate) {
            Some(existing) => existing.hash != hash,
            None => history.holder(&candidate)?.is_some_and(|held| held != hash),
        };
        if !taken {
            known.push(Reference {
                hash: hash.clone(),
                reference: candidate.clone(),
            });
            return Ok((hash, candidate));
        }
    }
    Ok((hash.clone(), hash))
}

/// The line appended to the visible part, so the model can fetch the rest.
pub fn citation(reference: &str, content: &str) -> String {
    format!(
        "\n\n[truncated: {} lines, {} bytes; full text is #{reference} — name #{reference} in a bash command to read it]",
        content.lines().count(),
        content.len()
    )
}
