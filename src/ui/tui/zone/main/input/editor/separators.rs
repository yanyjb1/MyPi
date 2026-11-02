//! 分隔符定义表 —— 编辑器"块"的唯一定义源。
//!
//! 一个集合：块与块的分界。词删除（Ctrl+W）与词滑动（Ctrl+方向）在这里停下，
//! 补全触发词的回扫也停在这里。`src/main.rs`、`a-b_c.tar.gz` 因此是一整块。
//!
//! 默认表内置；config 重构后通过覆盖表注入。

use std::collections::HashSet;

/// 块与块的分界。编辑器与补全触发共用这一个定义。
#[derive(Debug, Clone)]
pub struct SeparatorTable {
    hard: HashSet<char>,
}

impl Default for SeparatorTable {
    fn default() -> Self {
        // 硬边界：空白 + CJK 标点 + 拉丁标点（连接符除外）。
        // 用户决策：文件命名不含拉丁标点，碰见即断块。
        let hard: &[char] = &[
            // 空白
            ' ', '\t', '\n', '\r',
            // 拉丁标点（`.` `/` `\` `-` `_` 是连接符，不在硬边界）
            ',', ';', ':', '!', '?', '\'', '"', '(', ')', '[', ']', '{', '}', '<', '>', '|', '&',
            '%', '$', '#', '@', '=', '+', '*', '^', '`', // CJK 标点
            '，', '。', '、', '；', '：', '？', '！', '“', '”', '‘', '’', '（', '）', '【', '】',
            '《', '》', '「', '」', '『', '』', '—', '…', '·', '～',
        ];
        Self {
            hard: hard.iter().copied().collect(),
        }
    }
}

impl SeparatorTable {
    /// 硬边界：块与块的分界。
    pub fn is_hard(&self, c: char) -> bool {
        self.hard.contains(&c)
    }
}
