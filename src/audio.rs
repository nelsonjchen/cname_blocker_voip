use std::f32::consts::TAU;
use std::fs::File;
use std::io::{Cursor, Read, Seek};
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result, bail};

pub const SAMPLE_RATE: u32 = 8_000;
const DEFAULT_MESSAGE_OGG: &[u8] = include_bytes!("../assets/disconnected.ogg");

#[derive(Debug, Clone)]
pub struct DisconnectAudio {
    samples: Arc<Vec<i16>>,
}

impl DisconnectAudio {
    pub fn load(path: Option<&str>) -> Result<Self> {
        let mut message = match path {
            Some(path) => load_audio_file(Path::new(path))
                .with_context(|| format!("failed to load BLOCKER_MESSAGE_AUDIO={path}"))?,
            None => decode_ogg_opus(Cursor::new(DEFAULT_MESSAGE_OGG))
                .context("failed to decode built-in disconnected.ogg")?,
        };

        trim_leading_silence(&mut message);
        let mut samples = generate_sit_tones();
        samples.extend(silence_ms(160));
        samples.extend(message);
        Ok(Self {
            samples: Arc::new(samples),
        })
    }

    pub fn samples(&self) -> Arc<Vec<i16>> {
        Arc::clone(&self.samples)
    }

    pub fn duration(&self) -> std::time::Duration {
        std::time::Duration::from_secs_f64(self.samples.len() as f64 / SAMPLE_RATE as f64)
    }
}

pub fn load_audio_file(path: &Path) -> Result<Vec<i16>> {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase())
        .as_deref()
    {
        Some("ogg") | Some("opus") => {
            let file = File::open(path)?;
            decode_ogg_opus(file)
        }
        Some("wav") => load_wav(path),
        Some(other) => bail!("unsupported audio extension .{other}; use .ogg, .opus, or .wav"),
        None => bail!("audio path must have an extension"),
    }
}

pub fn decode_ogg_opus<T>(reader: T) -> Result<Vec<i16>>
where
    T: Read + Seek,
{
    let (samples, play_data) = ogg_opus::decode::<_, SAMPLE_RATE>(reader)?;
    Ok(downmix_interleaved(samples, play_data.channels as usize))
}

pub fn load_wav(path: &Path) -> Result<Vec<i16>> {
    let mut reader = hound::WavReader::open(path)?;
    let spec = reader.spec();
    let channels = spec.channels as usize;
    if channels == 0 {
        bail!("WAV has zero channels");
    }

    let raw = match spec.sample_format {
        hound::SampleFormat::Int if spec.bits_per_sample <= 16 => reader
            .samples::<i16>()
            .collect::<Result<Vec<_>, _>>()
            .context("failed to read 16-bit WAV samples")?,
        hound::SampleFormat::Int => {
            let shift = spec.bits_per_sample.saturating_sub(16);
            reader
                .samples::<i32>()
                .map(|sample| {
                    sample.map(|value| {
                        (value >> shift).clamp(i16::MIN as i32, i16::MAX as i32) as i16
                    })
                })
                .collect::<Result<Vec<_>, _>>()
                .context("failed to read integer WAV samples")?
        }
        hound::SampleFormat::Float => reader
            .samples::<f32>()
            .map(|sample| {
                sample.map(|value| {
                    (value.clamp(-1.0, 1.0) * i16::MAX as f32)
                        .round()
                        .clamp(i16::MIN as f32, i16::MAX as f32) as i16
                })
            })
            .collect::<Result<Vec<_>, _>>()
            .context("failed to read float WAV samples")?,
    };

    let mono = downmix_interleaved(raw, channels);
    Ok(resample_linear(&mono, spec.sample_rate, SAMPLE_RATE))
}

pub fn generate_sit_tones() -> Vec<i16> {
    let mut samples = Vec::new();
    for frequency in [913.8_f32, 1370.6, 1776.7] {
        samples.extend(sine_tone(frequency, 330, 0.42));
        samples.extend(silence_ms(35));
    }
    samples
}

pub fn silence_ms(duration_ms: u32) -> Vec<i16> {
    vec![0; samples_for_ms(duration_ms)]
}

fn sine_tone(frequency: f32, duration_ms: u32, amplitude: f32) -> Vec<i16> {
    let count = samples_for_ms(duration_ms);
    (0..count)
        .map(|index| {
            let t = index as f32 / SAMPLE_RATE as f32;
            let sample = (TAU * frequency * t).sin() * amplitude * i16::MAX as f32;
            sample.round().clamp(i16::MIN as f32, i16::MAX as f32) as i16
        })
        .collect()
}

fn samples_for_ms(duration_ms: u32) -> usize {
    ((SAMPLE_RATE as u64 * duration_ms as u64) / 1000) as usize
}

fn downmix_interleaved(samples: Vec<i16>, channels: usize) -> Vec<i16> {
    if channels <= 1 {
        return samples;
    }

    samples
        .chunks(channels)
        .map(|frame| {
            let sum = frame.iter().map(|sample| *sample as i32).sum::<i32>();
            (sum / frame.len() as i32).clamp(i16::MIN as i32, i16::MAX as i32) as i16
        })
        .collect()
}

pub fn resample_linear(samples: &[i16], from_rate: u32, to_rate: u32) -> Vec<i16> {
    if samples.is_empty() || from_rate == to_rate {
        return samples.to_vec();
    }

    let out_len = ((samples.len() as u64 * to_rate as u64) / from_rate as u64).max(1) as usize;
    let ratio = from_rate as f64 / to_rate as f64;
    (0..out_len)
        .map(|index| {
            let source_pos = index as f64 * ratio;
            let left = source_pos.floor() as usize;
            let right = (left + 1).min(samples.len() - 1);
            let frac = source_pos - left as f64;
            let value = samples[left] as f64 * (1.0 - frac) + samples[right] as f64 * frac;
            value.round().clamp(i16::MIN as f64, i16::MAX as f64) as i16
        })
        .collect()
}

fn trim_leading_silence(samples: &mut Vec<i16>) {
    let Some(index) = samples.iter().position(|sample| sample.unsigned_abs() > 96) else {
        return;
    };
    if index > 0 {
        samples.drain(0..index);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sit_tones_have_expected_duration_and_signal() {
        let samples = generate_sit_tones();
        let expected = samples_for_ms((330 + 35) * 3);
        assert_eq!(samples.len(), expected);
        assert!(samples.iter().any(|sample| *sample > 10_000));
        assert!(samples.iter().any(|sample| *sample < -10_000));
    }

    #[test]
    fn built_in_ogg_decodes_to_telephony_pcm() {
        let audio = DisconnectAudio::load(None).unwrap();
        assert!(audio.samples().len() > SAMPLE_RATE as usize);
        assert!(audio.duration() > std::time::Duration::from_secs(1));
    }

    #[test]
    fn resampler_changes_length() {
        let samples = vec![0_i16; 48_000];
        let resampled = resample_linear(&samples, 48_000, SAMPLE_RATE);
        assert_eq!(resampled.len(), SAMPLE_RATE as usize);
    }
}
