use std::sync::atomic::Ordering;

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager};
use tracing::{error, info, warn};

use crate::error::{Error, Result};
use crate::events::{self, PausedEvent};
use crate::windows;
use crate::AppState;

const TRAY_ID: &str = "main";

pub fn init(app: &AppHandle) -> Result<()> {
    let settings_item = MenuItem::with_id(app, "settings", "Settings…", true, None::<&str>)?;
    let pause_item = MenuItem::with_id(app, "toggle-pause", "Pause", true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, "quit", "Quit Speakeasy", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &settings_item,
            &pause_item,
            &PredefinedMenuItem::separator(app)?,
            &quit_item,
        ],
    )?;

    {
        let state = app.state::<AppState>();
        *state.pause_item.write() = Some(pause_item);
    }

    #[cfg(target_os = "macos")]
    let icon = tauri::image::Image::from_bytes(include_bytes!("../icons/tray-template.png"))
        .map_err(|err| Error::Tray(err.to_string()))?;

    #[cfg(not(target_os = "macos"))]
    let icon = app
        .default_window_icon()
        .cloned()
        .ok_or_else(|| Error::Tray("the bundled icon could not be loaded".into()))?;

    let builder = TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon)
        .tooltip("Speakeasy — hold to talk")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| {
            if let Err(err) = handle_menu(app, event.id().as_ref()) {
                error!(%err, "tray menu action failed");
            }
        });

    // macOS menu-bar icons should follow the system light/dark appearance.
    #[cfg(target_os = "macos")]
    let builder = builder.icon_as_template(true);

    builder
        .build(app)
        .map_err(|err| Error::Tray(err.to_string()))?;

    info!("tray icon installed");
    Ok(())
}

fn handle_menu(app: &AppHandle, id: &str) -> Result<()> {
    match id {
        "settings" => windows::show_settings(app),
        "toggle-pause" => {
            let paused = {
                let state = app.state::<AppState>();
                !state.paused.load(Ordering::Relaxed)
            };
            set_paused(app, paused)
        }
        "quit" => {
            info!("quit requested from tray");
            app.exit(0);
            Ok(())
        }
        other => {
            warn!(id = other, "unknown tray menu item");
            Ok(())
        }
    }
}

pub fn set_paused(app: &AppHandle, paused: bool) -> Result<()> {
    let state = app.state::<AppState>();
    state.paused.store(paused, Ordering::Relaxed);
    let label_update = state
        .pause_item
        .read()
        .as_ref()
        .map(|item| item.set_text(if paused { "Resume" } else { "Pause" }));
    drop(state);

    if let Some(result) = label_update {
        result?;
    }

    if paused {
        // Finish any clip already in flight so pausing never leaves the
        // microphone open. No recording is a normal case, so the error is
        // intentionally ignored.
        let _ = crate::recording::end(app);
    }

    app.emit(events::EVENT_APP_STATE, PausedEvent { paused })?;
    info!(paused, "pause state changed");
    Ok(())
}
