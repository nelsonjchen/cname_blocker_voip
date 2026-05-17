use std::sync::mpsc;

use anyhow::{Result, bail};
use cname_blocker_voip::{AppConfig, blocker};
use tracing_subscriber::EnvFilter;

fn main() -> Result<()> {
    load_dotenv()?;

    tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_env_filter(EnvFilter::from_default_env().add_directive("info".parse()?))
        .init();

    let config = AppConfig::from_env()?;
    let (shutdown_tx, shutdown_rx) = mpsc::channel::<()>();
    ctrlc::set_handler(move || {
        let _ = shutdown_tx.send(());
    })?;

    blocker::run(config, move || {
        let _ = shutdown_rx.recv();
    })
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
