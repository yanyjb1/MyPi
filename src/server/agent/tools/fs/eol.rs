// The line ending a text is written with: the one it already had, else LF.

// What a file's lines end in. A file that carries any CRLF is written back as all CRLF.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Eol {
    Lf,
    Crlf,
}

impl Eol {
    // The ending this text is written with.
    pub(crate) fn of(text: &str) -> Self {
        match text.contains("\r\n") {
            true => Self::Crlf,
            false => Self::Lf,
        }
    }

    // The text with every line ending read as a plain newline: what the tools edit and the languages parse.
    pub(crate) fn plain(self, text: &str) -> std::borrow::Cow<'_, str> {
        if !text.contains('\r') {
            return std::borrow::Cow::Borrowed(text);
        }
        let mut out = String::with_capacity(text.len());
        let mut rest = text;
        while let Some(at) = rest.find('\r') {
            out.push_str(&rest[..at]);
            out.push('\n');
            rest = rest[at + 1..].strip_prefix('\n').unwrap_or(&rest[at + 1..]);
        }
        out.push_str(rest);
        std::borrow::Cow::Owned(out)
    }

    // The text with every line ending written as this one. A text already in another ending is read as plain first.
    pub(crate) fn write(self, text: &str) -> String {
        let plain = self.plain(text);
        match self {
            Self::Lf => plain.into_owned(),
            Self::Crlf => {
                let mut out = String::with_capacity(plain.len() + plain.len() / 16);
                for (at, line) in plain.split('\n').enumerate() {
                    if at > 0 {
                        out.push_str("\r\n");
                    }
                    out.push_str(line);
                }
                out
            }
        }
    }
}
