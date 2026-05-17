use std::sync::Arc;
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use tracing::{error, info, warn};
use xphone::{Call, Phone};

use crate::audio::DisconnectAudio;
use crate::config::AppConfig;
use crate::matcher::PatternMatcher;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallDecision {
    Block,
    Cascade,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallFacts {
    pub caller_name: String,
    pub from_headers: Vec<String>,
}

#[derive(Clone)]
pub struct CnameBlocker {
    matcher: PatternMatcher,
    audio: DisconnectAudio,
}

impl CnameBlocker {
    pub fn new(patterns: Vec<String>, audio: DisconnectAudio) -> Self {
        Self {
            matcher: PatternMatcher::new(patterns),
            audio,
        }
    }

    pub fn decision(&self, facts: &CallFacts) -> CallDecision {
        if self
            .matcher
            .is_match(&facts.caller_name, &facts.from_headers)
        {
            CallDecision::Block
        } else {
            CallDecision::Cascade
        }
    }

    pub fn install_on(&self, phone: &Phone) {
        let blocker = self.clone();
        phone.on_incoming(move |call| {
            let blocker = blocker.clone();
            thread::spawn(move || blocker.handle_call(call));
        });
    }

    fn handle_call(&self, call: Arc<Call>) {
        let facts = CallFacts::from_call(&call);
        match self.decision(&facts) {
            CallDecision::Block => {
                info!(
                    caller_name = %facts.caller_name,
                    from = ?facts.from_headers,
                    "blocking call by CNAME"
                );
                if let Err(err) = self.answer_play_and_hangup(call) {
                    error!(error = %err, "failed to complete blocked-call audio flow");
                }
            }
            CallDecision::Cascade => {
                info!(
                    caller_name = %facts.caller_name,
                    from = ?facts.from_headers,
                    "non-matching call; returning 486 Busy Here for call hunting"
                );
                if let Err(err) = call.reject(486, "Busy Here") {
                    warn!(error = %err, "failed to reject non-matching call");
                }
            }
        }
    }

    fn answer_play_and_hangup(&self, call: Arc<Call>) -> Result<()> {
        call.accept().context("accept failed")?;
        let samples = self.audio.samples();

        if let Some(writer) = call.paced_pcm_writer() {
            writer
                .send((*samples).clone())
                .context("failed to queue disconnect audio")?;
            thread::sleep(self.audio.duration() + Duration::from_millis(350));
        } else {
            warn!("call accepted but no PCM writer was available; hanging up after a short pause");
            thread::sleep(Duration::from_secs(2));
        }

        call.end().context("hangup failed")?;
        Ok(())
    }

    pub fn matcher(&self) -> &PatternMatcher {
        &self.matcher
    }
}

impl CallFacts {
    pub fn from_call(call: &Call) -> Self {
        Self {
            caller_name: call.from_name(),
            from_headers: call.header("From"),
        }
    }
}

pub fn run(config: AppConfig, shutdown: impl FnOnce() + Send + 'static) -> Result<()> {
    let audio = DisconnectAudio::load(config.message_audio.as_deref())?;
    let blocker = CnameBlocker::new(config.block_patterns.clone(), audio);
    let phone = Phone::new(config.xphone_config());
    blocker.install_on(&phone);

    phone.on_registered(|| {
        info!("registered and waiting for inbound calls");
    });
    phone.on_unregistered(|| {
        warn!("unregistered from SIP server");
    });

    phone
        .connect()
        .context("failed to connect/register SIP phone")?;
    shutdown();
    phone.disconnect().ok();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nelson_is_blocked_when_configured() {
        let audio = DisconnectAudio::load(None).unwrap();
        let blocker = CnameBlocker::new(vec!["nelson".into()], audio);
        let facts = CallFacts {
            caller_name: "Nelson".into(),
            from_headers: vec![],
        };
        assert_eq!(blocker.decision(&facts), CallDecision::Block);
    }

    #[test]
    fn non_matching_call_cascades() {
        let audio = DisconnectAudio::load(None).unwrap();
        let blocker = CnameBlocker::new(vec!["pch".into()], audio);
        let facts = CallFacts {
            caller_name: "Nelson".into(),
            from_headers: vec!["\"Nelson\" <sip:+15551212@example.test>".into()],
        };
        assert_eq!(blocker.decision(&facts), CallDecision::Cascade);
    }
}
