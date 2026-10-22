//! The command contract: what a slash command declares, and the one table the server and every front
//! end read (rationale in `memo.md`).

/// Where a completion candidate for one argument position comes from: the
/// table names the kind, a provider hands the candidates out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arg {
    /// A filesystem path.
    Path,
    /// A model, addressed as `<provider>:<id>`.
    Model,
    /// A profile name.
    Profile,
    /// A number (a token budget, a block count, a session id). No candidates.
    Number,
    /// Free text. No candidates.
    Text,
}

/// Who runs this command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// The server: session state (naming, directory, model, profile, …).
    Server,
    /// The surface itself (`/q`, `/resume`, a theme picker).
    Local,
}

/// One command's declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandMeta {
    /// The name with its leading slash, e.g. `/name`.
    pub name: &'static str,
    /// Alternate spellings of the same command.
    pub aliases: &'static [&'static str],
    /// This command's argument positions, in order. An empty list means the
    /// command takes no arguments.
    pub args: &'static [Arg],
    pub scope: Scope,
    /// One line for help and completion.
    pub detail: &'static str,
}

impl CommandMeta {
    /// The argument position `level` (0-based). `None` = past the last
    /// position: that level has no candidates at all.
    pub fn arg(&self, level: usize) -> Option<Arg> {
        self.args.get(level).copied()
    }
}

/// One completion candidate. `more` makes the tree walkable: a candidate whose
/// text is complete and whose `more` is true has another level under it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Candidate {
    /// The text this candidate inserts for **this** level (never a whole line).
    pub value: String,
    /// What the popup shows next to the value.
    pub detail: String,
    /// There is another level under this candidate.
    pub more: bool,
}

/// Does this name start with `/`? (`str::starts_with` is not const.)
const fn slash(name: &str) -> bool {
    let b = name.as_bytes();
    !b.is_empty() && b[0] == b'/'
}

/// Byte equality (`str::eq` is not const, and `&str` has no const `==`).
const fn eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

/// Declare commands: one line each, the field order is fixed.
macro_rules! commands {
    ($(
        $ty:ident {
            name: $name:expr,
            aliases: [$($alias:expr),* $(,)?],
            args: [$($arg:ident),* $(,)?],
            scope: $scope:ident,
            detail: $detail:expr $(,)?
        }
    ),* $(,)?) => {
        $(
            /// A command's marker type: its declaration is in `META`, its
            /// implementation is an inherent impl in the owning domain.
            #[derive(Debug, Clone, Copy, PartialEq, Eq)]
            pub struct $ty;

            impl $ty {
                /// This command's declaration.
                pub const META: CommandMeta = CommandMeta {
                    name: $name,
                    aliases: &[$($alias),*],
                    args: &[$(Arg::$arg),*],
                    scope: Scope::$scope,
                    detail: $detail,
                };
            }
        )*

        /// Every command there is, in declaration order.
        pub const COMMANDS: &[CommandMeta] = &[$($ty::META),*];

        // The rules are checked while the table is built: a violation fails here.
        const _: () = {
            let mut i = 0;
            while i < COMMANDS.len() {
                let c = COMMANDS[i];
                if !slash(c.name) {
                    panic!("a command name must start with '/'");
                }
                let mut m = 0;
                while m < i {
                    if eq(c.name, COMMANDS[m].name) {
                        panic!("two commands must not share a name");
                    }
                    m += 1;
                }
                if c.detail.is_empty() {
                    panic!("a command detail must not be empty");
                }
                let mut k = 0;
                while k < c.args.len() {
                    if k + 1 < c.args.len() && matches!(c.args[k], Arg::Text) {
                        panic!("a free-text argument must be the last position");
                    }
                    k += 1;
                }
                let mut j = 0;
                while j < c.aliases.len() {
                    let a = c.aliases[j];
                    if !slash(a) {
                        panic!("an alias must start with '/'");
                    }
                    let mut m = 0;
                    while m < COMMANDS.len() {
                        if eq(a, COMMANDS[m].name) {
                            panic!("an alias must not collide with a command name");
                        }
                        let mut n = 0;
                        while n < COMMANDS[m].aliases.len() {
                            // Only the aliases before this one: comparing an
                            // alias with itself would always fire.
                            if (m < i || (m == i && n < j)) && eq(a, COMMANDS[m].aliases[n]) {
                                panic!("two aliases must not collide");
                            }
                            n += 1;
                        }
                        m += 1;
                    }
                    j += 1;
                }
                i += 1;
            }
        };
    };
}

commands! {
    Name {
        name: "/name",
        aliases: [],
        args: [Text],
        scope: Server,
        detail: "给会话起名（分支继承，兄弟分支看不到）",
    },
    Cdp {
        name: "/cdp",
        aliases: ["/cd"],
        args: [Path],
        scope: Server,
        detail: "切换工作目录（持久化，后续相对路径以它为准）",
    },
    Compact {
        name: "/compact",
        aliases: [],
        args: [Number, Text],
        scope: Server,
        detail: "把历史压缩成检查点：`/compact 20k` 保留约 20k token 的尾巴，`/compact 18` 保留最新 18 个块，后面还能跟一句焦点",
    },
    Switch {
        name: "/switch",
        aliases: [],
        args: [Model],
        scope: Server,
        detail: "切换本会话的模型（落会话状态）",
    },
    Model {
        name: "/model",
        aliases: [],
        args: [Model],
        scope: Server,
        detail: "换模型并写为默认（改配置文件，下次启动生效）",
    },
    Profile {
        name: "/profile",
        aliases: [],
        args: [Profile],
        scope: Server,
        detail: "切换系统提示词 profile",
    },
    Usage {
        name: "/usage",
        aliases: [],
        args: [Text],
        scope: Server,
        detail: "看本会话的用量与账单（分模型；lang:zh 用万/亿）",
    },
    Delete {
        name: "/delete",
        aliases: ["/rm"],
        args: [Number],
        scope: Server,
        detail: "删掉一个会话（参数是会话号，一次一个；连带只被它引用的回合、块与巨物）",
    },
    Resume {
        name: "/resume",
        aliases: [],
        args: [],
        scope: Local,
        detail: "恢复本项目的某个会话",
    },
    New {
        name: "/new",
        aliases: [],
        args: [],
        scope: Local,
        detail: "开一个新会话",
    },
    Quit {
        name: "/q",
        aliases: ["/quit", "/exit"],
        args: [],
        scope: Local,
        detail: "退出",
    },
}

/// Look a command up by name or alias.
pub fn lookup(name: &str) -> Option<&'static CommandMeta> {
    COMMANDS
        .iter()
        .find(|c| c.name == name || c.aliases.contains(&name))
}

/// Is this input a command the table knows? An unrecognized `/xxx` is the
/// caller's to report.
pub fn split(input: &str) -> Option<(&'static CommandMeta, &str)> {
    let input = input.trim();
    let rest = input.strip_prefix('/')?;
    if rest.is_empty() {
        return None;
    }
    let (name, args) = match rest.split_once(char::is_whitespace) {
        Some((n, a)) => (format!("/{n}"), a.trim()),
        None => (format!("/{rest}"), ""),
    };
    // The name has to be a whole word: `/nameless` is not `/name`.
    lookup(&name).map(|c| (c, args))
}
