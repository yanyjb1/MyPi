//! 保守基线，以及"这个地址像不像私网"。
//!
//! [`Resolved::conservative`] 是全部探测的起点：每一项都选**不会惹恼严格
//! 网关**的那一侧。规则只负责在它之上做增量覆盖（见 [`super::engine`]）。

/// 这个 base_url 是不是回环 / 私网地址。
///
/// 命中后只拿 [`Resolved::conservative`] 的基线，**不开任何可选特性**。
/// 地址不携带"上游是谁"的信息，任何从地址出发的推断都是猜。需要什么特性
/// 应当由**规则**声明，而不是由地址推断。
pub(super) fn is_local(base_url: &str) -> bool {
    let host = super::rules::host_of(base_url);
    if host.is_empty() {
        return false;
    }
    if host == "localhost" || host == "127.0.0.1" || host == "0.0.0.0" || host == "::1" {
        return true;
    }
    if host.starts_with("10.") || host.starts_with("192.168.") {
        return true;
    }
    // 172.16.0.0/12 → 第二段 16..=31。别用 `starts_with("172.1")` 之类
    // 的字符串前缀糊过去：`172.16` 到 `172.31` 不是"以 172.1/172.2/172.3
    // 开头"能说清的（172.30 和 172.3 的边界）。
    if host.starts_with("172.")
        && let Some(second) = host.split('.').nth(1).and_then(|s| s.parse::<u8>().ok())
    {
        return (16..=31).contains(&second);
    }
    false
}
