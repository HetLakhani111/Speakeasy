use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_autostart::ManagerExt;
use tracing::{info, warn};

use crate::audio::{self, InputDeviceInfo, Recorder};
use crate::error::{Error, Result};
use crate::events;
use crate::hotkey::{self, HotkeyStatus};
use crate::model::{ModelOption, ModelStatus};
use crate::permissions;
use crate::settings::Settings;
use crate::{recording, tray, AppState};

/// How long the onboarding microphone check listens for.
const MIC_PROBE: Duration = Duration::from_millis(2200);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub version: String,
    pub platform: String,
    pub arch: String,
    pub settings_path: String,
    pub log_dir: String,
    pub data_dir: String,
    pub paused: bool,
}

#[tauri::command]
pub fn get_app_info(state: State<'_, AppState>) -> Result<AppInfo> {
    Ok(AppInfo {
        version: env!("CARGO_PKG_VERSION").to_string(),
        platform: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        settings_path: state.settings.path().display().to_string(),
        log_dir: state.log_dir.display().to_string(),
        data_dir: state.data_dir.display().to_string(),
        paused: state
            .paused
            .load(std::sync::atomic::Ordering::Relaxed),
    })
}

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> Settings {
    state.settings.get()
}

#[tauri::command]
pub fn update_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    settings: Settings,
) -> Result<Settings> {
    let previous = state.settings.get();
    let saved = state.settings.save(settings)?;

    if saved.launch_at_login != previous.launch_at_login {
        let autolaunch = app.autolaunch();
        let outcome = if saved.launch_at_login {
            autolaunch.enable()
        } else {
            autolaunch.disable()
        };

        if let Err(err) = outcome {
            // Keep the persisted value truthful if the OS refused the change.
            restore(&state, &previous);
            return Err(Error::Autostart(err.to_string()));
        }
    }

    if saved.hotkey != previous.hotkey {
        match hotkey::request_apply(&app, &saved.hotkey) {
            Ok(status) if status.registered => {}
            Ok(status) => {
                // Put the old binding back so the user is never left without one.
                restore(&state, &previous);
                let _ = hotkey::request_apply(&app, &previous.hotkey);
                return Err(Error::Hotkey(
                    status
                        .error
                        .unwrap_or_else(|| "That hotkey could not be registered.".into()),
                ));
            }
            Err(err) => {
                restore(&state, &previous);
                let _ = hotkey::request_apply(&app, &previous.hotkey);
                return Err(err);
            }
        }
    }

    info!(
        language = %saved.language,
        mute_system_audio = saved.mute_system_audio,
        launch_at_login = saved.launch_at_login,
        hotkey = %saved.hotkey,
        "settings updated"
    );

    app.emit(events::EVENT_SETTINGS_CHANGED, &saved)?;
    Ok(saved)
}

#[tauri::command]
pub fn set_paused(app: AppHandle, paused: bool) -> Result<()> {
    tray::set_paused(&app, paused)
}

#[tauri::command]
pub fn quit_app(app: AppHandle) {
    info!("quit requested from settings");
    app.exit(0);
}

/// Restarts the app. macOS only grants Accessibility changes to a freshly
/// launched process, so onboarding offers this after the user grants access.
#[tauri::command]
pub fn restart_app(app: AppHandle) {
    info!("restart requested from settings");
    app.restart();
}

/// Called by the overlay once it has shown the outcome of a dictation.
#[tauri::command]
pub fn hide_overlay(app: AppHandle) {
    if let Err(err) = crate::overlay::hide(&app) {
        warn!(%err, "could not hide the overlay");
    }
}

/// Closing onboarding without finishing it should put the app back in the
/// tray, not quit.
#[tauri::command]
pub fn hide_settings(app: AppHandle) -> Result<()> {
    crate::windows::hide_settings(&app)
}

/// Suppresses (or resumes) dictation while the settings window records a new
/// hotkey, so pressing the old binding during capture does not start a clip.
#[tauri::command]
pub fn set_hotkey_capture(state: State<'_, AppState>, active: bool) {
    state.hotkey_capture.store(active, Ordering::Relaxed);
}

/// Onboarding's test step shows the transcript instead of pasting it. Only the
/// step itself turns this on; skipping setup leaves normal dictation intact.
#[tauri::command]
pub fn set_test_mode(state: State<'_, AppState>, active: bool) {
    state.test_mode.store(active, Ordering::Relaxed);
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MicProbe {
    pub device: String,
    /// Highest absolute sample value; 0.0 is silence and 1.0 is clipping.
    pub peak: f32,
    pub rms: f32,
    pub duration_ms: u64,
}

/// Restores the previous `hotkey_capture` value when the microphone probe ends.
struct CaptureGuard<'a> {
    flag: &'a AtomicBool,
    previous: bool,
}

impl Drop for CaptureGuard<'_> {
    fn drop(&mut self) {
        self.flag.store(self.previous, Ordering::Relaxed);
    }
}

