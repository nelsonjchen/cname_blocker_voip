use std::sync::mpsc;

use anyhow::Result;
use cname_blocker_voip::{AppConfig, blocker};
use tracing_subscriber::EnvFilter;

fn main() -> Result<()> {
    tracing_subscriber::fmt()
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
