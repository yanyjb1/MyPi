//! The declarative key macro.

use super::types::{Key, Kind};

macro_rules! keys {
    ($( $konst:ident = $head:literal, $path:literal, $name:literal, $kind:ident, $default:expr, $comment:literal, $queryable:literal; )*) => {
        $( pub const $konst: &str = concat!($head, ".", $path, ".", $name); )*
        pub static KEYS: &[Key] = &[ $( Key {
            full: concat!($head, ".", $path, ".", $name),
            head: $head,
            path: $path,
            name: $name,
            kind: Kind::$kind,
            default: $default,
            comment: $comment,
            queryable: $queryable,
        } ),* ];
    };
}

pub(crate) use keys;
