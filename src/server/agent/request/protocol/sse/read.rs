//! One raw read: the bytes up to the next newline, timeout included.

/// Read one raw line into `buf`, replacing what it held.
pub fn read(reader: &mut dyn std::io::BufRead, buf: &mut Vec<u8>) -> std::io::Result<usize> {
    buf.clear();
    reader.read_until(b'\n', buf)
}
