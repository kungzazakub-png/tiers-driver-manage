//! Which hosts / URLs the program may download from. The lists are compiled in: a catalog or manifest can never widen them.

/// Vendor and Microsoft download hosts (a host matches itself and its sub-domains).
pub const BUILT_IN_HOSTS: &[&str] = &["nvidia.com", "intel.com", "windowsupdate.com"];
/// Hosts that may be asked for driver lookups (not for downloads).
pub const API_HOSTS: &[&str] = &["nvidia.com", "geforce.com"];
/// Hosts for the program's own update manifest and online catalog.
pub const UPDATE_HOSTS: &[&str] = &["github.com", "githubusercontent.com"];
/// The publisher's own mirror: release assets of this one repository, not all of github.com.
pub const MIRROR_PREFIXES: &[&str] = &["https://github.com/kungzazakub-png/tiers-driver-manage/releases/download/"];

/// Splits "https://host[:port]/path" and returns the lower-case host. Credentials in the URL are refused
/// (`https://nvidia.com@evil.example/` would otherwise look like a nvidia.com link).
pub fn https_host(url: &str) -> Result<String, String> {
    let rest = match url.get(..8) {
        Some(p) if p.eq_ignore_ascii_case("https://") => &url[8..],
        _ => return Err(format!("Links that are not https are not allowed: {url}")),
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.is_empty() || authority.contains('@') {
        return Err(format!("Host {authority} is not in the program's allowed list"));
    }
    let host = authority.rsplit_once(':').filter(|(_, port)| port.chars().all(|c| c.is_ascii_digit())).map(|(h, _)| h).unwrap_or(authority);
    Ok(host.to_ascii_lowercase())
}

fn host_allowed(host: &str, hosts: &[&str]) -> bool {
    hosts.iter().any(|h| host == *h || host.ends_with(&format!(".{h}")))
}

/// Download check: https, and either an allowed host or the publisher's own mirror prefix.
pub fn assert_allowed_url(url: &str) -> Result<(), String> {
    assert_allowed_url_with(url, BUILT_IN_HOSTS, true)
}

pub fn assert_allowed_url_with(url: &str, hosts: &[&str], allow_mirror: bool) -> Result<(), String> {
    let host = https_host(url)?;
    if allow_mirror && MIRROR_PREFIXES.iter().any(|p| url.len() >= p.len() && url[..p.len()].eq_ignore_ascii_case(p)) {
        return Ok(());
    }
    if host_allowed(&host, hosts) { Ok(()) } else { Err(format!("Host {host} is not in the program's allowed list")) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_http_is_refused() {
        assert!(assert_allowed_url("http://us.download.nvidia.com/x.exe").unwrap_err().contains("not https"));
    }
    #[test]
    fn host_outside_the_list_is_refused() {
        assert!(assert_allowed_url("https://evil.example.com/x.exe").unwrap_err().contains("allowed list"));
    }
    #[test]
    fn allowed_host_and_subdomain_are_accepted() {
        assert!(assert_allowed_url("https://us.download.nvidia.com/x.exe").is_ok());
        assert!(assert_allowed_url("https://downloadmirror.intel.com/1/x.exe").is_ok());
        assert!(assert_allowed_url("https://catalog.s.download.windowsupdate.com/x.cab").is_ok());
    }
    #[test]
    fn look_alike_hosts_are_refused() {
        assert!(assert_allowed_url("https://notnvidia.com/x.exe").is_err());
        assert!(assert_allowed_url("https://nvidia.com.evil.example/x.exe").is_err());
        assert!(assert_allowed_url("https://nvidia.com@evil.example/x.exe").is_err());
    }
    #[test]
    fn port_is_ignored_for_the_host_check() {
        assert!(assert_allowed_url("https://us.download.nvidia.com:443/x.exe").is_ok());
    }
    #[test]
    fn publisher_mirror_is_allowed_but_other_github_repos_are_not() {
        assert!(assert_allowed_url("https://github.com/kungzazakub-png/tiers-driver-manage/releases/download/drivers/x.exe").is_ok());
        assert!(assert_allowed_url("https://github.com/someone/else/releases/download/v1/x.exe").is_err());
        assert!(assert_allowed_url("https://github.com/kungzazakub-png/tiers-driver-manage-evil/releases/download/v1/x.exe").is_err());
    }
    #[test]
    fn update_hosts_cover_github() {
        assert!(assert_allowed_url_with("https://github.com/o/r/releases/download/v/a.zip", UPDATE_HOSTS, false).is_ok());
        assert!(assert_allowed_url_with("https://evil.example.com/a.zip", UPDATE_HOSTS, false).is_err());
    }
}
