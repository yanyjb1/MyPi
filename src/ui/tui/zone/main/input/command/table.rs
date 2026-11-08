//! The table: every command file's declaration, the dispatch, and the two lookups.

use crate::ui::command::{self, CommandMeta};
use crate::ui::session::ModelId;

use super::{
    Call, Done, cdp, compact, delete, memo, model, name, new, profile, quit, resume, switch, usage,
};

pub const TABLE: &[CommandMeta] = &[
    name::META,
    cdp::META,
    compact::META,
    switch::META,
    model::META,
    profile::META,
    usage::META,
    memo::META,
    delete::META,
    resume::META,
    new::META,
    quit::META,
];

const _: () = assert!(command::valid(TABLE));

/// One line of input: the command it names, and its argument text.
pub fn split(input: &str) -> Option<(&'static CommandMeta, &str)> {
    command::split(TABLE, input)
}

/// One name (with its slash) to its declaration.
pub fn lookup(name: &str) -> Option<&'static CommandMeta> {
    command::lookup(TABLE, name)
}

/// Run one command.
pub fn run(meta: &CommandMeta, call: &Call<'_>) -> Done {
    match meta.name {
        "/name" => name::run(call),
        "/cdp" => cdp::run(call),
        "/compact" => compact::run(call),
        "/switch" => switch::run(call),
        "/model" => model::run(call),
        "/profile" => profile::run(call),
        "/usage" => usage::run(call),
        "/memo" => memo::run(call),
        "/delete" => delete::run(call),
        "/resume" => resume::run(call),
        "/new" => new::run(call),
        "/q" => quit::run(call),
        _ => Done::Hold,
    }
}

/// `provider:model`.
pub fn pair(args: &str) -> Option<ModelId> {
    let (provider, model) = args.split_once(':')?;
    let (provider, model) = (provider.trim(), model.trim());
    if provider.is_empty() || model.is_empty() {
        return None;
    }
    Some(ModelId {
        provider: provider.to_string(),
        model: model.to_string(),
    })
}
