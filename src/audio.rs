use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use hound::{SampleFormat, WavSpec, WavWriter};
use std::io::Cursor;
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use std::sync::{Arc, Mutex};

const MODEL_AUDIO_SAMPLE_RATE: u32 = 24_000;
const MODEL_AUDIO_CHANNELS: u16 = 1;

pub struct RecordedAudio {
    pub data: Vec<i16>,
    pub sample_rate: u32,
    pub channels: u16,
}

pub struct VoiceCapture {
    stream: Option<cpal::Stream>,
    sample_rate: u32,
    channels: u16,
    data: Arc<Mutex<Vec<i16>>>,
    stopped: Arc<AtomicBool>,
    last_peak: Arc<AtomicU16>,
}

impl VoiceCapture {
    pub fn start() -> Result<Self, String> {
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .ok_or_else(|| "no input audio device available".to_string())?;

        let config = preferred_input_config(&device)?;
        let sample_rate = config.sample_rate().0;
        let channels = config.channels();
        let data: Arc<Mutex<Vec<i16>>> = Arc::new(Mutex::new(Vec::new()));
        let stopped = Arc::new(AtomicBool::new(false));
        let last_peak = Arc::new(AtomicU16::new(0));

        let stream = build_input_stream(&device, &config, data.clone(), last_peak.clone())?;
        stream
            .play()
            .map_err(|e| format!("failed to start input stream: {e}"))?;

        Ok(Self {
            stream: Some(stream),
            sample_rate,
            channels,
            data,
            stopped,
            last_peak,
        })
    }

    pub fn stop(mut self) -> Result<RecordedAudio, String> {
        self.stopped.store(true, Ordering::SeqCst);
        self.stream.take();
        let data = self
            .data
            .lock()
            .map_err(|_| "failed to lock audio buffer".to_string())?
            .clone();
        Ok(RecordedAudio {
            data,
            sample_rate: self.sample_rate,
            channels: self.channels,
        })
    }

    pub fn last_peak(&self) -> u16 {
        self.last_peak.load(Ordering::Relaxed)
    }

}

fn preferred_input_config(
    device: &cpal::Device,
) -> Result<cpal::SupportedStreamConfig, String> {
    let supported_configs = device
        .supported_input_configs()
        .map_err(|err| format!("failed to enumerate input audio configs: {err}"))?;

    supported_configs
        .filter_map(|range| {
            let sample_format_rank = match range.sample_format() {
                cpal::SampleFormat::I16 => 0u8,
                cpal::SampleFormat::U16 => 1u8,
                cpal::SampleFormat::F32 => 2u8,
                _ => return None,
            };
            let sample_rate = preferred_sample_rate(&range);
            let sample_rate_penalty = sample_rate.0.abs_diff(MODEL_AUDIO_SAMPLE_RATE);
            let channel_penalty = range.channels().abs_diff(MODEL_AUDIO_CHANNELS);
            Some((
                (sample_rate_penalty, channel_penalty, sample_format_rank),
                range.with_sample_rate(sample_rate),
            ))
        })
        .min_by_key(|(score, _)| *score)
        .map(|(_, config)| config)
        .or_else(|| device.default_input_config().ok())
        .ok_or_else(|| "failed to get default input config".to_string())
}

fn preferred_sample_rate(range: &cpal::SupportedStreamConfigRange) -> cpal::SampleRate {
    let min = range.min_sample_rate().0;
    let max = range.max_sample_rate().0;
    if (min..=max).contains(&MODEL_AUDIO_SAMPLE_RATE) {
        cpal::SampleRate(MODEL_AUDIO_SAMPLE_RATE)
    } else if MODEL_AUDIO_SAMPLE_RATE < min {
        cpal::SampleRate(min)
    } else {
        cpal::SampleRate(max)
    }
}

fn build_input_stream(
    device: &cpal::Device,
    config: &cpal::SupportedStreamConfig,
    data: Arc<Mutex<Vec<i16>>>,
    last_peak: Arc<AtomicU16>,
) -> Result<cpal::Stream, String> {
    match config.sample_format() {
        cpal::SampleFormat::F32 => device
            .build_input_stream(
                &config.clone().into(),
                move |input: &[f32], _| {
                    let peak = peak_f32(input);
                    last_peak.store(peak, Ordering::Relaxed);
                    if let Ok(mut buf) = data.lock() {
                        for &s in input {
                            buf.push(f32_to_i16(s));
                        }
                    }
                },
                move |err| eprintln!("audio input error: {err}"),
                None,
            )
            .map_err(|e| format!("failed to build input stream: {e}")),
        cpal::SampleFormat::I16 => device
            .build_input_stream(
                &config.clone().into(),
                move |input: &[i16], _| {
                    let peak = peak_i16(input);
                    last_peak.store(peak, Ordering::Relaxed);
                    if let Ok(mut buf) = data.lock() {
                        buf.extend_from_slice(input);
                    }
                },
                move |err| eprintln!("audio input error: {err}"),
                None,
            )
            .map_err(|e| format!("failed to build input stream: {e}")),
        cpal::SampleFormat::U16 => device
            .build_input_stream(
                &config.clone().into(),
                move |input: &[u16], _| {
                    if let Ok(mut buf) = data.lock() {
                        let peak = convert_u16_to_i16_and_peak(input, &mut buf);
                        last_peak.store(peak, Ordering::Relaxed);
                    }
                },
                move |err| eprintln!("audio input error: {err}"),
                None,
            )
            .map_err(|e| format!("failed to build input stream: {e}")),
        _ => Err("unsupported input sample format".to_string()),
    }
}

