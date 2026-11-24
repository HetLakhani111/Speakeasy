//! The recording overlay window.
//!
//! A frameless, transparent, click-through capsule pinned to the bottom centre
//! of the primary display. It is shown while the user holds the hotkey and the
//! frontend hides it again when the transcript lands or the pipeline reports a
//! problem. Nothing here ever takes keyboard focus: the whole point is that the
//! app the user is dictating into keeps it.

use tauri::{AppHandle, Manager, PhysicalPosition, WebviewWindow};
use tracing::warn;

use crate::error::Result;
pub const OVERLAY_LABEL: &str = "overlay";

/// Gap between the bottom of the capsule and the bottom of the screen, in
/// logical pixels. Leaves room for a Windows taskbar or the macOS Dock.
const BOTTOM_MARGIN: f64 = 72.0;

fn window(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window(OVERLAY_LABEL)
}

/// Applies the properties that must hold for the whole life of the overlay.
pub fn init(app: &AppHandle) -> Result<()> {
    let Some(window) = window(app) else {
        warn!("the overlay window is missing from the configuration");
        return Ok(());
    };

    window.set_ignore_cursor_events(true)?;
    window.set_always_on_top(true)?;
    // `set_focusable(false)` maps to WS_EX_NOACTIVATE on Windows and keeps the
    // window out of the focus ring on macOS.
    if let Err(err) = window.set_focusable(false) {
        warn!(%err, "could not make the overlay non-focusable");
    }
    place(&window)?;
    Ok(())
}

/// Shows the overlay for a new dictation. Safe to call while it is already up.
pub fn show(app: &AppHandle) -> Result<()> {
    let Some(window) = window(app) else {
        return Ok(());
    };
    place(&window)?;
    window.show()?;
    Ok(())
}

/// Hides the overlay. Called by the frontend once the outcome has been shown.
pub fn hide(app: &AppHandle) -> Result<()> {
    let Some(window) = window(app) else {
        return Ok(());
    };
    window.hide()?;
    Ok(())
}

/// Pins the capsule to the bottom centre of the primary monitor. Recomputed on
/// every show so resolution and scaling changes are handled.
fn place(window: &WebviewWindow) -> Result<()> {
    let Some(monitor) = window.primary_monitor()? else {
        return Ok(());
    };

    let scale = monitor.scale_factor();
    let screen = monitor.size();
    let origin = monitor.position();
    let size = window.outer_size()?;

    let (x, y) = bottom_center(
        (origin.x, origin.y),
        (screen.width, screen.height),
        (size.width, size.height),
        scale,
    );
    window.set_position(PhysicalPosition::new(x, y))?;
    Ok(())
}

/// Bottom-centre anchor maths, kept separate so it can be tested without a
/// display. All arguments and results are physical pixels.
fn bottom_center(
    screen_origin: (i32, i32),
    screen_size: (u32, u32),
    window_size: (u32, u32),
    scale: f64,
) -> (i32, i32) {
    let margin = (BOTTOM_MARGIN * scale).round() as i32;
    let x = screen_origin.0 + (screen_size.0 as i32 - window_size.0 as i32) / 2;
    let y = screen_origin.1 + screen_size.1 as i32 - window_size.1 as i32 - margin;
    (x, y)
}

/// Debug-only preview: `SPEAKEASY_DEV_OVERLAY=1` plays a synthetic dictation so
/// the overlay can be inspected (and screenshotted) without a microphone.
#[cfg(debug_assertions)]
pub fn maybe_preview(app: &AppHandle) {
    if std::env::var_os("SPEAKEASY_DEV_OVERLAY").is_none() {
        return;
    }

    let app = app.clone();
    let spawned = {
        use std::thread;
        use std::time::Duration;

        use tauri::Emitter;

        thread::Builder::new()
            .name("speakeasy-overlay-preview".into())
            .spawn(move || {
                use crate::audio::meter::AudioLevels;
                use crate::events::{self, RecordingStarted, RecordingStopped, TranscriptEvent};

                // Let the webview finish loading so it receives the first events;
                // a preview that starts before the listener binds would leave the
                // capsule blank.
                thread::sleep(Duration::from_millis(2500));

                if let Err(err) = show(&app) {
                    warn!(%err, "could not show the overlay preview");
                    return;
                }

                if let Err(err) = app.emit(
                    events::EVENT_RECORDING_STARTED,
                    RecordingStarted {
                        device: Some("Preview".into()),
                    },
                ) {
                    warn!(%err, "could not emit the preview start event");
                }

                // Six seconds of a plausible voice spectrum: a few moving formants
                // over a periodic envelope.
                let frames = 360;
                for frame in 0..frames {
                    let t = frame as f32 / 60.0;
                    let envelope = (t * 2.4).sin().abs() * 0.65 + 0.2;
                    let bins = (0..64)
                        .map(|band| {
                            let position = band as f32 / 63.0;
                            let tilt = (1.0 - position).powf(1.4);
                            let wobble = ((t * 3.1) + band as f32 * 0.42).sin().abs();
                            let formant = (((position - 0.16) * 9.0).sin()
                                + ((position - 0.42) * 7.0).sin())
                            .abs()
                                * 0.35;
                            ((tilt * (0.45 + formant) + wobble * 0.22) * envelope).clamp(0.0, 1.0)
                        })
                        .collect::<Vec<_>>();
                    events::emit_levels(
                        &app,
                        AudioLevels {
                            level: envelope * 0.8,
                            bins,
                        },
                    );
                    thread::sleep(Duration::from_millis(16));
                }

                events::emit_recording_stopped(
                    &app,
                    RecordingStopped {
                        duration_ms: 6000,
                        captured_ms: 6200,
                        peak: 0.74,
                        wav_path: None,
                    },
                );

                thread::sleep(Duration::from_millis(1400));

                events::emit_transcript(
                    &app,
                    TranscriptEvent {
                        text: "The overlay preview is working as intended.".into(),
                        audio_ms: 6000,
                        load_ms: 0,
                        inference_ms: 320,
                        total_ms: 480,
                        wav_path: None,
                    },
                );
            })
    };

    if let Err(err) = spawned {
        warn!(%err, "could not start the overlay preview");
    }
}

#[cfg(not(debug_assertions))]
pub fn maybe_preview(_app: &AppHandle) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn centres_on_a_single_display() {
        // 1920x1080 at 1x with a 480x140 capsule and a 72 px margin.
        assert_eq!(
            bottom_center((0, 0), (1920, 1080), (480, 140), 1.0),
            (720, 868)
        );
    }

    #[test]
    fn accounts_for_scaling_and_display_origin() {
        // A 2x display starting at x=1920: the capsule is 960x280 physical
        // pixels and the margin doubles.
        assert_eq!(
            bottom_center((1920, 0), (2560, 1440), (960, 280), 2.0),
            (2720, 1016)
        );
    }
}
