//! Origin normalization: the one function both the vault (when saving a
//! site) and the login engine (before every fill) use, so the two sides
//! always compare strings built the same way.

use crate::{Result, VaultError};

/// Returns `scheme://host:port` for an `http` or `https` URL, always with
/// an explicit port, so `https://x` and `https://x:443` give the same
/// string. The `url` crate lowercases the host and puts IPv6 addresses in
/// brackets.
pub fn normalize_origin(raw: &str) -> Result<String> {
    let url = url::Url::parse(raw.trim()).map_err(|e| VaultError::InvalidInput(format!("login URL: {e}")))?;
    let scheme = url.scheme();
    if scheme != "http" && scheme != "https" {
        return Err(VaultError::InvalidInput(format!("login URL must be http or https, not {scheme}")));
    }
    let host = url.host().ok_or_else(|| VaultError::InvalidInput("login URL has no host".into()))?;
    let port = url
        .port_or_known_default()
        .ok_or_else(|| VaultError::InvalidInput("login URL has no port".into()))?;
    // `Host`'s Display writes IPv6 addresses with brackets, domains in
    // lowercase ASCII (punycode for international names).
    Ok(format!("{scheme}://{host}:{port}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_default_ports_and_lowercases() {
        assert_eq!(normalize_origin("https://UniFi.Local/login").unwrap(), "https://unifi.local:443");
        assert_eq!(normalize_origin("http://Router/").unwrap(), "http://router:80");
        assert_eq!(normalize_origin("https://10.100.100.18:8443/manage").unwrap(), "https://10.100.100.18:8443");
    }

    #[test]
    fn same_origin_with_or_without_default_port() {
        assert_eq!(normalize_origin("https://x.example").unwrap(), normalize_origin("https://x.example:443/a?b").unwrap());
        assert_ne!(normalize_origin("https://x.example").unwrap(), normalize_origin("http://x.example").unwrap());
        assert_ne!(normalize_origin("https://x.example").unwrap(), normalize_origin("https://x.example:8443").unwrap());
    }

    #[test]
    fn keeps_ipv6_brackets() {
        assert_eq!(normalize_origin("https://[::1]:8006/").unwrap(), "https://[::1]:8006");
    }

    #[test]
    fn ignores_credentials_in_the_url() {
        assert_eq!(normalize_origin("https://user:pw@host.example/").unwrap(), "https://host.example:443");
    }

    #[test]
    fn rejects_other_schemes_and_garbage() {
        assert!(normalize_origin("ftp://host/").is_err());
        assert!(normalize_origin("file:///C:/x").is_err());
        assert!(normalize_origin("not a url").is_err());
        assert!(normalize_origin("").is_err());
    }
}
