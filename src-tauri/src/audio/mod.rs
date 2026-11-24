//! Microphone capture.
//!
//! cpal owns the device stream on a dedicated thread; the callback only
//! converts frames to mono f32 and pushes them into a bounded channel. All
//! heavier work (resampling, trimming, persistence, transcription) happens
//! later on the pipeline thread so the audio callback never blocks.

pub mod meter;
pub mod resample;
pub mod wav;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SizedSample};
use crossbeam_channel::{bounded, RecvTimeoutError, Sender};
use serde::Serialize;
use tauri::AppHandle;
use tracing::{info, warn};

use crate::error::{Error, Result};
use crate::events;
use meter::Meter;

pub use resample::{resample_to_16k, trim_silence, TARGET_SAMPLE_RATE};

/// Recordings shorter than this are treated as an accidental tap.
pub const MIN_RECORDING: Duration = Duration::from_millis(300);

/// Hard stop so a missed key-release cannot grow memory without bound.
const MAX_RECORDING_SECONDS: usize = 300;

/// Meter updates are throttled to roughly 60 fps.
const METER_INTERVAL: Duration = Duration::from_millis(16);

const CHANNEL_CAPACITY: usize = 512;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InputDeviceInfo {
    pub name: String,
    pub is_default: bool,
}

#[derive(Debug, Clone)]
pub struct CapturedAudio {
    /// Mono samples at the device's native rate.
    pub samples: Vec<f32>,
    pub sample_rate: u32,
}

impl CapturedAudio {
    pub fn duration(&self) -> Duration {
        if self.sample_rate == 0 {
            return Duration::ZERO;
        }
        Duration::from_secs_f64(self.samples.len() as f64 / f64::from(self.sample_rate))
    }
}

pub fn list_input_devices() -> Result<Vec<InputDeviceInfo>> {
    let host = cpal::default_host();
    let default_name = host.default_input_device().and_then(|device| device_name(&device));

    let devices = host
        .input_devices()
        .map_err(|err| Error::Audio(format!("could not enumerate microphones: {err}")))?;

    let mut result: Vec<InputDeviceInfo> = Vec::new();
    for device in devices {
        // Some Windows drivers expose the same friendly name more than once.
        if let Some(name) = device_name(&device) {
            if name.trim().is_empty() || result.iter().any(|entry| entry.name == name) {
                continue;
            }
            let is_default = default_name.as_deref() == Some(name.as_str());
            result.push(InputDeviceInfo { name, is_default });
        }
    }

    if result.is_empty() {
        return Err(Error::Audio("no microphones were found".into()));
    }

    Ok(result)
}

/// Friendly name for a device. cpal reports the same information through
/// `description()` on every host; `name()` is only a convenience wrapper.
fn device_name(device: &cpal::Device) -> Option<String> {
    device
        .description()
        .ok()
        .map(|description| description.name().to_string())
}

enum CaptureMessage {
    Samples(Vec<f32>),
    Error(String),
}

pub struct Recorder {
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<Result<CapturedAudio>>>,
    started_at: Instant,
    device: String,
}

impl Recorder {
    /// Opens the selected microphone and begins buffering. Returns only once
    /// the stream is actually running, so a failure surfaces immediately.
    pub fn start(device_name: Option<String>, app: AppHandle) -> Result<Self> {
        // Development hook: replaying a WAV through the real recording
        // lifecycle makes transcription and pasting testable on a machine
        // whose microphone cannot pick up test speech. Never compiled into a
        // release build unless the `dev-wav` feature is enabled.
        #[cfg(any(debug_assertions, feature = "dev-wav"))]
        if let Some(path) = std::env::var_os("SPEAKEASY_DEBUG_WAV") {
            return Self::from_wav(std::path::PathBuf::from(path));
        }

        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let (ready_tx, ready_rx) =
            bounded::<std::result::Result<String, String>>(1);

        let join = thread::Builder::new()
            .name("speakeasy-audio".into())
            .spawn(move || record_loop(device_name, app, thread_stop, ready_tx))
            .map_err(|err| Error::Audio(format!("could not start the audio thread: {err}")))?;

        match ready_rx.recv_timeout(Duration::from_secs(3)) {
            Ok(Ok(device)) => Ok(Self {
                stop,
                join: Some(join),
                started_at: Instant::now(),
                device,
            }),
            Ok(Err(message)) => {
                stop.store(true, Ordering::Relaxed);
                let _ = join.join();
                Err(Error::Audio(message))
            }
            Err(_) => {
                stop.store(true, Ordering::Relaxed);
                Err(Error::Audio(
                    "the microphone did not start within three seconds".into(),
                ))
            }
        }
    }

    pub fn elapsed(&self) -> Duration {
        self.started_at.elapsed()
    }

