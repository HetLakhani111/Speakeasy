//! The processing worker.
//!
//! The hotkey listener hands raw microphone audio to this thread and returns
//! immediately. Resampling, silence trimming, optional WAV persistence, Whisper
//! inference, and pasting all happen here, entirely off the hotkey and UI
//! threads. The Whisper context is loaded once and kept for the life of the
//! thread.

use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use crossbeam_channel::{unbounded, Sender};
use tauri::{AppHandle, Manager};
use tracing::{info, warn};

use crate::audio::{self, CapturedAudio, MIN_RECORDING, TARGET_SAMPLE_RATE};
use crate::error::{Error, Result};
use crate::events::{self, NotificationLevel, RecordingStopped, TranscriptEvent};
use crate::paste;
use crate::transcribe::Transcriber;
use crate::transcript;
use crate::AppState;

/// The performance target for release-to-paste on a five second clip.
const LATENCY_BUDGET: Duration = Duration::from_millis(1500);

enum Job {
    Clip {
        captured: CapturedAudio,
        held_for: Duration,
        released_at: Instant,
        /// False for onboarding's test dictation, which shows the transcript
        /// without sending it to whatever window has focus.
        paste: bool,
        app: AppHandle,
    },
    /// Loads the model without a clip. Sent after a download finishes, after the
    /// user switches models, and when a returning user launches the app.
    Warmup { app: AppHandle },
}

pub struct Pipeline {
    sender: Sender<Job>,
}

impl Pipeline {
    pub fn spawn(data_dir: PathBuf, model_path: PathBuf, warm: bool) -> Result<Self> {
        let (sender, receiver) = unbounded::<Job>();

        thread::Builder::new()
            .name("speakeasy-pipeline".into())
            .spawn(move || {
                let mut transcriber = Transcriber::new(model_path);
                if warm && transcriber.available() {
                    // A failure here is only an optimization miss; the next
                    // clip will try again and surface the real error.
                    if let Err(err) = transcriber.preload() {
                        warn!(%err, "could not preload the model");
                    }
                }

                while let Ok(job) = receiver.recv() {
                    match job {
                        Job::Clip {
                            captured,
                            held_for,
                            released_at,
                            paste,
                            app,
                        } => {
                            // The user can point Speakeasy at another model at
                            // any time. Checking here means the next dictation
                            // uses it, at the cost of one reload.
                            let model = app.state::<AppState>().model.active_path();
                            transcriber.switch_to(model);
                            process(
                                captured,
                                held_for,
                                released_at,
                                paste,
                                &app,
                                &data_dir,
                                &mut transcriber,
                            )
                        }
                        Job::Warmup { app } => {
                            let model = app.state::<AppState>().model.active_path();
                            transcriber.switch_to(model);
                            if transcriber.available() {
                                if let Err(err) = transcriber.preload() {
                                    warn!(%err, "could not preload the model");
                                }
                            }
                        }
                    }
                }
            })
            .map_err(|err| Error::Audio(format!("could not start the processing thread: {err}")))?;

        Ok(Self { sender })
    }

    pub fn submit(
        &self,
        captured: CapturedAudio,
        held_for: Duration,
        released_at: Instant,
        paste: bool,
        app: AppHandle,
    ) -> Result<()> {
        self.sender
            .send(Job::Clip {
                captured,
                held_for,
                released_at,
                paste,
                app,
            })
            .map_err(|_| Error::Audio("the processing worker stopped unexpectedly".into()))
    }

    /// Asks the worker to load the model ahead of the next dictation.
    pub fn warmup(&self, app: AppHandle) -> Result<()> {
        self.sender
            .send(Job::Warmup { app })
            .map_err(|_| Error::Audio("the processing worker stopped unexpectedly".into()))
    }
}

