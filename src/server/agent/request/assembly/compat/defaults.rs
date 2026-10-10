//! The conservative baseline, and "does this address look private". `Resolved::conservative` is where
//! every detection starts; rules only add to it.

/// Is this base_url loopback or private? Then only the conservative baseline applies and no optional
/// feature is switched on: an address does not say who the upstream is.
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
    // 172.16.0.0/12 → second octet 16..=31; a string prefix test would confuse 172.30 with 172.3.
    if host.starts_with("172.")
        && let Some(second) = host.split('.').nth(1).and_then(|s| s.parse::<u8>().ok())
    {
        return (16..=31).contains(&second);
    }
    false
}
