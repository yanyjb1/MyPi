//! 推理循环守护。
//!
//! # 这是干什么的
//!
//! 模型偶尔会掉进一个退化循环：把同一个意思换着措辞反复吐（"确认安全性"、
//! "验证已完成"……），把一个回合的输出预算烧光，既不调工具也不回答。
//!
//! 这种跑飞**不是逐字节相同的**——只查"末尾有没有原样重复"会漏掉它。所以
//! 这里查两种形状：
//!
//! 1. **精确后缀循环**：一段字节原样连续重复。含很长的周期（观察到过
//!    311 字符的）。
//! 2. **近似重复段落**：按空行切段，两段的词三元组指纹高度重叠（Jaccard
//!    >= 0.8）。措辞变了但说的是同一件事。
//!
//! # 为什么不查"词汇停滞"
//!
//! `oh-my-pi` 还有第三条启发式（词表新颖度低且没提到具体路径/标识符的段
//! 落连成一片就判循环），阈值是在 13.5k 段真实思考上标定的。MyPi 没有那
//! 份语料，抄一套没标定过的阈值只会制造误报——而误报的代价是**掐断一次
//! 正常的长思考**，比漏报更贵。前两条不需要语料：一条是精确数学，一条是
//! 相似度上界。
//!
//! # 命中之后做什么
//!
//! 由调用方决定。`Client::stream_once` 把它当成一次**可重试的失败**：本轮
//! 的输出丢掉，不算成功。不这么做的话，用户拿到的是半段循环文本，还得自
//! 己发现。

/// 滚动尾巴的长度上限。只在这个窗口里找重复，不随回复变长而无界增长。
const EXACT_TAIL_WINDOW: usize = 4096;
/// 检测的最小周期。比这更长的"周期"多半是巧合。
const EXACT_MAX_UNIT: usize = 1024;
/// 每收到这么多字符才扫一次。逐 delta 扫是二次复杂度。
const EXACT_CHECK_STRIDE: usize = 128;
/// 短周期的分界：小于它的要求重复更多次、总字符更多。
const EXACT_SHORT_MAX_UNIT: usize = 60;
const EXACT_SHORT_MIN_REPEATED_CHARS: usize = 180;
const EXACT_LONG_MIN_REPEATED_CHARS: usize = 1024;

/// 段落按这个长度切片，保证每段可比。
const SEGMENT_CHAR_CAP: usize = 700;
/// 归一化后短于这个长度的段不参与比较（太短的三元组没信息量）。
const SEGMENT_MIN_NORM_CHARS: usize = 60;
/// 记住最近多少段。
const SEGMENT_WINDOW: usize = 16;
/// 两段的词三元组 Jaccard 相似度到这个值就算"同一件事换了个说法"。
const SEGMENT_SIMILARITY: f64 = 0.8;
/// 攒够多少段才开始判（warm-up，避免开头就误报）。
const SEGMENT_MIN_COUNT: usize = 8;
/// 最近窗口里有这么多段与当前段相似 → 循环。
const SEGMENT_MIN_CLUSTER: usize = 4;

/// 一个推理流的循环检测器。只喂**推理文本**的增量，不喂正文。
///
/// 只能由 [`ThinkingLoopGuard::for_compat`] 造出来——没有这个开关的宿主连
/// 检测器都不该存在，省掉每条 delta 上的扫描开销。
pub(crate) struct ThinkingLoopGuard {
    /// 滚动尾巴，用于精确后缀循环。
    tail: String,
    /// 距上次精确检测又收了多少字符。
    since_check: usize,
    /// 还没切成完整段的文本。
    pending: String,
    /// 已完成段的指纹（词三元组集合）。环形窗口。
    segments: std::collections::VecDeque<std::collections::HashSet<String>>,
    /// 已完成的段数（warm-up 用）。
    count: usize,
}

impl ThinkingLoopGuard {
    /// 这个宿主要不要守护。没有 `thinking_loop_guard` 就是不要。
    pub(crate) fn for_compat(
        compat: &crate::server::ai::compat::Resolved,
    ) -> Option<Self> {
        compat.thinking_loop_guard.map(|_| Self {
            tail: String::new(),
            since_check: 0,
            pending: String::new(),
            segments: std::collections::VecDeque::new(),
            count: 0,
        })
    }

