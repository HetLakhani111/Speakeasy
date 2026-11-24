use tauri::{AppHandle, Manager, Window, WindowEvent};

use crate::error::{Error, Result};

/// macOS runs as an accessory app: no Dock icon, only the menu-bar item.
pub fn apply_platform_chrome(app: &AppHandle) -> Result<()> {
    #[cfg(target_os = "macos")]
    app.set_activation_policy(tauri::ActivationPolicy::Accessory)?;

    #[cfg(not(target_os = "macos"))]
    let _ = app;

    Ok(())
}

/// Development nicety: `SPEAKEASY_DEV_OPEN_SETTINGS=1` opens the settings
/// window on launch so the UI can be inspected without clicking the tray.
pub fn maybe_open_for_development(app: &AppHandle) {
    if !cfg!(debug_assertions) {
        return;
    }

    if std::env::var_os("SPEAKEASY_DEV_OPEN_SETTINGS").is_some() {
        if let Err(err) = show_settings(app) {
            tracing::error!(%err, "could not open settings for development");
        }
    }
}

pub fn show_settings(app: &AppHandle) -> Result<()> {
    let window = app
        .get_webview_window("settings")
        .ok_or_else(|| Error::WindowMissing("settings".into()))?;

    window.show()?;
    window.unminimize()?;

    // An accessory app must briefly become regular for its windows to accept
    // keyboard focus; the Dock icon disappears again when settings close.
    #[cfg(target_os = "macos")]
    app.set_activation_policy(tauri::ActivationPolicy::Regular)?;

    window.set_focus()?;
    Ok(())
}

pub fn hide_settings(app: &AppHandle) -> Result<()> {
    let Some(window) = app.get_webview_window("settings") else {
        return Ok(());
    };

    window.hide()?;

    #[cfg(target_os = "macos")]
    app.set_activation_policy(tauri::ActivationPolicy::Accessory)?;

    Ok(())
}

/// Closing the settings window puts Speakeasy back in the tray instead of
/// shutting it down.
pub fn on_window_event(window: &Window, event: &WindowEvent) {
    if window.label() != "settings" {
        return;
    }

    if let WindowEvent::CloseRequested { api, .. } = event {
        api.prevent_close();
        let app = window.app_handle();
        if let Err(err) = hide_settings(app) {
            tracing::error!(%err, "could not hide the settings window");
        }
    }
}
