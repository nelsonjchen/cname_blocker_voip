use std::collections::HashMap;
use std::env;
use std::net::{IpAddr, ToSocketAddrs};
use std::time::Duration;

use anyhow::{Context, Result, bail};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppConfig {
    pub username: String,
    pub password: String,
    pub host: String,
    pub port: u16,
    pub block_patterns: Vec<String>,
    pub block_regexes: Vec<String>,
    pub message_audio: Option<String>,
    pub rtp_port_min: u16,
    pub rtp_port_max: u16,
    pub register_expiry: Duration,
    pub register_retry: Duration,
    pub register_max_retry: u32,
    pub nat_keepalive_interval: Option<Duration>,
    pub nat: bool,
}

impl AppConfig {
    pub fn from_env() -> Result<Self> {
        Self::from_lookup(|key| env::var(key).ok())
    }

    pub fn from_lookup<F>(lookup: F) -> Result<Self>
    where
        F: Fn(&str) -> Option<String>,
    {
        let username = required(&lookup, "VOIPMS_USER")?;
        let password = required(&lookup, "VOIPMS_PASSWORD")?;
        let host = required(&lookup, "VOIPMS_HOST")?;

        let port = parse_or(&lookup, "VOIPMS_PORT", 5060)?;
        let rtp_port_min = parse_or(&lookup, "RTP_PORT_MIN", 0)?;
        let rtp_port_max = parse_or(&lookup, "RTP_PORT_MAX", 0)?;
        let register_expiry = Duration::from_secs(parse_or(&lookup, "REGISTER_EXPIRY_SECS", 60)?);
        let register_retry = Duration::from_secs(parse_or(&lookup, "REGISTER_RETRY_SECS", 5)?);
        let register_max_retry = parse_or(&lookup, "REGISTER_MAX_RETRY", 3)?;
        let nat_keepalive_interval = parse_optional_secs(&lookup, "NAT_KEEPALIVE_SECS")?;
        let nat = parse_bool(lookup("SIP_NAT").as_deref()).unwrap_or(true);

        if (rtp_port_min == 0) != (rtp_port_max == 0) {
            bail!("RTP_PORT_MIN and RTP_PORT_MAX must be set together or both omitted");
        }
        if rtp_port_min != 0 && rtp_port_min > rtp_port_max {
            bail!("RTP_PORT_MIN must be less than or equal to RTP_PORT_MAX");
        }

        Ok(Self {
            username,
            password,
            host,
            port,
            block_patterns: parse_patterns(lookup("BLOCK_CNAME_PATTERNS").as_deref()),
            block_regexes: parse_list(lookup("BLOCK_CNAME_REGEXES").as_deref()),
            message_audio: lookup("BLOCKER_MESSAGE_AUDIO").filter(|v| !v.trim().is_empty()),
            rtp_port_min,
            rtp_port_max,
            register_expiry,
            register_retry,
            register_max_retry,
            nat_keepalive_interval,
            nat,
        })
    }

    pub fn xphone_config(&self) -> Result<xphone::Config> {
        let (host, port) = self.resolve_sip_server()?;
        Ok(xphone::Config {
            username: self.username.clone(),
            password: self.password.clone(),
            host,
            port,
            register_expiry: self.register_expiry,
            register_retry: self.register_retry,
            register_max_retry: self.register_max_retry,
            nat_keepalive_interval: self.nat_keepalive_interval,
            nat: self.nat,
            rtp_port_min: self.rtp_port_min,
            rtp_port_max: self.rtp_port_max,
            user_agent: concat!("cname-blocker-voip/", env!("CARGO_PKG_VERSION")).into(),
            ..xphone::Config::default()
        })
    }

    fn resolve_sip_server(&self) -> Result<(String, u16)> {
        if self.host.parse::<IpAddr>().is_ok() {
            return Ok((self.host.clone(), self.port));
        }

        let mut addrs = (self.host.as_str(), self.port)
            .to_socket_addrs()
            .with_context(|| format!("failed to resolve VOIPMS_HOST={}", self.host))?;
        let addr = addrs
            .next()
            .with_context(|| format!("VOIPMS_HOST={} resolved to no addresses", self.host))?;
        Ok((addr.ip().to_string(), addr.port()))
    }
}

