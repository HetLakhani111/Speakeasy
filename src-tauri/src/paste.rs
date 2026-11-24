//! Clipboard-and-keystroke paste.
//!
//! Typing long transcripts character by character is slow and layout
//! dependent, so the transcript goes to the clipboard and we send the paste
//! shortcut instead. The previous clipboard text is put back a moment later so
//! dictation never silently destroys what the user had copied.

use std::fmt::Display;
use std::thread;
use std::time::Duration;

use enigo::{Direction, Enigo, Key, Keyboard, Settings};
use tauri::AppHandle;
use tauri_plugin_clipboard_manager::ClipboardExt;
use tracing::warn;

use crate::error::{Error, Result};

/// Give the OS clipboard a moment to publish before the target app reads it.
const CLIPBOARD_SETTLE: Duration = Duration::from_millis(40);
/// Let the focused app finish handling the paste before restoring the old
/// clipboard contents.
const PASTE_SETTLE: Duration = Duration::from_millis(250);

pub fn paste(app: &AppHandle, text: &str) -> Result<()> {
    let clipboard = app.clipboard();

    // Only plain text can be preserved through this API. When the clipboard
    // holds an image, files, or nothing readable, typing the transcript is the
    // only way to paste without destroying what the user had copied.
    let previous = match clipboard.read_text() {
        Ok(previous) if previous != text => Some(previous),
        Ok(_) => None,
        Err(err) => {
            warn!(%err, "clipboard text cannot be restored; typing the transcript instead");
            return type_text(text);
        }
    };

    clipboard
        .write_text(text.to_string())
        .map_err(|err| Error::Paste(format!("could not write to the clipboard: {err}")))?;

    thread::sleep(CLIPBOARD_SETTLE);

    let mut enigo = match new_enigo() {
        Ok(enigo) => enigo,
        Err(err) => {
            restore(app, previous);
            return Err(err);
        }
    };

    let modifier = if cfg!(target_os = "macos") {
        Key::Meta
    } else {
        Key::Control
    };

    if let Err(err) = enigo.key(modifier, Direction::Press) {
        restore(app, previous);
        return Err(simulate_error(err));
    }

    if let Err(err) = enigo.key(Key::Unicode('v'), Direction::Click) {
        // Never leave a modifier stuck down.
        let _ = enigo.key(modifier, Direction::Release);
        restore(app, previous);
        return Err(simulate_error(err));
    }

    if let Err(err) = enigo.key(modifier, Direction::Release) {
        restore(app, previous);
        return Err(simulate_error(err));
    }

    thread::sleep(PASTE_SETTLE);
    restore(app, previous);
    Ok(())
}

fn new_enigo() -> Result<Enigo> {
    Enigo::new(&Settings::default()).map_err(simulate_error)
}

fn type_text(text: &str) -> Result<()> {
    let mut enigo = new_enigo()?;
    enigo.text(text).map_err(simulate_error)
}

fn restore(app: &AppHandle, previous: Option<String>) {
    if let Some(previous) = previous {
        if let Err(err) = app.clipboard().write_text(previous) {
            warn!(%err, "could not restore the previous clipboard text");
        }
    }
}

fn simulate_error(err: impl Display) -> Error {
    #[cfg(target_os = "macos")]
    let hint = " Grant Accessibility permission to Speakeasy in System Settings → Privacy & Security → Accessibility, then try again.";
    #[cfg(not(target_os = "macos"))]
    let hint = "";

    Error::Paste(format!("{err}.{hint}"))
}
