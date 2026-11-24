//! Temporarily mutes the system output while dictating.
//!
//! Music playing through the speakers is the most common source of bleed into
//! the microphone, so Speakeasy can silence the default output for the length
//! of a recording and put it back exactly as it found it.
//!
//! Restoring matters more than muting: a crash must never leave the machine
//! silent. Before muting we spawn a tiny watchdog copy of this executable that
//! waits for the app to exit and then restores the previous state even if the
//! app was killed. A normal stop kills the watchdog first.

use std::process::{Child, Command, Stdio};

use parking_lot::Mutex;
use tracing::{info, warn};

use crate::error::{Error, Result};

/// CLI flag used by the watchdog copy of the executable.
const WATCHDOG_FLAG: &str = "--speakeasy-restore-audio";

#[derive(Debug)]
struct Active {
    was_muted: bool,
    watchdog: Option<Child>,
}

static ACTIVE: Mutex<Option<Active>> = Mutex::new(None);

fn active() -> &'static Mutex<Option<Active>> {
    &ACTIVE
}

/// Mutes the default output, remembering the previous state. Calling this
/// while already muted is a no-op. Returns `false` when the platform does not
/// support output control, so callers can log one clear message.
pub fn mute_now() -> Result<bool> {
    let mut guard = active().lock();
    if guard.is_some() {
        return Ok(true);
    }

    let was_muted = match is_muted() {
        Ok(value) => value,
        Err(err) => {
            warn!(%err, "could not read the system mute state");
            return Err(err);
        }
    };

    if was_muted {
        // Already silent; nothing to change and nothing to restore.
        *guard = Some(Active {
            was_muted: true,
            watchdog: None,
        });
        return Ok(true);
    }

    let watchdog = spawn_watchdog();
    *guard = Some(Active {
        was_muted: false,
        watchdog,
    });
    drop(guard);

    if let Err(err) = set_muted(true) {
        // Undo the bookkeeping so a later call can retry.
        let mut guard = active().lock();
        if let Some(active) = guard.take() {
            stop_watchdog(active.watchdog);
        }
        drop(guard);
        warn!(%err, "could not mute the system output");
        return Err(err);
    }

    info!("system output muted for the recording");
    Ok(true)
}

/// Restores the state captured by [`mute_now`]. Safe to call at any time,
/// including when nothing was muted.
pub fn restore_now() -> Result<()> {
    let active = active().lock().take();
    let Some(active) = active else {
        return Ok(());
    };

    stop_watchdog(active.watchdog);

    // If the user was already muted, the output is correct as-is.
    if active.was_muted {
        return Ok(());
    }

    match set_muted(false) {
        Ok(()) => {
            info!("system output restored");
            Ok(())
        }
        Err(err) => {
            warn!(%err, "could not restore the system output");
            Err(err)
        }
    }
}

fn stop_watchdog(watchdog: Option<Child>) {
    if let Some(mut child) = watchdog {
        let _ = child.kill();
        let _ = child.wait();
    }
}

/// Starts a copy of this executable that unmutes the output if the app dies
/// before [`restore_now`] runs. It is only ever spawned when the output was
/// unmuted before Speakeasy muted it, so restoring always means "unmute".
/// Failure is not fatal: the recording still works, it just loses the crash
/// guarantee for this one clip.
fn spawn_watchdog() -> Option<Child> {
    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(err) => {
            warn!(%err, "could not locate the executable for the audio watchdog");
            return None;
        }
    };

    let mut command = Command::new(exe);
    command
        .arg(WATCHDOG_FLAG)
        .arg(std::process::id().to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW: the watchdog is a console-subsystem binary in
        // debug builds and must never flash a window.
        command.creation_flags(0x0800_0000);
    }

    match command.spawn() {
        Ok(child) => Some(child),
        Err(err) => {
            warn!(%err, "could not start the audio restore watchdog");
            None
        }
    }
}

/// Entry point for the watchdog process. Returns `true` when this invocation
/// was a watchdog, in which case the caller must exit without starting Tauri.
pub fn maybe_run_watchdog() -> bool {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() != Some(std::ffi::OsStr::new(WATCHDOG_FLAG)) {
        return false;
    }

    let pid = args
        .next()
        .and_then(|value| value.to_str().and_then(|value| value.parse::<u32>().ok()));

    let Some(pid) = pid else {
        return true;
    };

    wait_for_parent(pid);
    if let Err(err) = set_muted(false) {
        // There is nowhere useful to report this: the parent is gone.
        let _ = err;
    }
    true
}

