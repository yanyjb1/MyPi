//! 探测：把 `(base_url, model_id, 用户表态)` 变成确定的 [`Resolved`]。
//!
//! 流程：保守基线 → 跑规则表 → 叠用户表态。规则表是**数据**
//! （`builtin.yaml` 加用户的 `compat-rules.yaml`，见 [`super::rules`]），
//! 所以**加一个服务商不需要改这个文件**。

use super::defaults::is_local;
use super::rules::{Probe, Rule, host_of, rules};
use super::types::*;

impl Resolved {
    /// 按 base_url 与模型 id 探测。
    ///
    /// 顺序：保守基线 → 规则表（priority 升序，同优先级按声明顺序）
    /// → 用户表态（由 [`resolve`] 调 [`Resolved::apply`]）。
    ///
    /// 私网地址不额外假设任何东西：规则表里那条 `private-host` 与这里的
    /// 保守基线等价。理由见 [`super::defaults::is_local`]。
    pub fn detect(base_url: &str, model_id: &str) -> Self {
        let mut r = Self::conservative();
        let host = host_of(base_url);
        let model = model_id.to_ascii_lowercase();
        let probe = Probe {
            host: &host,
            model: &model,
            is_private: is_local(base_url),
        };
        // 启动时 `Config::load` 已经验过规则表。这里再失败，只可能是文件在
        // 进程运行期间被改坏——那时退回保守基线，比 panic 合理。
        if let Ok(list) = rules() {
            let mut matched: Vec<&Rule> =
                list.iter().filter(|rule| rule.matches(&probe)).collect();
            // 用户表整张排在出厂表之后；同来源内按 priority；都不相同时
            // `sort_by_key` 稳定，保持文件里的声明顺序。
            matched.sort_by_key(|rule| (rule.source, rule.priority));
            for rule in matched {
                r.apply(&rule.set);
            }
        }
        r
    }
}

pub fn resolve(base_url: &str, model_id: &str, user: Option<&Compat>) -> Resolved {
    let mut r = Resolved::detect(base_url, model_id);
    if let Some(u) = user {
        r.apply(u);
    }
    r
}