    /// 喂一段推理增量。返回 `Some(原因)` 表示判定为循环——调用方应当**停止
    /// 本轮**并按可重试失败处理。
    pub(crate) fn push(&mut self, delta: &str) -> Option<String> {
        if delta.is_empty() {
            return None;
        }

        // ---- 1. 精确后缀循环 ----
        self.tail.push_str(delta);
        if self.tail.len() > EXACT_TAIL_WINDOW * 2 {
            let cut = self.tail.len() - EXACT_TAIL_WINDOW;
            // 按字符边界切，别切断 UTF-8。
            let cut = (0..=cut).rev().find(|i| self.tail.is_char_boundary(*i))?;
            self.tail.drain(..cut);
        }
        self.since_check += delta.len();
        if self.since_check >= EXACT_CHECK_STRIDE || delta.len() >= EXACT_CHECK_STRIDE {
            self.since_check = 0;
            if let Some((unit, times)) = detect_exact_suffix_cycle(&self.tail) {
                return Some(format!(
                    "the same {} characters repeated {} times in a row",
                    unit.chars().count(),
                    times
                ));
            }
        }

        // ---- 2. 近似重复段落 ----
        self.pending.push_str(delta);
        while let Some(cut) = self.next_segment_boundary() {
            let segment: String = self.pending.drain(..cut).collect();
            if let Some(reason) = self.consume_segment(&segment) {
                return Some(reason);
            }
        }
        None
    }

    /// 下一个完整段的字节长度（含分隔符）。没有完整段就 `None`。
    fn next_segment_boundary(&self) -> Option<usize> {
        // 空行是段边界；找不到但已经超长就强制切，否则一个没有空行的
        // 跑飞段永远进不了检测。
        let mut search = 0;
        while let Some(at) = self.pending[search..].find('\n') {
            let abs = search + at;
            let rest = &self.pending[abs + 1..];
            let blank = rest.starts_with('\n') || rest.trim_start_matches(' ').starts_with('\n');
            if blank {
                let mut end = abs + 1;
                while end < self.pending.len()
                    && self.pending[end..].starts_with('\n')
                {
                    end += 1;
                }
                return Some(end);
            }
            search = abs + 1;
        }
        if self.pending.chars().count() > SEGMENT_CHAR_CAP {
            // 按字符数切，保证不切断 UTF-8。
            let byte_at = self
                .pending
                .char_indices()
                .nth(SEGMENT_CHAR_CAP)
                .map(|(i, _)| i);
            return byte_at.or(Some(self.pending.len()));
        }
        None
    }

    fn consume_segment(&mut self, raw: &str) -> Option<String> {
        let normalized = normalize_segment(raw);
        if normalized.chars().count() < SEGMENT_MIN_NORM_CHARS {
            return None;
        }
        let shingles = trigram_shingles(&normalized);
        if shingles.is_empty() {
            return None;
        }
        self.count += 1;

        // warm-up 之后才判：开头几段本来就可能是同构的（比如清单式思考）。
        if self.count > SEGMENT_MIN_COUNT {
            let similar = self
                .segments
                .iter()
                .filter(|prev| jaccard(prev, &shingles) >= SEGMENT_SIMILARITY)
                .count();
            if similar >= SEGMENT_MIN_CLUSTER {
                return Some(format!(
                    "{} consecutive segments are near-duplicates (the same thing said again)",
                    similar + 1
                ));
            }
        }

        if self.segments.len() >= SEGMENT_WINDOW {
            self.segments.pop_front();
        }
        self.segments.push_back(shingles);
        None
    }
}

/// 末尾有没有一段字节原样连续重复。
///
/// 从短周期往长周期扫：短周期几乎不可能是巧合，先撞上就先报。周期必须是
/// 良性内容（含字母）——`\n\n\n\n` 或 `----` 这种排版重复不是循环。
fn detect_exact_suffix_cycle(text: &str) -> Option<(String, usize)> {
    let bytes = text.as_bytes();
    if bytes.len() < EXACT_SHORT_MIN_REPEATED_CHARS {
        return None;
    }
    let max_unit = EXACT_MAX_UNIT.min(bytes.len() / 3);
    for len in 2..=max_unit {
        let unit = &bytes[bytes.len() - len..];
        // 良性内容才可能是循环：`\n\n\n` 或 `----` 这种排版重复不是。
        // 判据是"含字母"。**不能只看 ASCII 字母**：中文一个 ASCII 字母都没
        // 有。所以认 `>= 0x80` 的字节——UTF-8 里的非 ASCII 字符，在中文和
        // 别的文字里正是字母所在。
        //
        // 这一步只看字节、不构造字符串：它是逐周期跑的，不该在没命中时分配。
        if !unit.iter().any(|b| b.is_ascii_alphabetic() || *b >= 0x80) {
            continue;
        }
        let mut count = 1;
        let mut end = bytes.len() - len;
        while end >= len && &bytes[end - len..end] == unit {
            count += 1;
            end -= len;
        }
        let (min_count, min_chars) = if len <= EXACT_SHORT_MAX_UNIT {
            (4, EXACT_SHORT_MIN_REPEATED_CHARS)
        } else {
            (3, EXACT_LONG_MIN_REPEATED_CHARS)
        };
        if count >= min_count && len * count >= min_chars {
            return Some((String::from_utf8_lossy(unit).into_owned(), count));
        }
    }
    None
}

