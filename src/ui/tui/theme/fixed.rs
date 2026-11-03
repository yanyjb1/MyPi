//! The fixed palette: every color that is not the accent.
//!
//! Values come from the old bundled `titanium` theme, so nothing on screen moves.

use ratatui::style::Color;

const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::Rgb(r, g, b)
}

// Prose
pub const BODY: Color = Color::Reset;
pub const THINKING: Color = rgb(0x9c, 0xa3, 0xb0);
pub const SYSTEM: Color = rgb(0x9c, 0xa3, 0xb0);
pub const ERROR_TEXT: Color = rgb(0xff, 0x47, 0x57);

// Generic marks
pub const OK: Color = rgb(0x00, 0xff, 0x88);
pub const ERR: Color = rgb(0xff, 0x47, 0x57);
pub const WARN: Color = rgb(0xff, 0xb3, 0x47);
pub const DIM: Color = rgb(0x6b, 0x72, 0x80);
pub const MUTED: Color = rgb(0x9c, 0xa3, 0xb0);

// Cards
pub const USER_BG: Color = rgb(0x0f, 0x12, 0x16);
pub const USER_TEXT: Color = Color::Reset;
pub const TOOL_TITLE: Color = Color::Reset;
pub const TOOL_OUTPUT: Color = rgb(0x9c, 0xa3, 0xb0);
pub const TOOL_BG_PENDING: Color = rgb(0x0f, 0x12, 0x16);
pub const TOOL_BG_OK: Color = rgb(0x0f, 0x12, 0x16);
pub const TOOL_BG_ERR: Color = rgb(0x1a, 0x0f, 0x10);
pub const TOOL_EDGE_OK: Color = rgb(0x6b, 0x72, 0x80);
pub const TOOL_EDGE_ERR: Color = rgb(0xff, 0x47, 0x57);

// Markdown
pub const MD_HEADING: Color = rgb(0x00, 0xb4, 0xff);
pub const MD_CODE: Color = rgb(0x00, 0xff, 0x88);
pub const MD_CODE_BLOCK_BORDER: Color = rgb(0x2a, 0x30, 0x38);
pub const MD_TABLE_BORDER: Color = Color::White;
pub const MD_QUOTE: Color = rgb(0x9c, 0xa3, 0xb0);
pub const MD_QUOTE_BORDER: Color = rgb(0x2a, 0x30, 0x38);
pub const MD_HR: Color = rgb(0x2a, 0x30, 0x38);
pub const MD_LIST_BULLET: Color = rgb(0x00, 0xb4, 0xff);

// Diffs
pub const DIFF_ADDED: Color = rgb(0x00, 0xff, 0x88);
pub const DIFF_REMOVED: Color = rgb(0xff, 0x47, 0x57);

// Syntax
pub const SYN_COMMENT: Color = rgb(0x6b, 0x72, 0x80);
pub const SYN_KEYWORD: Color = rgb(0x00, 0xb4, 0xff);
pub const SYN_FUNCTION: Color = rgb(0x00, 0xff, 0x88);
pub const SYN_VARIABLE: Color = rgb(0xe8, 0xec, 0xf4);
pub const SYN_STRING: Color = rgb(0xd4, 0xc0, 0x90);
pub const SYN_NUMBER: Color = rgb(0xff, 0xb3, 0x47);
pub const SYN_TYPE: Color = rgb(0x00, 0xb4, 0xff);
pub const SYN_OPERATOR: Color = rgb(0x00, 0xb4, 0xff);
pub const SYN_PUNCTUATION: Color = rgb(0x9c, 0xa3, 0xb0);

// Tree
pub const TREE_ASSISTANT: Color = rgb(0x00, 0xb4, 0xff);
pub const TREE_TOOL_CALL: Color = rgb(0xff, 0xb3, 0x47);
pub const TREE_TOOL_RESULT: Color = rgb(0xd4, 0xc0, 0x90);
pub const TREE_REASONING: Color = rgb(0x9c, 0xa3, 0xb0);
pub const TREE_AXIS: Color = rgb(0x9c, 0xa3, 0xb0);

// Statusline
pub const STATUS_SEP: Color = rgb(0x2a, 0x30, 0x38);
pub const STATUS_PATH: Color = rgb(0xff, 0xff, 0xff);
pub const STATUS_GIT_CLEAN: Color = rgb(0x00, 0xff, 0x88);
pub const STATUS_GIT_DIRTY: Color = rgb(0xff, 0xb3, 0x47);
pub const STATUS_COST: Color = rgb(0xd4, 0xc0, 0x90);
