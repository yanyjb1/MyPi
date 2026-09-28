//! 兼容规则表：结构、加载、匹配、冲突检测。
//!
//! # 为什么是数据
//!
//! "某个服务商要让请求体长什么样"是**事实**，不是逻辑。写在代码里意味着
//! 每加一个服务商都要改 Rust、重新发版；写成这个文件里的一条记录，加一个
//! 服务商就是加一条 YAML。
//!
//! 字段名与 [`Compat`] 完全一致，所以一条规则的内容可以原样抄进
//! `models.yml` 里某个模型的 `compat:` 块做局部覆盖，反过来也行。
//!
//! # 选择器
//!
//! [`Rule::any`] 里任一 [`Selector`] 命中即算命中。一个 selector 内多个键是
//! AND，一个 glob 列表内多个模式是 OR。`any` 为空 = 匹配一切。
//!
//! # 冲突
//!
//! 两条**同优先级**的规则可能同时命中、又向同一字段写不同值时，
//! [`load`] 直接失败。静默取最后一个的代价是"某个模型的怪癖突然不生效"，
//! 而那是线上最难查的一类问题。让启动失败、说清是哪两条规则打架，是
//! 可诊断与不可诊断之间的分界线。

use std::path::PathBuf;
use std::sync::OnceLock;

use anyhow::{Context as _, bail};
use serde::{Deserialize, Serialize};

use super::types::Compat;

/// 编进二进制的出厂规则表。
pub const BUILTIN: &str = include_str!("builtin.yaml");

/// 规则表文件。
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RuleSet {
    #[serde(default = "one")]
    pub version: u32,
    #[serde(default)]
    pub rules: Vec<Rule>,
}

fn one() -> u32 {
    1
}

/// 一条规则：什么情况下（`any`）要改哪些开关（`set`）。
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Rule {
    /// 冲突报错时用来说清是哪两条。
    pub name: String,
    /// 数字大的后应用（覆盖小的）。默认 0，按声明顺序。
    #[serde(default)]
    pub priority: i32,
    /// 0 = 出厂表，1 = 用户表。**不写进 YAML**，由加载顺序决定：用户的
    /// 规则整张表都在出厂表之后，所以用户永远能覆盖出厂结论。
    #[serde(skip, default)]
    pub source: u8,
    /// 任一 selector 命中即算这条命中；为空 = 匹配一切。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub any: Vec<Selector>,
    /// 命中后要设的开关。与 `models.yml` 里的 `compat:` 块同形。
    #[serde(default)]
    pub set: Compat,
}

/// 一个匹配条件。组内多个键是 AND。
///
/// 三个键全部省略的 selector 匹配一切——可以，但没有意义（等于没写这条
/// 规则），所以 `deny_unknown_fields` + 文档都在往"写清楚"的方向推。
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Selector {
    /// 回环 / 私网地址。`true` 只匹配私网，`false` 只匹配公网。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_is_private: Option<bool>,
    /// base url 的 host，不含 scheme 与端口。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<Vec<String>>,
    /// 线上模型 id。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub models: Option<Vec<String>>,
}

/// 一次探测的输入，已归一化。
pub struct Probe<'a> {
    /// 小写、无端口、无 path 的 host。
    pub host: &'a str,
    /// 小写的线上模型 id。
    pub model: &'a str,
    pub is_private: bool,
}

impl Selector {
    fn matches(&self, probe: &Probe<'_>) -> bool {
        if let Some(want) = self.host_is_private
            && want != probe.is_private
        {
            return false;
        }
        if let Some(globs) = &self.host
            && !globs.iter().any(|g| glob(g, probe.host))
        {
            return false;
        }
        if let Some(globs) = &self.models
            && !globs.iter().any(|g| glob(g, probe.model))
        {
            return false;
        }
        true
    }
}

impl Rule {
    pub fn matches(&self, probe: &Probe<'_>) -> bool {
        self.any.is_empty() || self.any.iter().any(|s| s.matches(probe))
    }
}

/// 从 base url 里取出 host，小写、去端口。
///
/// `https://api.deepseek.com/v1` → `api.deepseek.com`；
/// `http://127.0.0.1:1/v1` → `127.0.0.1`；裸 host 也接受。
pub fn host_of(base_url: &str) -> String {
    let rest = base_url.split("://").nth(1).unwrap_or(base_url);
    let authority = rest.split('/').next().unwrap_or(rest);
    // user:pass@host  —— 取 @ 之后
    let authority = authority.rsplit('@').next().unwrap_or(authority);
    let host = if authority.starts_with('[') {
        // IPv6 字面量：[::1]:1 —— 端口在 ] 之后
        authority.split(']').next().unwrap_or(authority).trim_start_matches('[')
    } else {
        authority.split(':').next().unwrap_or(authority)
    };
    host.to_ascii_lowercase()
}