    pub fn device(&self) -> &str {
        &self.device
    }

    /// Stops capture and joins the recording thread.
    pub fn stop(mut self) -> Result<CapturedAudio> {
        self.stop.store(true, Ordering::Relaxed);
        let Some(join) = self.join.take() else {
            return Err(Error::Audio("the recorder was already stopped".into()));
        };

        match join.join() {
            Ok(result) => result,
            Err(_) => Err(Error::Audio("the audio thread panicked".into())),
        }
    }

    /// Builds a recorder that returns a fixed WAV instead of microphone audio.
    /// Debug builds and the `dev-wav` feature only; see the environment hook
    /// in `start`.
    #[cfg(any(debug_assertions, feature = "dev-wav"))]
    fn from_wav(path: std::path::PathBuf) -> Result<Self> {
        let (samples, sample_rate) = wav::read_mono(&path)?;
        if samples.is_empty() || sample_rate == 0 {
            return Err(Error::Audio(format!(
                "{} contains no usable audio",
                path.display()
            )));
        }

        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let join = thread::Builder::new()
            .name("speakeasy-debug-audio".into())
            .spawn(move || {
                while !thread_stop.load(Ordering::Relaxed) {
                    thread::sleep(Duration::from_millis(10));
                }
                Ok(CapturedAudio {
                    samples,
                    sample_rate,
                })
            })
            .map_err(|err| Error::Audio(format!("could not start the debug audio thread: {err}")))?;

        info!(path = %path.display(), "replaying a debug WAV instead of the microphone");
        Ok(Self {
            stop,
            join: Some(join),
            started_at: Instant::now(),
            device: "Debug WAV".to_string(),
        })
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

fn record_loop(
    device_name: Option<String>,
    app: AppHandle,
    stop: Arc<AtomicBool>,
    ready: Sender<std::result::Result<String, String>>,
) -> Result<CapturedAudio> {
    let host = cpal::default_host();
    let result = run_capture(&host, device_name, &app, &stop, &ready);
    if let Err(err) = &result {
        // The waiting side may already have reported this; the extra send is
        // harmless when the bounded channel is full.
        let _ = ready.try_send(Err(err.to_string()));
        warn!(%err, "microphone capture failed");
    }
    result
}

fn run_capture(
    host: &cpal::Host,
    device_name: Option<String>,
    app: &AppHandle,
    stop: &AtomicBool,
    ready: &Sender<std::result::Result<String, String>>,
) -> Result<CapturedAudio> {
    let (device, name) = select_device(host, device_name.as_deref())?;
    let supported = device
        .default_input_config()
        .map_err(|err| Error::Audio(format!("{name}: could not read the input format: {err}")))?;

    let sample_format = supported.sample_format();
    let stream_config: cpal::StreamConfig = supported.into();
    let sample_rate = stream_config.sample_rate;
    let channels = stream_config.channels as usize;
    let (tx, rx) = bounded::<CaptureMessage>(CHANNEL_CAPACITY);

    let stream = build_stream(&device, &stream_config, sample_format, channels, tx)
        .map_err(|err| Error::Audio(format!("{name}: could not open the microphone: {err}")))?;

    stream
        .play()
        .map_err(|err| Error::Audio(format!("{name}: could not start the microphone: {err}")))?;

    let _ = ready.try_send(Ok(name.clone()));
    info!(
        device = %name,
        sample_rate,
        channels,
        format = ?sample_format,
        "recording started"
    );

    let mut samples: Vec<f32> = Vec::with_capacity(sample_rate as usize * 8);
    let mut meter = match Meter::new(sample_rate) {
        Ok(meter) => Some(meter),
        Err(err) => {
            warn!(%err, "live visualizer disabled");
            None
        }
    };
    let mut last_meter = Instant::now();
    let max_samples = sample_rate as usize * MAX_RECORDING_SECONDS;
    let mut had_error = false;

    loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }

        match rx.recv_timeout(Duration::from_millis(25)) {
            Ok(CaptureMessage::Samples(chunk)) => {
                samples.extend_from_slice(&chunk);

                if last_meter.elapsed() >= METER_INTERVAL {
                    let mut disable_meter = false;
                    if let Some(active) = meter.as_mut() {
                        match active.analyze(&samples) {
                            Ok(levels) => events::emit_levels(app, levels),
                            Err(err) => {
                                warn!(%err, "could not analyze audio levels");
                                disable_meter = true;
                            }
                        }
                    }
                    if disable_meter {
                        meter = None;
                    }
                    last_meter = Instant::now();
                }

                if samples.len() >= max_samples {
                    warn!("recording hit the five minute limit; stopping capture");
                    break;
                }
            }
            Ok(CaptureMessage::Error(message)) => {
                warn!(%message, "the microphone reported an error; stopping capture");
                had_error = true;
                break;
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }

    // Drop the stream before draining so no further callbacks race the reader.
    drop(stream);

    // Pick up anything the callback queued just before the stream closed.
    while let Ok(message) = rx.try_recv() {
        if let CaptureMessage::Samples(chunk) = message {
            samples.extend_from_slice(&chunk);
        }
    }

    if had_error && samples.is_empty() {
        return Err(Error::Audio(format!(
            "{name}: the microphone stopped before delivering audio"
        )));
    }

    let captured = CapturedAudio {
        samples,
        sample_rate,
    };
    info!(
        device = %name,
        duration_ms = captured.duration().as_millis() as u64,
        peak = peak_amplitude(&captured.samples),
        "recording stopped"
    );
    Ok(captured)
}

fn select_device(host: &cpal::Host, wanted: Option<&str>) -> Result<(cpal::Device, String)> {
    if let Some(wanted) = wanted {
        if let Ok(devices) = host.input_devices() {
            for device in devices {
                if device_name(&device).as_deref() == Some(wanted) {
                    return Ok((device, wanted.to_string()));
                }
            }
        }
        warn!(requested = wanted, "selected microphone is unavailable; using the default");
    }

    let device = host
        .default_input_device()
        .ok_or_else(|| Error::Audio("no microphone is available".into()))?;
    let name =
        device_name(&device).unwrap_or_else(|| "System default microphone".to_string());
    Ok((device, name))
}

fn build_stream(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    format: SampleFormat,
    channels: usize,
    tx: Sender<CaptureMessage>,
) -> std::result::Result<cpal::Stream, cpal::BuildStreamError> {
    match format {
        SampleFormat::I8 => build_typed::<i8>(device, config, channels, tx),
        SampleFormat::I16 => build_typed::<i16>(device, config, channels, tx),
        SampleFormat::I24 => build_typed::<cpal::I24>(device, config, channels, tx),
        SampleFormat::I32 => build_typed::<i32>(device, config, channels, tx),
        SampleFormat::I64 => build_typed::<i64>(device, config, channels, tx),
        SampleFormat::U8 => build_typed::<u8>(device, config, channels, tx),
        SampleFormat::U16 => build_typed::<u16>(device, config, channels, tx),
        SampleFormat::U24 => build_typed::<cpal::U24>(device, config, channels, tx),
        SampleFormat::U32 => build_typed::<u32>(device, config, channels, tx),
        SampleFormat::U64 => build_typed::<u64>(device, config, channels, tx),
        SampleFormat::F32 => build_typed::<f32>(device, config, channels, tx),
        SampleFormat::F64 => build_typed::<f64>(device, config, channels, tx),
        _ => Err(cpal::BuildStreamError::StreamConfigNotSupported),
    }
}

fn build_typed<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    channels: usize,
    tx: Sender<CaptureMessage>,
) -> std::result::Result<cpal::Stream, cpal::BuildStreamError>
where
    T: SizedSample + Sample + Send + 'static,
    f32: FromSample<T>,
{
    let error_tx = tx.clone();
    device.build_input_stream(
        config,
        move |data: &[T], _: &cpal::InputCallbackInfo| {
            let mut mono = Vec::with_capacity(data.len() / channels.max(1));
            for frame in data.chunks(channels.max(1)) {
                let mut sum = 0.0_f32;
                for sample in frame {
                    sum += f32::from_sample(*sample);
                }
                mono.push(sum / frame.len() as f32);
            }

            // Never block the audio callback. If the consumer falls behind we
            // prefer dropping a buffer over glitching the stream.
            let _ = tx.try_send(CaptureMessage::Samples(mono));
        },
        move |err| {
            let _ = error_tx.try_send(CaptureMessage::Error(err.to_string()));
        },
        None,
    )
}

pub fn peak_amplitude(samples: &[f32]) -> f32 {
    samples
        .iter()
        .fold(0.0_f32, |peak, sample| peak.max(sample.abs()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rms(samples: &[f32]) -> f32 {
        if samples.is_empty() {
            return 0.0;
        }
        (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
    }

    #[test]
    fn captured_duration_tracks_samples() {
        let captured = CapturedAudio {
            samples: vec![0.0; 8_000],
            sample_rate: 16_000,
        };
        assert_eq!(captured.duration(), Duration::from_millis(500));
    }

    #[test]
    fn peak_and_rms_measure_a_sine() {
        let tone: Vec<f32> = (0..1_600)
            .map(|index| {
                (2.0 * std::f32::consts::PI * 440.0 * index as f32 / 16_000.0).sin() * 0.5
            })
            .collect();
        assert!((peak_amplitude(&tone) - 0.5).abs() < 0.01);
        assert!((rms(&tone) - 0.3535).abs() < 0.01);
    }
}
