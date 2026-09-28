//! OpenAI-compatible 服务商的兼容开关。
//!
//! # 这张表从哪来
//!
//! 字段与语义抄自 `oh-my-pi` 的 `packages/catalog/src/types.ts`
//! （`OpenAICompat` 接口，67 个字段）。`oh-my-pi` 是 `pi` 的下游分支，
//! 两边同名字段的注释逐字相同，但 `oh-my-pi` 多出 54 个**服务商怪癖**
//! 开关——那正是本模块要的东西。`pi` 独有的 14 个（标了 `pi:`）也并了
//! 进来：它们跟随上游 API 的新特性，`oh-my-pi` 用 `extra_body` 泛化处理
//! 了，这里单独留字段，语义更清楚。
//!
//! 不抄 `openai-responses` / `openai-codex-*` 专用的那批字段
//! （`supports_configuration_update`、`supports_steering`、
//! `strict_responses_pairing`、`supports_additional_tools`、
//! `supports_tool_search`、`supports_explicit_prompt_cache_mode`、
//! `supports_max_output_tokens`）：那些只对 Responses API 有意义，
//! MyPi 只说 chat/completions。
//!
//! # 为什么要这么细
//!
//! 每一个开关背后都是一个真实服务商的一个 400。举三个例子：
//!
//! * `requires_mistral_tool_ids`——Mistral 要求工具调用 id 恰好 9 位
//!   字母数字，别家的 `call_abc123def456` 它直接拒绝。
//! * `reasoning_content_field`——推理内容这个字段，DeepSeek 叫
//!   `reasoning_content`，OpenRouter 转发时叫 `reasoning`，有的叫
//!   `reasoning_text`。发错了模型就收不到自己的思考过程。
//! * `empty_length_finish_is_context_error`——有的网关在上下文塞满时
//!   返回一个**空的**、`finish_reason: "length"` 的正常响应。不特判的话
//!   用户看到的是"模型什么都没说"，而不是"上下文满了"。
//!
//! # 三层结构
//!
//! * [`Compat`]——用户的表态。全字段 `Option`：`None` = 没意见。
//!   `models.yml` 里每个模型可以写一份，逐项覆写。
//! * [`resolve`]——把 `Compat` 变成 [`Resolved`]：按 `base_url` 与
//!   模型 id 探测，补上默认值。
//! * [`Resolved`]——确定值。**装配层只看这个**，不再做 `Option` 判断。
//!
//! 这样探测规则集中在一处，请求装配处读起来是一串直白的 `if`。
//!
//! # 目录结构
//!
//! * [`types`]——类型定义。不含任何判断。
//! * [`defaults`]——保守基线。认不出来的服务商的默认答案。
//! * [`rules`]——规则表：结构、加载、匹配、冲突检测。**加服务商改这里的数据，
//!   不改代码**（`builtin.yaml`，以及用户的 `compat-rules.yaml`）。
//! * [`engine`]——探测：保守基线 → 跑规则表 → 叠用户表态。
//!
//! 对外暴露 [`types`] 的全部符号、[`resolve`]，以及 [`rules`]（配置层要在
//! 启动时校验规则表，测试也要能读路径）。

pub mod rules;

mod defaults;
mod engine;
mod types;

pub use engine::resolve;
pub use types::*;

#[cfg(test)]
mod tests {
    /// 开关表里的字段名。表由 `compat_table!` 声明，`Compat` 与 `Resolved`
    /// 都由它生成——所以这是唯一的字段清单。
    fn table_fields() -> Vec<String> {
        const TYPES: &str = include_str!("types.rs");
        let start = TYPES
            .find("compat_table! {")
            .expect("开关表不见了——改了名字，这条测试也要改");
        let body = &TYPES[start..];
        let body = &body[body.find('{').expect("表体") + 1..];
        let body = &body[..body.find("\n}").expect("表没有闭合")];
        body.lines()
            .filter_map(|l| {
                let l = l.trim();
                let rest = l
                    .strip_prefix("value ")
                    .or_else(|| l.strip_prefix("relay "))?;
                Some(rest.split(':').next()?.trim().to_string())
            })
            .collect()
    }

    /// 读 `src/**` 里除兼容层与测试代码之外的源码。
    ///
    /// **测试代码必须排除**：一个字段出现在 `assert!` 里不算被实现，那正是
    /// 上一版检查漏掉 `supports_store` 两个字段的原因。
    fn sources_outside_compat() -> String {
        fn walk(dir: &std::path::Path, out: &mut String) {
            for entry in std::fs::read_dir(dir).expect("读 src 失败") {
                let path = entry.expect("读目录项失败").path();
                if path.is_dir() {
                    if path.to_string_lossy().ends_with("ai/compat") {
                        continue; // 字段当然出现在那里：那是定义和搬运的地方。
                    }
                    walk(&path, out);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    let text = std::fs::read_to_string(&path).unwrap_or_default();
                    match text.find("#[cfg(test)]") {
                        Some(at) => out.push_str(&text[..at]),
                        None => out.push_str(&text),
                    }
                }
            }
        }
        let mut out = String::new();
        walk(std::path::Path::new("src"), &mut out);
        assert!(
            out.len() > 100_000,
            "只读到 {} 字节源码，测试大概不在 crate 根目录跑",
            out.len()
        );
        out
    }

    /// 表里的每个字段都必须被兼容层之外的**真实代码**读到。
    ///
    /// 这一条是给一次真实的翻车加的：抄 omp 的字段表时只抄了声明，实现没
    /// 跟上，于是 19 个字段里用户能写、能通过校验、**什么都不会发生**。
    /// 那是最坏的一种状态：看起来配好了，其实没有。
    ///
    /// `Compat` 与 `Resolved` 之间的搬运现在由 `compat_table!` 生成，漏搬
    /// 已经不可能——但"搬过来了却没人读"仍然可能，所以这条留着。
    #[test]
    fn every_field_is_read_outside_the_compat_module() {
        let fields = table_fields();
        assert!(
            fields.len() > 60,
            "只解析出 {} 个字段，说明解析方式坏了",
            fields.len()
        );
        let source = sources_outside_compat();

        let unread: Vec<&String> = fields
            .iter()
            .filter(|f| !source.contains(f.as_str()))
            .collect();
        assert!(
            unread.is_empty(),
            "这些开关没有任何真实代码读（测试里的 `assert!` 不算），写进规则表不会生效：{unread:?}\n\
             要么把它实现掉，要么从开关表里删掉——留着就是骗人的。"
        );
    }

    /// 表里只会出现 `value` 与 `relay` 两种形态。
    ///
    /// 拼错形态（比如 `val`）会被宏当成一个新的 `$kind:ident`，`@resolved_ty`
    /// 匹配不上就**编译不过**——但如果将来有人加了一条 `@resolved_ty` 的通配
    /// 规则，拼错就会变成静默的默认行为。这条测试把那个缺口堵上。
    #[test]
    fn every_row_declares_a_known_shape() {
        const TYPES: &str = include_str!("types.rs");
        let start = TYPES.find("compat_table! {").expect("开关表不见了");
        let body = &TYPES[start..];
        let body = &body[body.find('{').expect("表体") + 1..];
        let body = &body[..body.find("\n}").expect("表没有闭合")];
        let bad: Vec<&str> = body
            .lines()
            .map(str::trim)
            .filter(|l| l.contains(':') && l.ends_with(';') && !l.starts_with("//"))
            .filter(|l| !l.starts_with("value ") && !l.starts_with("relay "))
            .collect();
        assert!(bad.is_empty(), "这些行的形态不认：{bad:?}");
    }
}
