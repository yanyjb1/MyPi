//! The classes a zone declares. `local` is not one: it is delivered unconditionally.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Call,
    Post,
    Subscribe,
    Stream,
    Notice,
}

impl Kind {
    /// The class of one frame; `None` = nothing a zone may declare.
    pub fn of(kind: &str, name: &str) -> Option<Kind> {
        Some(match kind {
            "call" => Kind::Call,
            "post" => Kind::Post,
            "subscribe" => Kind::Subscribe,
            "stream" if name == "notice" => Kind::Notice,
            "stream" => Kind::Stream,
            _ => return None,
        })
    }

    /// The wire's kind string, for frames this front end sends.
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Call => "call",
            Kind::Post => "post",
            Kind::Subscribe => "subscribe",
            Kind::Stream => "stream",
            Kind::Notice => "stream",
        }
    }
}