pub fn lookup_from_map(map: HashMap<&str, &str>) -> impl Fn(&str) -> Option<String> {
    move |key| map.get(key).map(|value| (*value).to_string())
}

fn required<F>(lookup: &F, key: &str) -> Result<String>
where
    F: Fn(&str) -> Option<String>,
{
    lookup(key)
        .filter(|value| !value.trim().is_empty())
        .with_context(|| format!("{key} is required"))
}

fn parse_or<F, T>(lookup: &F, key: &str, default: T) -> Result<T>
where
    F: Fn(&str) -> Option<String>,
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    match lookup(key) {
        Some(value) if !value.trim().is_empty() => value
            .trim()
            .parse::<T>()
            .map_err(|err| anyhow::anyhow!("{key} must be a valid value: {err}")),
        _ => Ok(default),
    }
}

fn parse_optional_secs<F>(lookup: &F, key: &str) -> Result<Option<Duration>>
where
    F: Fn(&str) -> Option<String>,
{
    match lookup(key) {
        Some(value) if !value.trim().is_empty() => {
            let seconds = value
                .trim()
                .parse::<u64>()
                .with_context(|| format!("{key} must be a number of seconds"))?;
            if seconds == 0 {
                Ok(None)
            } else {
                Ok(Some(Duration::from_secs(seconds)))
            }
        }
        _ => Ok(Some(Duration::from_secs(15))),
    }
}

fn parse_bool(value: Option<&str>) -> Option<bool> {
    match value?.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

pub fn parse_patterns(value: Option<&str>) -> Vec<String> {
    let source = value.unwrap_or("pch");
    let patterns = parse_list(Some(source))
        .into_iter()
        .map(|pattern| pattern.to_ascii_lowercase())
        .collect::<Vec<_>>();

    if patterns.is_empty() {
        vec!["pch".to_string()]
    } else {
        patterns
    }
}

pub fn parse_list(value: Option<&str>) -> Vec<String> {
    value
        .unwrap_or_default()
        .split(',')
        .map(|pattern| pattern.trim().to_string())
        .filter(|pattern| !pattern.is_empty())
        .collect::<Vec<_>>()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config_with(extra: &[(&str, &str)]) -> AppConfig {
        let mut map = HashMap::from([
            ("VOIPMS_USER", "123456_blocker"),
            ("VOIPMS_PASSWORD", "secret"),
            ("VOIPMS_HOST", "losangeles1.voip.ms"),
        ]);
        for (key, value) in extra {
            map.insert(*key, *value);
        }
        AppConfig::from_lookup(lookup_from_map(map)).unwrap()
    }

    #[test]
    fn defaults_to_pch_and_voipms_port() {
        let config = config_with(&[]);
        assert_eq!(config.port, 5060);
        assert_eq!(config.block_patterns, vec!["pch"]);
        assert!(config.block_regexes.is_empty());
        assert_eq!(config.nat_keepalive_interval, Some(Duration::from_secs(15)));
    }

    #[test]
    fn parses_case_insensitive_pattern_list() {
        let config = config_with(&[("BLOCK_CNAME_PATTERNS", " Nelson, PCH ,,")]);
        assert_eq!(config.block_patterns, vec!["nelson", "pch"]);
    }

    #[test]
    fn parses_case_sensitive_regex_list_without_lowercasing() {
        let config = config_with(&[("BLOCK_CNAME_REGEXES", r"[[:alpha:]] CA$, ^PCH\\b ,,")]);
        assert_eq!(config.block_regexes, vec![r"[[:alpha:]] CA$", r"^PCH\\b"]);
    }

    #[test]
    fn rejects_partial_rtp_range() {
        let mut map = HashMap::from([
            ("VOIPMS_USER", "123456_blocker"),
            ("VOIPMS_PASSWORD", "secret"),
            ("VOIPMS_HOST", "losangeles1.voip.ms"),
            ("RTP_PORT_MIN", "30000"),
        ]);
        let err = AppConfig::from_lookup(lookup_from_map(map.clone())).unwrap_err();
        assert!(err.to_string().contains("RTP_PORT_MIN"));
        map.insert("RTP_PORT_MAX", "29999");
        let err = AppConfig::from_lookup(lookup_from_map(map)).unwrap_err();
        assert!(err.to_string().contains("less than or equal"));
    }
}