/// 小写、只留字母与数字的**连续串**，标点和空白都丢掉。
///
/// 不去分词：中文没有词边界，按空白切会把整段变成一个"词"，相似度就退化
/// 成"逐字相同才算相同"。去掉空白之后用**字符** n-gram，中英文都成立。
fn normalize_segment(segment: &str) -> String {
    segment
        .chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// 字符三元组集合。
fn trigram_shingles(normalized: &str) -> std::collections::HashSet<String> {
    let chars: Vec<char> = normalized.chars().collect();
    if chars.len() < 3 {
        return match chars.is_empty() {
            true => std::collections::HashSet::new(),
            false => [normalized.to_string()].into_iter().collect(),
        };
    }
    (0..=chars.len() - 3)
        .map(|i| chars[i..i + 3].iter().collect::<String>())
        .collect()
}

fn jaccard(a: &std::collections::HashSet<String>, b: &std::collections::HashSet<String>) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    let inter = a.intersection(b).count() as f64;
    let union = a.union(b).count() as f64;
    if union == 0.0 { 0.0 } else { inter / union }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn guard() -> ThinkingLoopGuard {
        ThinkingLoopGuard {
            tail: String::new(),
            since_check: 0,
            pending: String::new(),
            segments: std::collections::VecDeque::new(),
            count: 0,
        }
    }

    /// 原样重复一小段，很快就被抓到。
    #[test]
    fn an_exact_repeat_is_caught() {
        let mut g = guard();
        let unit = "让我再确认一下这个文件的内容。";
        let mut hit = None;
        for _ in 0..12 {
            if let Some(r) = g.push(unit) {
                hit = Some(r);
                break;
            }
        }
        assert!(hit.is_some(), "逐字重复必须被抓到");
    }

    /// 换了措辞但说的是同一件事——精确检测抓不到，段落指纹要抓到。
    #[test]
    fn a_paraphrased_loop_is_caught() {
        let mut g = guard();
        let mut hit = None;
        for i in 0..14 {
            let para = format!(
                "第{i}轮思考：我需要再确认一下这个函数的边界条件是否正确处理了空输入的情况，\
                 确保这个函数的边界条件在空输入的时候能够正确地返回而不是崩溃，\
                 确认这个函数的边界条件处理是正确的。\n\n"
            );
            if let Some(r) = g.push(&para) {
                hit = Some(r);
                break;
            }
        }
        assert!(hit.is_some(), "换着说法重复同一件事也要被抓到：{hit:?}");
    }

    /// 正常的、内容各不相同的长思考不能误报——误报的代价是掐断一次真思考。
    #[test]
    fn genuine_thinking_is_not_flagged() {
        let mut g = guard();
        let paras = [
            "先看 src/server/ai/client.rs 里的读循环，那里有一个 1 秒的 SO_RCVTIMEO。",
            "再确认 turn.rs 里请求头是在请求发出之前落盘的，否则 replay 对不上。",
            "然后检查 session.rs 的 replay_round 能不能只靠数据库重建请求体。",
            "最后看一下 compose 里的 http_worker 协议：body 是完全组装好的成品。",
            "还有 compat 的规则表，用户写在 XDG_CONFIG_HOME 下的那份要能盖过出厂表。",
            "另外 store.rs 的 SCHEMA_VERSION 现在是 4，改列的话要一起抬。",
            "TUI 那边的输入层有个 zone/main/input/mod.rs，改动要跟这部分对齐。",
            "最后跑一遍全量测试，确认没有回归，再看一眼有没有编译警告。",
            "收尾之前把文档里过期的配置键名找出来，ARCHITECTURE.md 里那几个是旧的。",
        ];
        for p in paras {
            assert!(g.push(p).is_none(), "正常思考被误判：{p}");
        }
    }

    /// 排版重复（空行、分隔线）不是循环。
    #[test]
    fn formatting_repeats_are_not_a_loop() {
        let mut g = guard();
        assert!(g.push(&"\n".repeat(400)).is_none());
        assert!(g.push(&"-".repeat(400)).is_none());
    }
}