/// `*` 任意串（含空），`?` 恰好一个字符。大小写不敏感。
///
/// 迭代式双指针 + 单点回溯：不递归，长模式不会爆栈。
pub fn glob(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().flat_map(char::to_lowercase).collect();
    let t: Vec<char> = text.chars().flat_map(char::to_lowercase).collect();
    let (mut pi, mut ti) = (0usize, 0usize);
    let mut star: Option<usize> = None;
    let mut mark = 0usize;
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

/// 用户规则表路径：与 `models.yml` 同目录，`$MYPI_MODELS` 生效时跟着走。
pub fn user_path() -> Option<PathBuf> {
    crate::server::ai::config::Config::models_path()
        .ok()
        .map(|p| p.with_file_name("compat-rules.yaml"))
}

/// 规则表是否可能同时命中同一个请求。
///
/// 这是**保守**判断：认为"可能"就检查字段冲突，认为"不可能"就跳过。两个
/// 不同 glob 实际相交却判成不相交，只会**漏报**冲突（行为退回"按顺序
/// 叠加"，与没有这条检查时一样），不会误报。
fn may_overlap(a: &Rule, b: &Rule) -> bool {
    if a.any.is_empty() || b.any.is_empty() {
        return true;
    }
    a.any
        .iter()
        .any(|x| b.any.iter().any(|y| selectors_may_overlap(x, y)))
}

fn selectors_may_overlap(x: &Selector, y: &Selector) -> bool {
    if let (Some(p), Some(q)) = (x.host_is_private, y.host_is_private)
        && p != q
    {
        return false;
    }
    globs_may_overlap(&x.host, &y.host) && globs_may_overlap(&x.models, &y.models)
}

fn globs_may_overlap(x: &Option<Vec<String>>, y: &Option<Vec<String>>) -> bool {
    match (x, y) {
        // 一方没有这个约束 = 它覆盖对方约束的全部取值 → 可能重叠。
        (None, _) | (_, None) => true,
        (Some(x), Some(y)) => x
            .iter()
            .any(|g| y.iter().any(|h| globs_intersect(g, h))),
    }
}

fn globs_intersect(g: &str, h: &str) -> bool {
    let (g, h) = (g.to_ascii_lowercase(), h.to_ascii_lowercase());
    if g.contains(&h) || h.contains(&g) {
        return true;
    }
    // 比 `*` 之前的字面前缀：`deepseek-*` 与 `deepseek-r1` 共前缀 → 相交。
    let pg = g.split('*').next().unwrap_or("");
    let ph = h.split('*').next().unwrap_or("");
    pg.starts_with(ph) || ph.starts_with(pg)
}

/// 同优先级、可能同时命中、向同一字段写不同值 → 报错。
pub fn check_conflicts(rules: &[Rule]) -> anyhow::Result<()> {
    for (i, a) in rules.iter().enumerate() {
        for b in &rules[i + 1..] {
            // 不同来源或不同优先级 = 作者已经表态谁赢，不是冲突。
            if a.source != b.source || a.priority != b.priority || !may_overlap(a, b) {
                continue;
            }
            let (va, vb) = (
                serde_json::to_value(&a.set).context("failed to serialize the rule set")?,
                serde_json::to_value(&b.set).context("failed to serialize the rule set")?,
            );
            let (Some(oa), Some(ob)) = (va.as_object(), vb.as_object()) else {
                continue;
            };
            for (key, x) in oa {
                // `null` 在这里就是"这条规则没表这个态"，不是"设成 null"。
                if x.is_null() {
                    continue;
                }
                match ob.get(key) {
                    Some(y) if !y.is_null() && y != x => bail!(
                        "compat rule conflict: `{key}` is set to different values at the same priority by `{a}` ({x}) \
                         and `{b}` ({y}). Give one of them a `priority`, or drop one.",
                        a = a.name,
                        b = b.name,
                    ),
                    _ => {}
                }
            }
        }
    }
    Ok(())
}

/// 解析一张规则表文本。`origin` 只用于错误信息。
pub fn parse(text: &str, origin: &str) -> anyhow::Result<Vec<Rule>> {
    let set: RuleSet =
        serde_yaml::from_str(text).with_context(|| format!("failed to parse the compat rule table {origin}"))?;
    if set.version != 1 {
        bail!(
            "compat rule table {origin} has version {}, but this build only understands 1",
            set.version
        );
    }
    Ok(set.rules)
}

/// 出厂表 + 用户表（存在才读）。
pub fn load() -> anyhow::Result<Vec<Rule>> {
    let mut all = parse(BUILTIN, "(built-in)")?;
    if let Some(path) = user_path()
        && path.exists()
    {
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let mut user = parse(&text, &path.display().to_string())?;
        for r in &mut user {
            r.source = 1;
        }
        all.extend(user);
    }
    for r in &all {
        if r.any.is_empty() {
            bail!(
                "compat rule `{}` has no selectors and would match everything. Either delete it, \
                 or give it an `any:`.",
                r.name
            );
        }
    }
    check_conflicts(&all)?;
    Ok(all)
}

/// 进程内缓存。规则表在启动时定形——改它要重启，与 `models.yml` 同待遇。
static RULES: OnceLock<Result<Vec<Rule>, String>> = OnceLock::new();

pub fn rules() -> anyhow::Result<&'static [Rule]> {
    match RULES.get_or_init(|| load().map_err(|e| format!("{e:#}"))) {
        Ok(v) => Ok(v),
        Err(e) => bail!("{e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::super::defaults::is_local;
    use super::super::types::{MaxTokensField, Resolved, ThinkingFormat};
    use super::*;
    use crate::server::ai::compat::resolve;

    fn probe<'a>(host: &'a str, model: &'a str, is_private: bool) -> Probe<'a> {
        Probe {
            host,
            model,
            is_private,
        }
    }

    /// 把一条规则表跑成一个 `Resolved`——与 `engine::detect` 同一条路径，
    /// 只是不做 IO。
    fn run(text: &str, host: &str, model: &str) -> Resolved {
        let rules = parse(text, "test").expect("解析");
        let p = probe(host, model, is_local(&format!("http://{host}/v1")));
        let mut matched: Vec<&Rule> = rules.iter().filter(|r| r.matches(&p)).collect();
        matched.sort_by_key(|r| (r.source, r.priority));
        let mut out = Resolved::conservative();
        for rule in matched {
            out.apply(&rule.set);
        }
        out
    }

    const MYCORP: &str = r#"
version: 1
rules:
  - name: mycorp
    any:
      - host: ["llm.mycorp.internal"]
    set:
      maxTokensField: max_completion_tokens
      supportsStore: true
      thinkingFormat: deepseek
"#;

    /// 加一条规则，行为就变——这是"声明式真的生效"的唯一证据。
    /// 没有任何 Rust 代码知道 `llm.mycorp.internal` 是谁。
    #[test]
    fn a_rule_written_as_data_changes_the_wire() {
        let off = run(MYCORP, "api.example.com", "whatever");
        assert_eq!(off.max_tokens_field, MaxTokensField::MaxTokens, "没命中就是基线");
        assert!(!off.supports_store);

        let on = run(MYCORP, "llm.mycorp.internal", "whatever");
        assert_eq!(on.max_tokens_field, MaxTokensField::MaxCompletionTokens);
        assert!(on.supports_store);
    }

    /// `models` 这一维单独就能命中：OpenRouter 转发的 deepseek 与自建网关
    /// 挂的 deepseek 域名不同，但都该走同一条规则。
    #[test]
    fn a_model_glob_matches_regardless_of_host() {
        let r = run(MYCORP, "api.example.com", "x");
        assert_eq!(r.thinking_format, ThinkingFormat::Openai);

        let custom = r#"
version: 1
rules:
  - name: by-model
    any:
      - models: ["*-flash-*"]
    set:
      thinkingFormat: openrouter
"#;
        let hit = run(custom, "api.whatever.example", "deepseek-v4.1-flash-0813");
        assert_eq!(
            hit.thinking_format,
            ThinkingFormat::Openrouter
        );
        let miss = run(custom, "api.whatever.example", "deepseek-v4.1-pro");
        assert_eq!(
            miss.thinking_format,
            ThinkingFormat::Openai
        );
    }

    #[test]
    fn a_higher_priority_rule_wins() {
        let text = r#"
version: 1
rules:
  - name: first
    priority: 1
    any: [{host: ["h.example"]}]
    set: {maxTokensField: max_completion_tokens}
  - name: second
    priority: 2
    any: [{host: ["h.example"]}]
    set: {maxTokensField: max_tokens}
"#;
        check_conflicts(&parse(text, "test").unwrap()).expect("不同优先级不是冲突");
        assert_eq!(run(text, "h.example", "m").max_tokens_field, MaxTokensField::MaxTokens);
    }

    /// 同优先级、可能同时命中、写同一字段的不同值 → 加载就失败。
    /// 静默取最后一个的代价是"某个模型的怪癖突然不生效"，线上最难查的一类。
    #[test]
    fn same_priority_disagreement_is_refused_not_silently_picked() {
        let text = r#"
version: 1
rules:
  - name: base
    any: [{host: ["h.example"]}]
    set: {maxTokensField: max_completion_tokens}
  - name: other
    any: [{host: ["h.example"]}]
    set: {maxTokensField: max_tokens}
"#;
        let err = format!(
            "{:#}",
            check_conflicts(&parse(text, "test").unwrap()).unwrap_err()
        );
        assert!(err.contains("base") && err.contains("other"), "{err}");
        assert!(err.contains("maxTokensField"), "{err}");
    }

    /// 选择器不可能同时命中时不算冲突（两条规则管的是两拨不同的服务商）。
    #[test]
    fn disjoint_selectors_are_not_a_conflict() {
        let text = r#"
version: 1
rules:
  - name: a
    any: [{host: ["a.example"]}]
    set: {maxTokensField: max_completion_tokens}
  - name: b
    any: [{host: ["b.example"]}]
    set: {maxTokensField: max_tokens}
"#;
        check_conflicts(&parse(text, "test").unwrap()).expect("不同 host 不该报冲突");
    }

    /// 用户表整张排在出厂表之后——同 priority 也压得过出厂结论。
    #[test]
    fn the_user_table_overrides_the_builtin_one() {
        let mut builtin = parse(BUILTIN, "builtin").unwrap();
        let mut user = parse(
            r#"
version: 1
rules:
  - name: user-says-no-store
    any: [{host: ["api.openai.com"]}]
    set: {supportsStore: false}
"#,
            "user",
        )
        .unwrap();
        for r in &mut user {
            r.source = 1;
        }
        builtin.extend(user);
        check_conflicts(&builtin).expect("来源不同不是冲突");

        let p = probe("api.openai.com", "gpt-5.6", false);
        let mut matched: Vec<&Rule> = builtin.iter().filter(|r| r.matches(&p)).collect();
        matched.sort_by_key(|r| (r.source, r.priority));
        let mut out = Resolved::conservative();
        for rule in matched {
            out.apply(&rule.set);
        }
        assert!(!out.supports_store, "用户写的 false 必须压过出厂表的 true");
    }

    /// 三层覆写的最高层：`models.yml` 里那个模型的 `compat:` 块。
    #[test]
    fn the_model_compat_block_beats_every_rule() {
        let user = Compat {
            max_tokens_field: Some(MaxTokensField::MaxTokens),
            ..Default::default()
        };
        let r = resolve("https://api.openai.com/v1", "gpt-5.6", Some(&user));
        assert_eq!(
            r.max_tokens_field,
            MaxTokensField::MaxTokens,
            "出厂规则说 max_completion_tokens，用户说 max_tokens，用户赢"
        );
    }

    #[test]
    fn globs_match_their_edges() {
        assert!(glob("*v4.1-flash*", "deepseek-v4.1-flash-0813"));
        assert!(!glob("*v4.1-flash*", "deepseek-v4-pro"));
        assert!(glob("gpt-5*", "GPT-5.6"), "大小写不敏感");
        assert!(glob("a?c", "abc"));
        assert!(!glob("a?c", "abbc"), "? 恰好一个字符");
        assert!(glob("*", ""), "* 匹配空串");
        assert!(glob("", ""), "空模式匹配空串");
        assert!(!glob("", "x"));
        assert!(!glob("deepseek-*", "xdeepseek-r1"), "前缀不加 * 就不匹配中间");
    }

    #[test]
    fn private_hosts_are_recognized_by_address_alone() {
        for h in [
            "127.0.0.1",
            "localhost",
            "10.1.2.3",
            "192.168.1.1",
            "172.16.0.1",
            "172.31.255.1",
        ] {
            assert!(is_local(&format!("http://{h}:1/v1")), "{h} 应当算私网");
        }
        for h in ["api.deepseek.com", "8.8.8.8", "172.32.0.1", "172.15.0.1"] {
            assert!(!is_local(&format!("http://{h}/v1")), "{h} 不该算私网");
        }
    }

    /// 拼错的开关必须报错。一个拼错的开关等于一个不生效的开关，
    /// 而它看起来完全正常——那是最贵的一种错。
    #[test]
    fn a_mistyped_switch_is_refused_not_ignored() {
        let err = format!(
            "{:#}",
            parse(
                "version: 1\nrules:\n  - name: x\n    any: [{host: [\"h\"]}]\n    set: {supportsStorr: true}\n",
                "test"
            )
            .unwrap_err()
        );
        assert!(err.contains("supportsStorr"), "{err}");
    }

    /// 一条什么选择器都不写的规则会匹配一切，等于悄悄改掉所有服务商。
    #[test]
    fn a_rule_without_selectors_is_refused() {
        let text = "version: 1\nrules:\n  - name: greedy\n    set: {supportsStore: true}\n";
        let rules = parse(text, "test").unwrap();
        assert!(rules[0].any.is_empty());
        assert!(rules[0].matches(&probe("anywhere", "anything", false)));
    }
}
