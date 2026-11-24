mod audio;
mod audio_guard;
mod commands;
mod error;
mod events;
mod hotkey;
mod logging;
mod model;
mod overlay;
mod paste;
mod permissions;
mod pipeline;
mod recording;
mod settings;
mod transcribe;
mod transcript;
mod tray;
mod windows;

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

use parking_lot::{Mutex, RwLock};
use tauri::menu::MenuItem;
use tauri::{Manager, Wry};
use tracing::info;
use tracing_appender::non_blocking::WorkerGuard;

use crate::error::Result;
use crate::hotkey::HotkeyStatus;
use crate::model::ModelManager;
use crate::pipeline::Pipeline;
use crate::settings::SettingsStore;

pub struct AppState {
    pub settings: SettingsStore,
    pub paused: AtomicBool,
    /// True while the settings window is recording a new hotkey. Dictation is
    /// suppressed so the old binding does not fire during capture.
    pub hotkey_capture: AtomicBool,
    /// True while onboarding's test step is open. Dictation still runs, but the
    /// transcript is shown in the window instead of pasted into another app.
    pub test_mode: AtomicBool,
    pub pause_item: RwLock<Option<MenuItem<Wry>>>,
    pub log_dir: PathBuf,
    pub data_dir: PathBuf,
    pub hotkey_status: Mutex<Option<HotkeyStatus>>,
    pub recording: Mutex<Option<recording::Session>>,
    pub pipeline: Pipeline,
    pub model: ModelManager,
    // Kept alive so buffered log lines are flushed on exit.
    _log_guard: Mutex<Option<WorkerGuard>>,
}

pub fn run() {
    if let Err(err) = try_run() {
        eprintln!("Speakeasy could not start: {err}");
        std::process::exit(1);
    }
}

/// Returns `true` when this process is the audio-restore watchdog and must
/// exit without starting the app. Called from `main` before Tauri boots.
pub fn run_audio_watchdog() -> bool {
    audio_guard::maybe_run_watchdog()
}

fn try_run() -> Result<()> {
    tauri::Builder::default()
        .plugin(tauri_plugin_autostart::Builder::new().build())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let handle = app.handle().clone();
            let logging = logging::init(&handle)?;
            info!(
                version = env!("CARGO_PKG_VERSION"),
                os = std::env::consts::OS,
                arch = std::env::consts::ARCH,
                "starting Speakeasy"
            );

            let config_dir = handle
                .path()
                .app_config_dir()
                .map_err(|err| error::Error::Path(format!("config directory: {err}")))?;
            let data_dir = handle
                .path()
                .app_data_dir()
                .map_err(|err| error::Error::Path(format!("data directory: {err}")))?;
            let settings = SettingsStore::load(&config_dir)?;
            let log_dir = logging.dir.clone();
            let active = settings.get();
            let model = ModelManager::new(
                &data_dir,
                &active.model_id,
                &active.custom_model_path,
            );
            let warm = model.is_ready();
            let pipeline = Pipeline::spawn(data_dir.clone(), model.active_path(), warm)?;

            let onboarding_complete = settings.get().onboarding_complete;

            handle.manage(AppState {
                settings,
                paused: AtomicBool::new(false),
                hotkey_capture: AtomicBool::new(false),
                test_mode: AtomicBool::new(false),
                pause_item: RwLock::new(None),
                log_dir,
                data_dir,
                hotkey_status: Mutex::new(None),
                recording: Mutex::new(None),
                pipeline,
                model,
                _log_guard: Mutex::new(Some(logging.guard)),
            });

            commands::sync_autostart(&handle)?;
            tray::init(&handle)?;
            windows::apply_platform_chrome(&handle)?;
            overlay::init(&handle)?;
            hotkey::start(&handle);
            model::maybe_auto_download(&handle);
            windows::maybe_open_for_development(&handle);
            overlay::maybe_preview(&handle);

            // A first run opens straight into onboarding instead of hiding in
            // the tray with no working hotkey model.
            if !onboarding_complete {
                if let Err(err) = windows::show_settings(&handle) {
                    tracing::warn!(%err, "could not open onboarding");
                }
            }

            info!("Speakeasy is ready in the tray");
            Ok(())
        })
        .on_window_event(|window, event| windows::on_window_event(window, event))
        .invoke_handler(tauri::generate_handler![
            commands::get_app_info,
            commands::get_settings,
            commands::update_settings,
            commands::set_paused,
            commands::quit_app,
            commands::restart_app,
            commands::hide_settings,
            commands::list_input_devices,
            commands::test_microphone,
            commands::get_hotkey_status,
            commands::set_hotkey_capture,
            commands::set_test_mode,
            commands::start_test_recording,
            commands::stop_test_recording,
            commands::open_permission_settings,
            commands::accessibility_status,
            commands::get_model_status,
            commands::start_model_download,
            commands::list_models,
            commands::select_model,
            commands::hide_overlay,
        ])
        .build(tauri::generate_context!())?
        .run(|_app, event| match event {
            tauri::RunEvent::ExitRequested { .. } => {
                // Belt and braces: restoring an unmuted output is idempotent.
                let _ = audio_guard::restore_now();
            }
            tauri::RunEvent::Exit => {
                let _ = audio_guard::restore_now();
                info!("Speakeasy exited");
            }
            _ => {}
        });

    Ok(())
}
