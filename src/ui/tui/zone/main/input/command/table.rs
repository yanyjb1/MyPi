// The table: every command's declaration, its own dispatch, and the two lookups.

use super::meta::{self, CommandMeta};
use super::{
    cdp, compact, effort, hub, memo, model, name, new, prepare, profile, quit, resume, switch,
    usage,
};

pub const TABLE: &[CommandMeta] = &[
    name::META,
    cdp::META,
    compact::META,
    switch::META,
    effort::META,
    model::META,
    profile::META,
    usage::META,
    memo::META,
    prepare::META,
    resume::META,
    hub::META,
    new::META,
    quit::META,
];

const _: () = assert!(meta::valid(TABLE));

// One line of input: the command it names, and its argument text.
pub fn split(input: &str) -> Option<(&'static CommandMeta, &str)> {
    meta::split(TABLE, input)
}

// One name (with its slash) to its declaration.
pub fn lookup(name: &str) -> Option<&'static CommandMeta> {
    meta::lookup(TABLE, name)
}

// `provider:model`, both halves non-empty.
pub fn pair(args: &str) -> Option<(String, String)> {
    let (provider, model) = args.split_once(':')?;
    let (provider, model) = (provider.trim(), model.trim());
    if provider.is_empty() || model.is_empty() {
        return None;
    }
    Some((provider.to_string(), model.to_string()))
}