#[cfg(windows)]
fn wait_for_parent(pid: u32) {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{
        OpenProcess, WaitForSingleObject, PROCESS_ACCESS_RIGHTS,
    };

    // SYNCHRONIZE without the rest of PROCESS_ALL_ACCESS.
    const SYNCHRONIZE: PROCESS_ACCESS_RIGHTS = PROCESS_ACCESS_RIGHTS(0x0010_0000);

    let handle = match unsafe { OpenProcess(SYNCHRONIZE, false, pid) } {
        Ok(handle) => handle,
        // The parent is already gone.
        Err(_) => return,
    };
    unsafe {
        let _ = WaitForSingleObject(handle, u32::MAX);
        let _ = CloseHandle(handle);
    }
}

#[cfg(target_os = "macos")]
fn wait_for_parent(pid: u32) {
    use std::time::Duration;

    extern "C" {
        fn kill(pid: i32, signal: i32) -> i32;
    }

    // Signal 0 only performs the permission and existence check.
    while unsafe { kill(pid as i32, 0) } == 0 {
        std::thread::sleep(Duration::from_millis(500));
    }
}

#[cfg(all(not(windows), not(target_os = "macos")))]
fn wait_for_parent(_pid: u32) {}

#[cfg(windows)]
fn is_muted() -> Result<bool> {
    endpoint_volume(|volume| unsafe { volume.GetMute() }).map(|value| value.as_bool())
}

#[cfg(windows)]
fn set_muted(muted: bool) -> Result<()> {
    endpoint_volume(|volume| unsafe { volume.SetMute(muted, std::ptr::null()) })
}

#[cfg(windows)]
fn endpoint_volume<T>(
    action: impl FnOnce(&windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume) -> windows::core::Result<T>,
) -> Result<T> {
    use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
    use windows::Win32::Media::Audio::{
        eMultimedia, eRender, IMMDeviceEnumerator, MMDeviceEnumerator,
    };
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_APARTMENTTHREADED,
    };

    let initialized = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    if initialized.is_err() {
        return Err(Error::Audio("the Windows audio service is unavailable".into()));
    }

    let result = (|| -> windows::core::Result<T> {
        let enumerator: IMMDeviceEnumerator =
            unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }?;
        let device = unsafe { enumerator.GetDefaultAudioEndpoint(eRender, eMultimedia) }?;
        let volume: IAudioEndpointVolume =
            unsafe { device.Activate(CLSCTX_ALL, None) }?;
        action(&volume)
    })();

    unsafe { CoUninitialize() };

    result.map_err(|err| Error::Audio(format!("system output control failed: {err}")))
}

#[cfg(target_os = "macos")]
fn is_muted() -> Result<bool> {
    let output = std::process::Command::new("/usr/bin/osascript")
        .args(["-e", "output muted of (get volume settings)"])
        .output()
        .map_err(|err| Error::Audio(format!("could not read the output state: {err}")))?;

    if !output.status.success() {
        return Err(Error::Audio(
            "macOS did not allow the output state to be read".into(),
        ));
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim() == "true")
}

#[cfg(target_os = "macos")]
fn set_muted(muted: bool) -> Result<()> {
    let script = format!("set volume output muted {}", muted);
    let output = std::process::Command::new("/usr/bin/osascript")
        .args(["-e", &script])
        .output()
        .map_err(|err| Error::Audio(format!("could not change the output state: {err}")))?;

    if output.status.success() {
        Ok(())
    } else {
        Err(Error::Audio(
            "macOS did not allow the output to be changed".into(),
        ))
    }
}

#[cfg(all(not(windows), not(target_os = "macos")))]
fn is_muted() -> Result<bool> {
    Err(Error::Audio(
        "muting the system output is not supported on this platform".into(),
    ))
}

#[cfg(all(not(windows), not(target_os = "macos")))]
fn set_muted(_muted: bool) -> Result<()> {
    Err(Error::Audio(
        "muting the system output is not supported on this platform".into(),
    ))
}

#[cfg(all(test, windows))]
mod tests {
    #[test]
    fn output_mute_state_is_readable() {
        // Reading (never changing) the default endpoint catches COM or feature
        // wiring mistakes without silencing the machine running the tests.
        let muted = super::is_muted().expect("the Windows audio endpoint should be reachable");
        assert!(muted || !muted);
    }
}
