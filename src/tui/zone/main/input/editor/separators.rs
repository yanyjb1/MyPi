//! 分隔符定义表 —— 编辑器"块"的唯一定义源。
//!
//! 两档：
//! - **硬边界**（hard）：块与块的分界。词删除（Ctrl+W）与词滑动
//!   （Ctrl+方向）在硬边界处停下；补全触发词的回扫也停在这里。
//! - **连接符**（connector）：词内合法字符。`src/main.rs`、`./foo`、
//!   `a-b_c.tar.gz` 是**一整块**——滑动滑过、删除一删整块。
//!
//! 其余字符（字母数字、CJK 正文）永远属于块内。
//!
//! 默认表内置；config 重构后通过 [`SeparatorTable::with_overrides`]
//! 从配置注入覆盖表。

use std::collections::HashSet;

/// 两档分隔符表。编辑器与补全触发共用这一个定义。
#[derive(Debug, Clone)]
pub struct SeparatorTable {
    hard: HashSet<char>,
    connector: HashSet<char>,
}

impl Default for SeparatorTable {
    fn default() -> Self {
        // 硬边界：空白 + CJK 标点 + 拉丁标点（连接符除外）。
        // 用户决策：文件命名不含拉丁标点，碰见即断块。
        let hard: &[char] = &[
            // 空白
            ' ', '\t', '\n', '\r',
            // 拉丁标点（`.` `/` `\` `-` `_` 是连接符，不在硬边界）
            ',', ';', ':', '!', '?', '\'', '"', '(', ')', '[', ']', '{', '}',
            '<', '>', '|', '&', '%', '$', '#', '@', '=', '+', '*', '^', '`',
            // CJK 标点
            '，', '。', '、', '；', '：', '？', '！', '“', '”', '‘', '’',
            '（', '）', '【', '】', '《', '》', '「', '」', '『', '』',
            '—', '…', '·', '～',
        ];
        // 连接符：path-like 词的骨架。
        let connector: &[char] = &['.', '/', '\\', '-', '_'];
        Self {
            hard: hard.iter().copied().collect(),
            connector: connector.iter().copied().collect(),
        }
    }
}

impl SeparatorTable {
    /// 硬边界：块与块的分界。
    pub fn is_hard(&self, c: char) -> bool {
        self.hard.contains(&c)
    }

    /// 连接符：词内合法，不构成边界。
    pub fn is_connector(&self, c: char) -> bool {
        self.connector.contains(&c)
    }

    /// 兼容旧语义："是分隔符" = 硬边界或连接符（都停下滑动）。
    /// 注意：词删除的"一删一块"只看 [`Self::is_hard`]。
    pub fn is_separator(&self, c: char) -> bool {
        self.is_hard(c) || self.is_connector(c)
    }

    /// config 注入口：在默认表基础上覆盖某一档。
    pub fn with_overrides(
        mut hard_extra: Vec<char>,
        mut connector_extra: Vec<char>,
    ) -> Self {
        let mut t = Self::default();
        t.hard.extend(hard_extra.drain(..));
        t.connector.extend(connector_extra.drain(..));
        t
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_chars_are_connectors_not_hard() {
        let t = SeparatorTable::default();
        for c in ['.', '/', '\\', '-', '_'] {
            assert!(t.is_connector(c), "{c} 必须是连接符");
            assert!(!t.is_hard(c), "{c} 不能是硬边界");
        }
    }

    #[test]
    fn space_and_cjk_punct_are_hard() {
        let t = SeparatorTable::default();
        assert!(t.is_hard(' '));
        assert!(t.is_hard('，'));
        assert!(t.is_hard('。'));
        assert!(!t.is_connector('，'), "CJK 标点不是连接符");
    }

    #[test]
    fn overrides_extend_default() {
        let t = SeparatorTable::with_overrides(vec!['§'], vec!['~']);
        assert!(t.is_hard('§'));
        assert!(t.is_connector('~'));
    }
}