/// Records a short sample and reports its level. Used by onboarding to prove
/// the microphone works before the user relies on it. Nothing is transcribed,
/// pasted, or persisted.
#[tauri::command]
pub async fn test_microphone(
    app: AppHandle,
    state: State<'_, AppState>,
    device: Option<String>,
) -> Result<MicProbe> {
    if state.recording.lock().is_some() {
        return Err(Error::Audio(
            "A recording is already in progress. Try again in a moment.".into(),
        ));
    }

    tauri::async_runtime::spawn_blocking(move || {
        // Suppress the hotkey while the probe holds the device, so a press
        // cannot open a second stream. The previous value is restored even if
        // the probe fails.
        let state_owner = app.clone();
        let state = state_owner.state::<AppState>();
        let previous = state.hotkey_capture.swap(true, Ordering::Relaxed);
        let _capture = CaptureGuard {
            flag: &state.hotkey_capture,
            previous,
        };

        let recorder = Recorder::start(device, app)?;
        let name = recorder.device().to_string();
        std::thread::sleep(MIC_PROBE);
        let captured = recorder.stop()?;

        let samples = &captured.samples;
        let peak = audio::peak_amplitude(samples);
        let rms = if samples.is_empty() {
            0.0
        } else {
            (samples.iter().map(|sample| sample * sample).sum::<f32>()
                / samples.len() as f32)
                .sqrt()
        };

        Ok(MicProbe {
            device: name,
            peak,
            rms,
            duration_ms: captured.duration().as_millis() as u64,
        })
    })
    .await
    .map_err(|err| Error::Audio(format!("microphone check failed: {err}")))?
}

/// Opens the operating system page for `pane` (`microphone`, `accessibility`,
/// or `input-monitoring`).
#[tauri::command]
pub fn open_permission_settings(pane: String) -> Result<()> {
    let pane = permissions::Pane::parse(&pane)
        .ok_or_else(|| Error::Invalid(format!("unknown permission pane `{pane}`")))?;
    permissions::open(pane)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionStatus {
    pub accessibility: bool,
}

#[tauri::command]
pub fn accessibility_status() -> PermissionStatus {
    PermissionStatus {
        accessibility: permissions::accessibility_trusted(),
    }
}

#[tauri::command]
pub async fn list_input_devices() -> Result<Vec<InputDeviceInfo>> {
    tauri::async_runtime::spawn_blocking(audio::list_input_devices)
        .await
        .map_err(|err| Error::Audio(format!("microphone enumeration failed: {err}")))?
}

#[tauri::command]
pub fn get_hotkey_status(app: AppHandle) -> Option<HotkeyStatus> {
    hotkey::status(&app)
}

/// Starts a recording without the hotkey. Used by onboarding's dictation test.
#[tauri::command]
pub fn start_test_recording(app: AppHandle) -> Result<()> {
    recording::begin(&app, true)
}

#[tauri::command]
pub fn stop_test_recording(app: AppHandle) -> Result<()> {
    recording::end(&app)
}

/// Returns the current model state, re-checking the file on disk first so a
/// model the user deleted is reported as missing rather than ready.
#[tauri::command]
pub fn get_model_status(state: State<'_, AppState>) -> ModelStatus {
    state.model.refresh();
    state.model.status()
}

/// Starts (or resumes) the one-time model download and returns immediately.
#[tauri::command]
pub fn start_model_download(app: AppHandle, state: State<'_, AppState>) -> Result<ModelStatus> {
    state.model.refresh();
    state.model.start_download(app)?;
    Ok(state.model.status())
}

/// Every model the user can choose, including one already on their disk.
#[tauri::command]
pub fn list_models(state: State<'_, AppState>) -> Vec<ModelOption> {
    state.model.list()
}

/// Switches the active model, persists the choice, and warms the new model when
/// it is already available. `custom_path` is only meaningful with the `custom`
/// id.
#[tauri::command]
pub fn select_model(
    app: AppHandle,
    state: State<'_, AppState>,
    model_id: String,
    custom_path: String,
) -> Result<ModelStatus> {
    let previous = state.settings.get();
    let status = state.model.select(&model_id, &custom_path)?;

    let mut next = previous.clone();
    next.model_id = status.id.clone();
    next.custom_model_path = if status.custom {
        status.path.clone()
    } else {
        String::new()
    };

    if let Err(err) = state.settings.save(next.clone()) {
        // Put the old model back so the running app matches what is stored.
        let _ = state
            .model
            .select(&previous.model_id, &previous.custom_model_path);
        return Err(err);
    }

    app.emit(events::EVENT_SETTINGS_CHANGED, &next)?;
    app.emit(events::EVENT_MODEL_STATUS, &status)?;

    if status.phase == crate::model::ModelPhase::Ready {
        if let Err(err) = state.pipeline.warmup(app.clone()) {
            warn!(%err, "could not warm the selected model");
        }
    }

    Ok(status)
}

/// Aligns the persisted setting with what the OS actually has registered.
pub fn sync_autostart(app: &AppHandle) -> Result<()> {
    let state = app.state::<AppState>();
    let current = state.settings.get();

    match app.autolaunch().is_enabled() {
        Ok(enabled) if enabled != current.launch_at_login => {
            let mut next = current;
            next.launch_at_login = enabled;
            match state.settings.save(next) {
                Ok(saved) => {
                    if let Err(err) = app.emit(events::EVENT_SETTINGS_CHANGED, &saved) {
                        warn!(%err, "could not emit reconciled settings");
                    }
                }
                Err(err) => warn!(%err, "could not reconcile launch-at-login setting"),
            }
        }
        Ok(_) => {}
        Err(err) => warn!(%err, "could not read launch-at-login state"),
    }

    Ok(())
}

fn restore(state: &State<'_, AppState>, previous: &Settings) {
    if let Err(err) = state.settings.save(previous.clone()) {
        warn!(%err, "could not roll back settings");
    }
}
