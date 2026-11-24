//! Event names and payloads shared with the TypeScript frontend.
//!
//! `src/lib/events.ts` mirrors this file. Keep both sides in sync whenever a
//! payload changes.

use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tracing::warn;

use crate::audio::meter::AudioLevels;

pub const EVENT_APP_STATE: &str = "app-state";
pub const EVENT_SETTINGS_CHANGED: &str = "settings-changed";
pub const EVENT_AUDIO_LEVELS: &str = "audio-levels";
pub const EVENT_RECORDING_STARTED: &str = "recording-started";
pub const EVENT_RECORDING_STOPPED: &str = "recording-stopped";
pub const EVENT_NOTIFICATION: &str = "notification";
pub const EVENT_HOTKEY_STATUS: &str = "hotkey-status";
pub const EVENT_TRANSCRIPT: &str = "transcript";
pub const EVENT_MODEL_STATUS: &str = "model-status";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PausedEvent {
    pub paused: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingStarted {
    pub device: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingStopped {
    /// Length of audio actually sent to Whisper after resampling and trimming.
    pub duration_ms: u64,
    /// Length of audio captured from the microphone before processing.
    pub captured_ms: u64,
    pub peak: f32,
    /// Debug-only path to the saved WAV, if one was written.
    pub wav_path: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptEvent {
    pub text: String,
    /// Length of the audio Whisper received.
    pub audio_ms: u64,
    /// Time spent loading the model, if this clip triggered the first load.
    pub load_ms: u64,
    pub inference_ms: u64,
    /// Hotkey release to paste. This is the number the 1.5 s budget refers to.
    pub total_ms: u64,
    /// Debug-only path to the WAV that was transcribed, if one was written.
    pub wav_path: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum NotificationLevel {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Notification {
    pub level: NotificationLevel,
    pub message: String,
}

/// Sends a non-intrusive message to whichever windows are listening.
pub fn notify(app: &AppHandle, level: NotificationLevel, message: impl Into<String>) {
    let payload = Notification {
        level,
        message: message.into(),
    };
    if let Err(err) = app.emit(EVENT_NOTIFICATION, payload) {
        warn!(%err, "could not emit notification");
    }
}

pub fn emit_levels(app: &AppHandle, levels: AudioLevels) {
    if let Err(err) = app.emit(EVENT_AUDIO_LEVELS, levels) {
        // This races ordinary window teardown, so it is not treated as an error.
        warn!(%err, "could not emit audio levels");
    }
}

pub fn emit_recording_stopped(app: &AppHandle, payload: RecordingStopped) {
    if let Err(err) = app.emit(EVENT_RECORDING_STOPPED, payload) {
        warn!(%err, "could not emit recording-stopped");
    }
}

pub fn emit_transcript(app: &AppHandle, payload: TranscriptEvent) {
    if let Err(err) = app.emit(EVENT_TRANSCRIPT, payload) {
        warn!(%err, "could not emit transcript");
    }
}
