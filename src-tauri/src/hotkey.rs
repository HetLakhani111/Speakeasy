//! Global hold-to-talk hotkey.
//!
//! Both press and release are required: press starts recording, release ends
//! it. `global-hotkey` provides those events on Windows and macOS, except for
//! the fn key, which macOS does not expose as a Carbon hotkey. That binding is
//! handled by a small CGEventTap in `fn_tap` on macOS.
//!
//! `global-hotkey` must be created on a thread that runs the platform event
//! loop, and on Windows the manager owns an `HWND`, which is neither `Send` nor
//! `Sync`. The manager therefore lives in a thread-local on the main thread.
//! Everything else (settings, status) is plain data stored in `AppState`.
//! Other threads call [`request_apply`], which hops to the main thread and
//! waits for the result.

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{mpsc, OnceLock};
use std::thread::{self, ThreadId};
use std::time::Duration;

use global_hotkey::hotkey::HotKey;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState as GlobalState};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tracing::{error, info, warn};

use crate::error::{Error, Result};
use crate::events::{self, NotificationLevel};
use crate::{recording, AppState};

#[cfg(any(target_os = "macos", windows))]
mod modifier_tap;

/// Id of the binding the listener should act on. Zero means "ignore
/// everything", which is also the state while no binding is active.
static CURRENT_ID: AtomicU32 = AtomicU32::new(0);
static LISTENER_STARTED: AtomicBool = AtomicBool::new(false);
static MAIN_THREAD: OnceLock<ThreadId> = OnceLock::new();

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HotkeyStatus {
    pub binding: String,
    pub registered: bool,
    /// Set when the binding could not be parsed or registered.
    pub error: Option<String>,
    /// True when this binding is served by a platform event hook rather than
    /// `global-hotkey`. Modifier-only bindings and the macOS fn key use this.
    pub native: bool,
}

impl HotkeyStatus {
    fn failed(binding: &str, error: impl Into<String>) -> Self {
        Self {
            binding: binding.to_string(),
            registered: false,
            error: Some(error.into()),
            native: false,
        }
    }
}

struct ManagerSlot {
    manager: Option<GlobalHotKeyManager>,
    manager_error: Option<String>,
    current: Option<HotKey>,
    #[cfg(any(target_os = "macos", windows))]
    modifier_tap: Option<modifier_tap::ModifierTap>,
}

impl ManagerSlot {
    fn new() -> Self {
        let (manager, manager_error) = match GlobalHotKeyManager::new() {
            Ok(manager) => (Some(manager), None),
            Err(err) => (None, Some(err.to_string())),
        };

        Self {
            manager,
            manager_error,
            current: None,
            #[cfg(any(target_os = "macos", windows))]
            modifier_tap: None,
        }
    }

    /// Registers `binding`, replacing any previous binding. The returned
    /// status always describes the attempt; it never panics or aborts startup.
    fn register(&mut self, _app: &AppHandle, binding: &str) -> HotkeyStatus {
        let binding = binding.trim();

        if binding.is_empty() {
            self.unregister_current();
            return HotkeyStatus::failed(binding, "No hotkey is set.");
        }

        #[cfg(any(target_os = "macos", windows))]
        if let Some(modifier) = modifier_tap::Modifier::parse(binding) {
            #[cfg(windows)]
            if modifier == modifier_tap::Modifier::Fn {
                self.unregister_current();
                return HotkeyStatus::failed(
                    binding,
                    "The fn key is only available on macOS.",
                );
            }

            self.unregister_current();
            return match modifier_tap::ModifierTap::start(_app, modifier) {
                Ok(tap) => {
                    self.modifier_tap = Some(tap);
                    info!(binding, "modifier-only hotkey registered");
                    HotkeyStatus {
                        binding: binding.to_string(),
                        registered: true,
                        error: None,
                        native: true,
                    }
                }
                Err(err) => HotkeyStatus::failed(binding, err),
            };
        }

        #[cfg(not(any(target_os = "macos", windows)))]
        if binding.eq_ignore_ascii_case("fn") {
            self.unregister_current();
            return HotkeyStatus::failed(binding, "The fn key is only available on macOS.");
        }

        let hotkey: HotKey = match binding.parse() {
            Ok(hotkey) => hotkey,
            Err(err) => {
                self.unregister_current();
                return HotkeyStatus::failed(binding, format!("Unrecognized hotkey: {err}"));
            }
        };

        if self.manager.is_none() {
            let message = self
                .manager_error
                .clone()
                .unwrap_or_else(|| "Global hotkeys are unavailable.".into());
            return HotkeyStatus::failed(binding, message);
        };

        self.unregister_current();

        let Some(manager) = self.manager.as_ref() else {
            return HotkeyStatus::failed(binding, "Global hotkeys are unavailable.");
        };

        match manager.register(hotkey) {
            Ok(()) => {
                self.current = Some(hotkey);
                CURRENT_ID.store(hotkey.id(), Ordering::Relaxed);
                info!(binding, id = hotkey.id(), "hotkey registered");
                HotkeyStatus {
                    binding: binding.to_string(),
                    registered: true,
                    error: None,
                    native: false,
                }
            }
            Err(err) => {
                let mut message = format!("{err}");
                if cfg!(target_os = "windows") && binding.eq_ignore_ascii_case("ctrl+space") {
                    message.push_str(
                        " Ctrl+Space is frequently claimed by the Windows input method; choose a different shortcut.",
                    );
                }
                warn!(binding, %err, "hotkey registration failed");
                HotkeyStatus::failed(binding, message)
            }
        }
    }

