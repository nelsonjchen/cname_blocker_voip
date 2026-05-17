use std::{env, sync::mpsc};

use anyhow::{Result, bail};
use cname_blocker_voip::{AppConfig, blocker};
use tracing_subscriber::EnvFilter;

fn main() -> Result<()> {
    load_dotenv()?;
    init_logging()?;

    let config = AppConfig::from_env()?;
    let (shutdown_tx, shutdown_rx) = mpsc::channel::<()>();
    ctrlc::set_handler(move || {
        let _ = shutdown_tx.send(());
    })?;

    blocker::run(config, move || {
        let _ = shutdown_rx.recv();
    })
}

fn init_logging() -> Result<()> {
    let ansi = env_bool("LOG_ANSI", false)?;
    let timestamps = env_bool("LOG_TIMESTAMPS", false)?;
    let filter = EnvFilter::from_default_env().add_directive("info".parse()?);
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(ansi)
        .with_env_filter(filter);

    if timestamps {
        subscriber.init();
    } else {
        subscriber.without_time().init();
    }

    Ok(())
}

fn load_dotenv() -> Result<()> {
    match dotenvy::dotenv() {
        Ok(_) => Ok(()),
        Err(err) if err.not_found() => Ok(()),
        Err(dotenvy::Error::LineParse(line, index)) => {
            bail!(
                "failed to parse .env near column {index}: {}",
                redact_env_line(&line)
            )
        }
        Err(err) => bail!("failed to load .env: {err}"),
    }
}

fn redact_env_line(line: &str) -> String {
    let Some((key, _)) = line.split_once('=') else {
        return "<malformed line>".into();
    };
    format!("{}=<redacted>", key.trim())
}

fn env_bool(key: &str, default: bool) -> Result<bool> {
    let Ok(value) = env::var(key) else {
        return Ok(default);
    };

    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => bail!("{key} must be true/false, yes/no, on/off, or 1/0"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_env_line_values() {
        assert_eq!(
            redact_env_line("VOIPMS_PASSWORD=secret"),
            "VOIPMS_PASSWORD=<redacted>"
        );
        assert_eq!(redact_env_line("not-an-assignment"), "<malformed line>");
    }
}
