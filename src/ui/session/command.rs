//! A slash command's arguments → the call that carries them out. Design: `README.md`.

use crate::commands::CommandMeta;
use crate::server::api::subscribe::Retention;
use crate::server::api::types::{ModelId, UsageLang};

use super::{Level, Life, Session};

impl Session {
    /// Carry out one server-side command. A bad argument is reported, never guessed at.
    pub fn command(&mut self, spec: &'static CommandMeta, args: &str) {
        let args = args.trim();
        match spec.name {
            "/name" => self.rename((!args.is_empty()).then(|| args.to_string())),
            "/cdp" => {
                if args.is_empty() {
                    self.refused(spec, "缺一个目录");
                } else {
                    self.set_cwd(args.to_string());
                }
            }
            "/compact" => {
                let (number, focus) = match args.split_once(char::is_whitespace) {
                    Some((n, f)) => (n, Some(f.trim().to_string())),
                    None => (args, None),
                };
                match retention(number) {
                    Some(retention) => self.compact(retention, focus),
                    None => self.refused(spec, "保留量认不出（`20k` 是词元，`18` 是块）"),
                }
            }
            "/switch" => match model_id(args) {
                Some(id) => self.switch_model(id),
                None => self.refused(spec, "模型要写成 `provider:id`"),
            },
            "/model" => match model_id(args) {
                Some(id) => self.set_default_model(id),
                None => self.refused(spec, "模型要写成 `provider:id`"),
            },
            "/profile" => {
                if args.is_empty() {
                    self.refused(spec, "缺一个 profile 名");
                } else {
                    self.set_profile(args.to_string());
                }
            }
            "/usage" => self.usage(if args.contains("zh") {
                UsageLang::Zh
            } else {
                UsageLang::En
            }),
            "/delete" => match args.parse::<i64>() {
                Ok(id) => self.delete(id),
                Err(_) => self.refused(spec, "参数是会话号"),
            },
            // A server-side command this build has no call for: say so, do not pretend.
            other => self.refused(spec, &format!("{other} 还没接线")),
        }
    }

    fn refused(&mut self, spec: &CommandMeta, why: &str) {
        self.notice(format!("{}：{why}", spec.name), Level::Info, Life::Short);
    }
}

/// `/compact`'s number: `20k` tokens, `18` blocks, nothing = the configured tail.
fn retention(arg: &str) -> Option<Retention> {
    let arg = arg.trim();
    if arg.is_empty() {
        return Some(Retention::Default);
    }
    let (digits, scale) = match arg.as_bytes().last() {
        Some(b'k' | b'K') => (&arg[..arg.len() - 1], 1_000),
        Some(b'm' | b'M') => (&arg[..arg.len() - 1], 1_000_000),
        _ => (arg, 1),
    };
    let n: usize = digits.parse().ok()?;
    Some(if scale == 1 {
        Retention::Blocks(n)
    } else {
        Retention::Tokens(n * scale)
    })
}

/// A model as the config addresses it: `<provider>:<id>`.
fn model_id(arg: &str) -> Option<ModelId> {
    let (provider, model) = arg.trim().split_once(':')?;
    if provider.is_empty() || model.is_empty() {
        return None;
    }
    Some(ModelId {
        provider: provider.to_string(),
        model: model.to_string(),
    })
}