#[inline]
fn f32_to_i16(s: f32) -> i16 {
    (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16
}

fn peak_f32(input: &[f32]) -> u16 {
    let mut peak: f32 = 0.0;
    for &s in input {
        let a = s.abs();
        if a > peak {
            peak = a;
        }
    }
    (peak.min(1.0) * i16::MAX as f32) as u16
}

fn peak_i16(input: &[i16]) -> u16 {
    let mut peak: i32 = 0;
    for &s in input {
        let a = (s as i32).unsigned_abs() as i32;
        if a > peak {
            peak = a;
        }
    }
    peak as u16
}

fn convert_u16_to_i16_and_peak(input: &[u16], out: &mut Vec<i16>) -> u16 {
    let mut peak: i32 = 0;
    for &s in input {
        let v_i16 = (s as i32 - 32768) as i16;
        let a = (v_i16 as i32).unsigned_abs() as i32;
        if a > peak {
            peak = a;
        }
        out.push(v_i16);
    }
    peak as u16
}

pub fn convert_pcm16(
    input: &[i16],
    input_sample_rate: u32,
    input_channels: u16,
    output_sample_rate: u32,
    output_channels: u16,
) -> Vec<i16> {
    if input.is_empty() || input_channels == 0 || output_channels == 0 {
        return Vec::new();
    }

    let in_channels = input_channels as usize;
    let out_channels = output_channels as usize;
    let in_frames = input.len() / in_channels;
    if in_frames == 0 {
        return Vec::new();
    }

    let out_frames = if input_sample_rate == output_sample_rate {
        in_frames
    } else {
        (((in_frames as u64) * (output_sample_rate as u64)) / (input_sample_rate as u64)).max(1)
            as usize
    };

    let mut out = Vec::with_capacity(out_frames.saturating_mul(out_channels));
    for out_frame_idx in 0..out_frames {
        let src_frame_idx = if out_frames <= 1 || in_frames <= 1 {
            0
        } else {
            ((out_frame_idx as u64) * ((in_frames - 1) as u64) / ((out_frames - 1) as u64))
                as usize
        };
        let src_start = src_frame_idx.saturating_mul(in_channels);
        let src = &input[src_start..src_start + in_channels];
        match (in_channels, out_channels) {
            (1, 1) => out.push(src[0]),
            (1, n) => {
                for _ in 0..n {
                    out.push(src[0]);
                }
            }
            (n, 1) if n >= 2 => {
                let sum: i32 = src.iter().map(|s| *s as i32).sum();
                out.push((sum / (n as i32)) as i16);
            }
            (n, m) if n == m => out.extend_from_slice(src),
            (_n, m) => out.extend_from_slice(&src[..m]),
        }
    }
    out
}

pub fn clip_duration_seconds(audio: &RecordedAudio) -> f32 {
    let total_samples = audio.data.len() as f32;
    let samples_per_second = (audio.sample_rate as f32) * (audio.channels as f32);
    if samples_per_second > 0.0 {
        total_samples / samples_per_second
    } else {
        0.0
    }
}

pub fn encode_wav_normalized(audio: &RecordedAudio) -> Result<Vec<u8>, String> {
    let converted;
    let (channels, sample_rate, segment) =
        if audio.channels == MODEL_AUDIO_CHANNELS && audio.sample_rate == MODEL_AUDIO_SAMPLE_RATE {
            (audio.channels, audio.sample_rate, audio.data.as_slice())
        } else {
            converted = convert_pcm16(
                &audio.data,
                audio.sample_rate,
                audio.channels,
                MODEL_AUDIO_SAMPLE_RATE,
                MODEL_AUDIO_CHANNELS,
            );
            (
                MODEL_AUDIO_CHANNELS,
                MODEL_AUDIO_SAMPLE_RATE,
                converted.as_slice(),
            )
        };

    let mut wav_bytes: Vec<u8> = Vec::new();
    let spec = WavSpec {
        channels,
        sample_rate,
        bits_per_sample: 16,
        sample_format: SampleFormat::Int,
    };
    let mut cursor = Cursor::new(&mut wav_bytes);
    let mut writer =
        WavWriter::new(&mut cursor, spec).map_err(|_| "failed to create wav writer".to_string())?;

    // Peak normalization with headroom
    let mut peak: i16 = 0;
    for &s in segment {
        let a = s.unsigned_abs();
        if a > peak.unsigned_abs() {
            peak = s;
        }
    }
    let peak_abs = (peak as i32).unsigned_abs() as i32;
    let target = (i16::MAX as f32) * 0.9;
    let gain: f32 = if peak_abs > 0 {
        target / (peak_abs as f32)
    } else {
        1.0
    };

    for &s in segment {
        let v = ((s as f32) * gain)
            .round()
            .clamp(i16::MIN as f32, i16::MAX as f32) as i16;
        writer
            .write_sample(v)
            .map_err(|_| "failed writing wav sample".to_string())?;
    }
    writer
        .finalize()
        .map_err(|_| "failed to finalize wav".to_string())?;
    Ok(wav_bytes)
}
