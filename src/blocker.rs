use std::sync::Arc;
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use tracing::{error, info, warn};
use xphone::{Call, Phone};

use crate::audio::DisconnectAudio;
use crate::config::AppConfig;
use crate::matcher::PatternMatcher;
use crate::twilio::TwilioLookup;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallDecision {
    Block,
    Cascade,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallFacts {
    pub caller_name: String,
    pub caller_number: String,
    pub from_headers: Vec<String>,
    pub name_source: CallerNameSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallerNameSource {
    Sip,
    Twilio,
    TwilioUnavailable,
}

#[derive(Clone)]
pub struct CnameBlocker {
    matcher: PatternMatcher,
    audio: DisconnectAudio,
    twilio_lookup: Option<TwilioLookup>,
}

impl CnameBlocker {
    pub fn new(patterns: Vec<String>, audio: DisconnectAudio) -> Self {
        Self::try_new(patterns, vec![], audio).expect("empty regex list must compile")
    }

    pub fn try_new(
        patterns: Vec<String>,
        regexes: Vec<String>,
        audio: DisconnectAudio,
    ) -> Result<Self> {
        let matcher = PatternMatcher::try_new(patterns, regexes)?;
        Ok(Self {
            matcher,
            audio,
            twilio_lookup: None,
        })
    }

    fn from_matcher(
        matcher: PatternMatcher,
        audio: DisconnectAudio,
        twilio_lookup: Option<TwilioLookup>,
    ) -> Self {
        Self {
            matcher,
            audio,
            twilio_lookup,
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
        let facts = self.call_facts(&call);
        match self.decision(&facts) {
            CallDecision::Block => {
                info!(
                    caller_name = %facts.caller_name,
                    caller_number = %facts.caller_number,
                    source = ?facts.name_source,
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
                    caller_number = %facts.caller_number,
                    source = ?facts.name_source,
                    from = ?facts.from_headers,
                    "non-matching call; returning 486 Busy Here for call hunting"
                );
                if let Err(err) = call.reject(486, "Busy Here") {
                    warn!(error = %err, "failed to reject non-matching call");
                }
            }
        }
    }

    fn call_facts(&self, call: &Call) -> CallFacts {
        let upstream = CallFacts::from_call(call);
        let Some(twilio_lookup) = &self.twilio_lookup else {
            return upstream;
        };

        match twilio_lookup.lookup_caller_name(&upstream.caller_number) {
            Ok(Some(caller_name)) => CallFacts {
                caller_name,
                caller_number: upstream.caller_number,
                from_headers: Vec::new(),
                name_source: CallerNameSource::Twilio,
            },
            Ok(None) => {
                warn!(
                    caller_number = %upstream.caller_number,
                    upstream_caller_name = %upstream.caller_name,
                    "Twilio Lookup returned no caller name; not using upstream CNAME for matching"
                );
                CallFacts {
                    caller_name: String::new(),
                    caller_number: upstream.caller_number,
                    from_headers: Vec::new(),
                    name_source: CallerNameSource::TwilioUnavailable,
                }
            }
            Err(err) => {
                warn!(
                    caller_number = %upstream.caller_number,
                    upstream_caller_name = %upstream.caller_name,
                    error = %err,
                    "Twilio Lookup failed; not using upstream CNAME for matching"
                );
                CallFacts {
                    caller_name: String::new(),
                    caller_number: upstream.caller_number,
                    from_headers: Vec::new(),
                    name_source: CallerNameSource::TwilioUnavailable,
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
            caller_number: call.from(),
            from_headers: call.header("From"),
            name_source: CallerNameSource::Sip,
        }
    }
}

pub fn run(config: AppConfig, shutdown: impl FnOnce() + Send + 'static) -> Result<()> {
    let audio = DisconnectAudio::load(config.message_audio.as_deref())?;
    let matcher =
        PatternMatcher::try_new(config.block_patterns.clone(), config.block_regexes.clone())?;
    let twilio_lookup = config
        .twilio_lookup
        .clone()
        .map(TwilioLookup::new)
        .transpose()?;
    let blocker = CnameBlocker::from_matcher(matcher, audio, twilio_lookup);
    let phone = Phone::new(config.xphone_config()?);
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
            caller_number: "+15551212".into(),
            from_headers: vec![],
            name_source: CallerNameSource::Sip,
        };
        assert_eq!(blocker.decision(&facts), CallDecision::Block);
    }

    #[test]
    fn non_matching_call_cascades() {
        let audio = DisconnectAudio::load(None).unwrap();
        let blocker = CnameBlocker::new(vec!["pch".into()], audio);
        let facts = CallFacts {
            caller_name: "Nelson".into(),
            caller_number: "+15551212".into(),
            from_headers: vec!["\"Nelson\" <sip:+15551212@example.test>".into()],
            name_source: CallerNameSource::Sip,
        };
        assert_eq!(blocker.decision(&facts), CallDecision::Cascade);
    }
}