    fn unregister_current(&mut self) {
        #[cfg(any(target_os = "macos", windows))]
        if let Some(tap) = self.modifier_tap.take() {
            tap.stop();
        }

        CURRENT_ID.store(0, Ordering::Relaxed);

        if let (Some(manager), Some(hotkey)) = (self.manager.as_ref(), self.current.take()) {
            if let Err(err) = manager.unregister(hotkey) {
                warn!(%err, "could not unregister the previous hotkey");
            }
        }
    }
}

thread_local! {
    static MANAGER: RefCell<Option<ManagerSlot>> = const { RefCell::new(None) };
}

fn is_main_thread() -> bool {
    MAIN_THREAD
        .get()
        .is_some_and(|id| *id == thread::current().id())
}

fn ensure_listener(app: &AppHandle) {
    if LISTENER_STARTED.swap(true, Ordering::SeqCst) {
        return;
    }

    let app = app.clone();
    let result = thread::Builder::new()
        .name("speakeasy-hotkey".into())
        .spawn(move || {
            let receiver = GlobalHotKeyEvent::receiver();
            while let Ok(event) = receiver.recv() {
                if event.id != CURRENT_ID.load(Ordering::Relaxed) {
                    continue;
                }

                match event.state {
                    GlobalState::Pressed => recording::on_hotkey_press(&app),
                    GlobalState::Released => recording::on_hotkey_release(&app),
                }
            }
        });

    if let Err(err) = result {
        LISTENER_STARTED.store(false, Ordering::SeqCst);
        error!(%err, "could not start the hotkey listener");
    }
}

/// Creates the manager and registers the saved binding. Main thread only.
pub fn start(app: &AppHandle) {
    let _ = MAIN_THREAD.set(thread::current().id());

    MANAGER.with(|cell| {
        let mut slot = cell.borrow_mut();
        if slot.is_none() {
            *slot = Some(ManagerSlot::new());
        }
    });

    if !is_main_thread() {
        error!("hotkey startup ran off the main thread; global hotkeys will not work");
    }

    ensure_listener(app);

    let binding = app.state::<AppState>().settings.get().hotkey;
    let status = apply_on_main(app, &binding);

    if status.registered {
        info!(binding = %status.binding, native = status.native, "hold-to-talk ready");
    } else {
        warn!(
            binding = %status.binding,
            error = ?status.error,
            "hold-to-talk is not registered"
        );
    }
}

/// Registers `binding`, replacing the previous one. Must run on the main thread.
fn apply_on_main(app: &AppHandle, binding: &str) -> HotkeyStatus {
    if !is_main_thread() {
        return HotkeyStatus::failed(
            binding,
            "Hotkeys can only be registered on the main thread.",
        );
    }

    let status = MANAGER.with(|cell| {
        let mut guard = cell.borrow_mut();
        match guard.as_mut() {
            Some(slot) => slot.register(app, binding),
            None => HotkeyStatus::failed(binding, "Hotkeys were not initialized."),
        }
    });

    *app.state::<AppState>().hotkey_status.lock() = Some(status.clone());
    publish(app, &status);
    status
}

/// Registers `binding` from any thread, returning the resulting status.
pub fn request_apply(app: &AppHandle, binding: &str) -> Result<HotkeyStatus> {
    if is_main_thread() {
        return Ok(apply_on_main(app, binding));
    }

    let (tx, rx) = mpsc::channel();
    let binding = binding.to_string();
    let main_app = app.clone();

    app.run_on_main_thread(move || {
        let status = apply_on_main(&main_app, &binding);
        let _ = tx.send(status);
    })
    .map_err(|err| Error::Hotkey(format!("could not reach the main thread: {err}")))?;

    rx.recv_timeout(Duration::from_secs(5))
        .map_err(|_| Error::Hotkey("timed out while registering the hotkey".into()))
}

pub fn status(app: &AppHandle) -> Option<HotkeyStatus> {
    app.state::<AppState>().hotkey_status.lock().clone()
}

fn publish(app: &AppHandle, status: &HotkeyStatus) {
    if !status.registered {
        if let Some(message) = status.error.as_ref() {
            events::notify(app, NotificationLevel::Warning, message.clone());
        }
    }

    if let Err(err) = app.emit(events::EVENT_HOTKEY_STATUS, status) {
        warn!(%err, "could not emit hotkey status");
    }
}
