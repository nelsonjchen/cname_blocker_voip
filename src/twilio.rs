use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use serde::Deserialize;
use tracing::warn;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TwilioLookupConfig {
    pub api_key_sid: String,
    pub api_key_secret: String,
    pub timeout: Duration,
}

#[derive(Clone)]
pub struct TwilioLookup {
    config: TwilioLookupConfig,
    client: reqwest::blocking::Client,
    cache: Arc<Mutex<HashMap<String, Option<String>>>>,
}

impl TwilioLookup {
    pub fn new(config: TwilioLookupConfig) -> Result<Self> {
        let client = reqwest::blocking::Client::builder()
            .timeout(config.timeout)
            .build()
            .context("failed to build Twilio Lookup HTTP client")?;
        Ok(Self {
            config,
            client,
            cache: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    pub fn lookup_caller_name(&self, raw_number: &str) -> Result<Option<String>> {
        let Some(phone_number) = normalize_phone_number(raw_number) else {
            return Ok(None);
        };

        if let Some(cached) = self
            .cache
            .lock()
            .expect("lookup cache poisoned")
            .get(&phone_number)
        {
            return Ok(cached.clone());
        }

        let encoded = urlencoding::encode(&phone_number);
        let url = format!("https://lookups.twilio.com/v2/PhoneNumbers/{encoded}");
        let response = self
            .client
            .get(url)
            .basic_auth(&self.config.api_key_sid, Some(&self.config.api_key_secret))
            .query(&[("Fields", "caller_name")])
            .send()
            .context("Twilio Lookup request failed")?;

        if !response.status().is_success() {
            warn!(status = %response.status(), "Twilio Lookup returned non-success status");
            self.cache
                .lock()
                .expect("lookup cache poisoned")
                .insert(phone_number, None);
            return Ok(None);
        }

        let body = response
            .json::<TwilioLookupResponse>()
            .context("failed to parse Twilio Lookup response")?;
        let caller_name = body.caller_name.and_then(|caller_name| {
            caller_name
                .caller_name
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        });

        self.cache
            .lock()
            .expect("lookup cache poisoned")
            .insert(phone_number, caller_name.clone());
        Ok(caller_name)
    }
}

pub fn normalize_phone_number(raw_number: &str) -> Option<String> {
    let trimmed = raw_number.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Some(rest) = trimmed.strip_prefix('+') {
        let digits = only_digits(rest);
        return (!digits.is_empty()).then(|| format!("+{digits}"));
    }

    let digits = only_digits(trimmed);
    match digits.len() {
        10 => Some(format!("+1{digits}")),
        11 if digits.starts_with('1') => Some(format!("+{digits}")),
        _ => None,
    }
}

fn only_digits(value: &str) -> String {
    value.chars().filter(|ch| ch.is_ascii_digit()).collect()
}

#[derive(Debug, Deserialize)]
struct TwilioLookupResponse {
    caller_name: Option<TwilioCallerName>,
}

#[derive(Debug, Deserialize)]
struct TwilioCallerName {
    caller_name: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_us_phone_numbers() {
        assert_eq!(
            normalize_phone_number("6267233427"),
            Some("+16267233427".into())
        );
        assert_eq!(
            normalize_phone_number("1 626 723 3427"),
            Some("+16267233427".into())
        );
        assert_eq!(
            normalize_phone_number("+1 (626) 723-3427"),
            Some("+16267233427".into())
        );
        assert_eq!(normalize_phone_number("abc"), None);
    }

    #[test]
    fn parses_twilio_caller_name_response() {
        let response: TwilioLookupResponse = serde_json::from_str(
            r#"{
                "caller_name": {
                    "caller_name": "PCH CLAIMS",
                    "caller_type": "CONSUMER"
                }
            }"#,
        )
        .unwrap();
        assert_eq!(
            response.caller_name.unwrap().caller_name.as_deref(),
            Some("PCH CLAIMS")
        );
    }
}
