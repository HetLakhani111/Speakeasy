// Prevents an extra console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // A copy of this executable spawned to restore the system mute state runs
    // no part of the app; it waits for the parent to exit and then quits.
    if speakeasy_lib::run_audio_watchdog() {
        return;
    }

    speakeasy_lib::run();
}