fn process(
    captured: CapturedAudio,
    held_for: Duration,
    released_at: Instant,
    paste: bool,
    app: &AppHandle,
    data_dir: &Path,
    transcriber: &mut Transcriber,
) {
    let captured_ms = captured.duration().as_millis() as u64;
    info!(captured_ms, "processing recording");

    if held_for < MIN_RECORDING || captured.duration() < MIN_RECORDING {
        emit_stopped(app, 0, captured_ms, 0.0, None);
        events::notify(
            app,
            NotificationLevel::Info,
            "Recording too short — hold the hotkey a little longer.",
        );
        return;
    }

    let mut samples = audio::resample_to_16k(&captured.samples, captured.sample_rate);
    drop(captured);

    // Persist the full resampled capture before trimming. Hearing the raw
    // microphone signal is the fastest way to diagnose a quiet or noisy
    // input; this only ever runs in debug builds or when explicitly asked.
    if keep_recordings() {
        let path = data_dir.join("recordings").join("last-capture.wav");
        match audio::wav::write_pcm16(&path, &samples, TARGET_SAMPLE_RATE) {
            Ok(()) => info!(path = %path.display(), "saved raw debug recording"),
            Err(err) => warn!(%err, "could not save the raw debug recording"),
        }
    }

    let before = samples.len();
    audio::trim_silence(&mut samples, TARGET_SAMPLE_RATE);
    let trimmed = before.saturating_sub(samples.len());

    let duration = Duration::from_secs_f64(samples.len() as f64 / f64::from(TARGET_SAMPLE_RATE));
    let peak = audio::peak_amplitude(&samples);

    if duration < MIN_RECORDING {
        emit_stopped(app, 0, captured_ms, peak, None);
        events::notify(
            app,
            NotificationLevel::Info,
            "No speech detected — try again a little closer to the microphone.",
        );
        return;
    }

    if peak < 0.001 {
        emit_stopped(app, 0, captured_ms, peak, None);
        events::notify(
            app,
            NotificationLevel::Warning,
            "That recording was almost silent. Check the selected microphone in Settings.",
        );
        return;
    }

    let wav_path = if keep_recordings() {
        let path = data_dir.join("recordings").join("last-recording.wav");
        match audio::wav::write_pcm16(&path, &samples, TARGET_SAMPLE_RATE) {
            Ok(()) => {
                info!(
                    path = %path.display(),
                    duration_ms = duration.as_millis() as u64,
                    "saved debug recording"
                );
                Some(path.display().to_string())
            }
            Err(err) => {
                warn!(%err, "could not save the debug recording");
                None
            }
        }
    } else {
        None
    };

    let audio_ms = duration.as_millis() as u64;
    info!(
        captured_ms,
        processed_ms = audio_ms,
        trimmed_ms = (trimmed as u64 * 1000) / u64::from(TARGET_SAMPLE_RATE),
        peak,
        "clip ready for transcription"
    );

    emit_stopped(app, audio_ms, captured_ms, peak, wav_path.clone());

    let state = app.state::<AppState>();
    if !state.model.is_ready() {
        warn!("a clip arrived before the model was ready");
        events::notify(
            app,
            NotificationLevel::Warning,
            "The speech model isn't ready yet. Download it from Settings and try again.",
        );
        return;
    }

    let language = state.settings.get().language;
    drop(state);

    let transcription = match transcriber.transcribe(&samples, &language) {
        Ok(transcription) => transcription,
        Err(err) => {
            warn!(%err, "transcription failed");
            events::notify(
                app,
                NotificationLevel::Error,
                format!("Could not transcribe that clip: {err}"),
            );
            return;
        }
    };

    let cleaned = transcript::clean(&transcription.text);
    if cleaned.is_empty() {
        info!(
            inference_ms = transcription.inference.as_millis() as u64,
            "whisper returned no speech"
        );
        events::notify(
            app,
            NotificationLevel::Info,
            "Didn't catch any speech in that clip.",
        );
        return;
    }

    let paste_started = Instant::now();
    let paste_result = if paste {
        paste::paste(app, &cleaned)
    } else {
        Ok(())
    };
    let paste_ms = paste_started.elapsed().as_millis() as u64;
    let total_ms = released_at.elapsed().as_millis() as u64;
    let load_ms = transcription.load.as_millis() as u64;
    let inference_ms = transcription.inference.as_millis() as u64;

    if total_ms as u128 > LATENCY_BUDGET.as_millis() {
        warn!(
            total_ms,
            load_ms,
            inference_ms,
            paste_ms,
            "dictation exceeded the latency budget"
        );
    } else {
        info!(
            total_ms,
            load_ms,
            inference_ms,
            paste_ms,
            audio_ms,
            pasted = paste,
            hold_ms = held_for.as_millis() as u64,
            chars = cleaned.chars().count(),
            "dictation finished"
        );
    }

    if let Err(err) = &paste_result {
        warn!(%err, "could not paste the transcript");
        events::notify(app, NotificationLevel::Error, err.to_string());
    }

    events::emit_transcript(
        app,
        TranscriptEvent {
            text: cleaned,
            audio_ms,
            load_ms,
            inference_ms,
            total_ms,
            wav_path,
        },
    );
}

/// Recordings are only persisted while developing (or when explicitly asked),
/// never in a release build the user did not opt into.
fn keep_recordings() -> bool {
    cfg!(debug_assertions) || std::env::var_os("SPEAKEASY_KEEP_WAV").is_some()
}

fn emit_stopped(
    app: &AppHandle,
    duration_ms: u64,
    captured_ms: u64,
    peak: f32,
    wav_path: Option<String>,
) {
    events::emit_recording_stopped(
        app,
        RecordingStopped {
            duration_ms,
            captured_ms,
            peak,
            wav_path,
        },
    );
}
