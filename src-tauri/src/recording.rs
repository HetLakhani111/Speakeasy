//! Recording lifecycle shared by the hotkey listener and the test-recording
//! commands. The listener is never allowed to block on capture shutdown or on
//! processing, so stopping hands the microphone off to a short-lived thread.

use std::sync::atomic::Ordering;
use std::thread;
use std::time::Instant;

use tauri::{AppHandle, Emitter, Manager};
use tracing::{info, warn};

use crate::audio::Recorder;
use crate::error::{Error, Result};
use crate::events::{self, NotificationLevel, RecordingStarted, RecordingStopped};
use crate::{audio_guard, AppState};

/// An open microphone plus the reason it was opened. Onboarding's test
/// dictation uses `test` so the transcript is shown but never pasted into the
/// settings window.
pub struct Session {
    pub recorder: Recorder,
    pub test: bool,
}

pub fn on_hotkey_press(app: &AppHandle) {
    let state = app.state::<AppState>();
    if state.paused.load(Ordering::Relaxed) || state.hotkey_capture.load(Ordering::Relaxed) {
        return;
    }
    let test = state.test_mode.load(Ordering::Relaxed);
    drop(state);

    if let Err(err) = begin(app, test) {
        warn!(%err, "could not start recording");
        events::notify(app, NotificationLevel::Warning, err.to_string());
    }
}

pub fn on_hotkey_release(app: &AppHandle) {
    // A release with no open microphone is normal: the press may have been
    // suppressed while the user was rebinding the key.
    let has_session = app.state::<AppState>().recording.lock().is_some();
    if !has_session {
        return;
    }

    if let Err(err) = end(app) {
        warn!(%err, "could not stop recording");
    }
}

/// Opens the microphone and starts buffering.
pub fn begin(app: &AppHandle, test: bool) -> Result<()> {
    let state = app.state::<AppState>();

    if state.paused.load(Ordering::Relaxed) {
        return Err(Error::Audio("Speakeasy is paused.".into()));
    }

    let mut slot = state.recording.lock();
    if slot.is_some() {
        return Err(Error::Audio("A recording is already in progress.".into()));
    }

    let settings = state.settings.get();

    // Open the microphone before touching the system output: capture latency is
    // what the user feels, and muting spawns a small watchdog process. A few
    // frames of system audio in the recording is harmless; a late microphone is
    // not. A mute failure is never fatal.
    let recorder = Recorder::start(settings.microphone.clone(), app.clone())?;

    if settings.mute_system_audio {
        match audio_guard::mute_now() {
            Ok(true) => {}
            Ok(false) => warn!("system output control is unavailable on this platform"),
            Err(err) => warn!(%err, "continuing without muting the system output"),
        }
    }

    let device = recorder.device().to_string();
    *slot = Some(Session { recorder, test });
    drop(slot);

    info!(%device, test, "hold-to-talk started");
    if let Err(err) = crate::overlay::show(app) {
        warn!(%err, "could not show the recording overlay");
    }
    if let Err(err) = app.emit(
        events::EVENT_RECORDING_STARTED,
        RecordingStarted {
            device: Some(device),
        },
    ) {
        warn!(%err, "could not emit recording-started");
    }

    Ok(())
}

/// Stops capture and schedules processing. Returns once the buffer is handed
/// off, not once transcription is finished.
pub fn end(app: &AppHandle) -> Result<()> {
    let state = app.state::<AppState>();
    let session = state
        .recording
        .lock()
        .take()
        .ok_or_else(|| Error::Audio("No recording is in progress.".into()))?;
    drop(state);

    // Restore the speakers as early as possible; transcription still has to
    // run but the user should not stay in silence while it does.
    if let Err(err) = audio_guard::restore_now() {
        warn!(%err, "system output was not restored");
    }

    let held_for = session.recorder.elapsed();
    let test = session.test;
    // Start the latency clock at the moment the user let go, so the reported
    // number includes microphone shutdown and queueing, not just inference.
    let released_at = Instant::now();
    let app = app.clone();

    thread::Builder::new()
        .name("speakeasy-clip".into())
        .spawn(move || {
            match session.recorder.stop() {
                Ok(captured) => {
                    let state = app.state::<AppState>();
                    if let Err(err) = state.pipeline.submit(
                        captured,
                        held_for,
                        released_at,
                        !test,
                        app.clone(),
                    ) {
                        warn!(%err, "could not queue the audio clip");
                        events::notify(
                            &app,
                            NotificationLevel::Error,
                            format!("Could not process the recording: {err}"),
                        );
                        emit_empty_stop(&app);
                    }
                }
                Err(err) => {
                    warn!(%err, "could not finish recording");
                    events::notify(&app, NotificationLevel::Error, err.to_string());
                    emit_empty_stop(&app);
                }
            }
        })
        .map_err(|err| Error::Audio(format!("could not stop the recorder: {err}")))?;

    Ok(())
}

fn emit_empty_stop(app: &AppHandle) {
    events::emit_recording_stopped(
        app,
        RecordingStopped {
            duration_ms: 0,
            captured_ms: 0,
            peak: 0.0,
            wav_path: None,
        },
    );
}
