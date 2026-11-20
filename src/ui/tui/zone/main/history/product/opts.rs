// What painting needs besides the block itself.

use crate::ui::tui::lang::Lang;

pub struct Opts<'a> {
    pub thinking: bool,
    pub tool: bool,
    pub placeholder: &'a str,
    pub cwd: &'a str,
    pub fold: usize,
    pub lang: Lang,
    pub shell: Option<&'static str>,
}
