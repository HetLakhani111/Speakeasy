//! Hold detection for modifier-only hotkeys.
//!
//! `global-hotkey` can only express a chord that ends in a real key, so a
//! binding such as `Ctrl` (or macOS `fn`) is served by a small platform hook
//! instead:
//!
//! * macOS installs a listen-only `CGEventTap` for `FlagsChanged` events. This
//!   is the only way to see the fn key, and it needs Input Monitoring.
//! * Windows installs a `WH_KEYBOARD_LL` hook. The hook never swallows keys:
//!   it forwards everything, so a held Ctrl still behaves like Ctrl.
//!
//! In both cases a worker thread serializes press and release so a very quick
//! tap can never deliver the release before the recorder has opened.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Modifier {
    Fn,
    Control,
    Alt,
    Shift,
    Meta,
}

impl Modifier {
    /// Parses a canonical modifier-only binding. `None` means the string is a
    /// normal hotkey chord and belongs to `global-hotkey`.
    pub fn parse(binding: &str) -> Option<Self> {
        match binding.trim().to_ascii_lowercase().as_str() {
            "fn" => Some(Self::Fn),
            "ctrl" | "control" => Some(Self::Control),
            "alt" | "option" => Some(Self::Alt),
            "shift" => Some(Self::Shift),
            "cmd" | "command" | "meta" | "super" | "win" | "windows" => Some(Self::Meta),
            _ => None,
        }
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use std::cell::Cell;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::Sender;
    use std::sync::Arc;
    use std::thread::{self, JoinHandle};
    use std::time::Duration;

    use core_foundation::runloop::{kCFRunLoopCommonModes, kCFRunLoopDefaultMode, CFRunLoop};
    use core_graphics::event::{
        CGEvent, CGEventFlags, CGEventTap, CGEventTapLocation, CGEventTapOptions,
        CGEventTapPlacement, CGEventTapProxy, CGEventType,
    };
    use tauri::AppHandle;
    use tracing::{info, warn};

    use super::Modifier;
    use crate::recording;

    pub struct ModifierTap {
        runloop: CFRunLoop,
        stopped: Arc<AtomicBool>,
        join: Option<JoinHandle<()>>,
    }

    impl ModifierTap {
        pub fn start(app: &AppHandle, modifier: Modifier) -> Result<Self, String> {
            let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<CFRunLoop, String>>();
            let stopped = Arc::new(AtomicBool::new(false));
            let thread_stopped = Arc::clone(&stopped);
            let app = app.clone();

            let join = thread::Builder::new()
                .name("speakeasy-modifier-tap".into())
                .spawn(move || run_tap(app, modifier, thread_stopped, ready_tx))
                .map_err(|err| format!("could not start the modifier monitor: {err}"))?;

            match ready_rx.recv_timeout(Duration::from_secs(5)) {
                Ok(Ok(runloop)) => Ok(Self {
                    runloop,
                    stopped,
                    join: Some(join),
                }),
                Ok(Err(message)) => {
                    stopped.store(true, Ordering::Relaxed);
                    let _ = join.join();
                    Err(message)
                }
                Err(_) => {
                    stopped.store(true, Ordering::Relaxed);
                    Err(
                        "the modifier monitor did not start; check Input Monitoring permission."
                            .into(),
                    )
                }
            }
        }

        pub fn stop(self) {
            drop(self);
        }

        fn shutdown(&mut self) {
            self.stopped.store(true, Ordering::Relaxed);
            self.runloop.stop();
            if let Some(join) = self.join.take() {
                let _ = join.join();
            }
        }
    }

    impl Drop for ModifierTap {
        fn drop(&mut self) {
            self.shutdown();
        }
    }

    fn flag_for(modifier: Modifier) -> CGEventFlags {
        match modifier {
            Modifier::Fn => CGEventFlags::CGEventFlagSecondaryFn,
            Modifier::Control => CGEventFlags::CGEventFlagControl,
            Modifier::Alt => CGEventFlags::CGEventFlagAlternate,
            Modifier::Shift => CGEventFlags::CGEventFlagShift,
            Modifier::Meta => CGEventFlags::CGEventFlagCommand,
        }
    }

    fn run_tap(
        app: AppHandle,
        modifier: Modifier,
        stopped: Arc<AtomicBool>,
        ready: Sender<Result<CFRunLoop, String>>,
    ) {
        let runloop = CFRunLoop::get_current();
        let (event_tx, event_rx) = crossbeam_channel::unbounded::<bool>();

        let worker = thread::Builder::new()
            .name("speakeasy-modifier-events".into())
            .spawn(move || {
                while let Ok(pressed) = event_rx.recv() {
                    if pressed {
                        recording::on_hotkey_press(&app);
                    } else {
                        recording::on_hotkey_release(&app);
                    }
                }
            });

        let worker = match worker {
            Ok(worker) => worker,
            Err(err) => {
                let _ = ready.send(Err(format!(
                    "could not start the modifier worker: {err}"
                )));
                return;
            }
        };

        let flag = flag_for(modifier);
        let was_down = Cell::new(false);
        let tap = CGEventTap::new(
            CGEventTapLocation::HID,
            CGEventTapPlacement::HeadInsertEventTap,
            CGEventTapOptions::ListenOnly,
            vec![CGEventType::FlagsChanged],
            move |_proxy: CGEventTapProxy, _kind: CGEventType, event: &CGEvent| {
                let down = event.get_flags().contains(flag);
                if down != was_down.get() {
                    was_down.set(down);
                    let _ = event_tx.try_send(down);
                }
                None
            },
        );

        let tap = match tap {
            Ok(tap) => tap,
            Err(()) => {
                warn!("macOS refused the modifier event tap");
                let _ = ready.send(Err(
                    "macOS blocked the hotkey monitor. Allow Speakeasy under System Settings → Privacy & Security → Input Monitoring, then try again."
                        .into(),
                ));
                let _ = worker.join();
                return;
            }
        };

        let source = match tap.mach_port.create_runloop_source(0) {
            Ok(source) => source,
            Err(()) => {
                let _ = ready.send(Err(
                    "could not attach the modifier monitor to a run loop".into(),
                ));
                drop(tap);
                let _ = worker.join();
                return;
            }
        };

        // The run loop mode constants are `extern static`s, which recent Rust
        // requires us to touch inside `unsafe`.
        unsafe { runloop.add_source(&source, kCFRunLoopCommonModes) };
        tap.enable();

        if ready.send(Ok(runloop.clone())).is_err() || stopped.load(Ordering::Relaxed) {
            drop(source);
            drop(tap);
            let _ = worker.join();
            return;
        }

        info!("modifier-only hotkey monitor running");

        while !stopped.load(Ordering::Relaxed) {
            let _ = unsafe {
                CFRunLoop::run_in_mode(kCFRunLoopDefaultMode, Duration::from_millis(250), false)
            };
        }

        drop(source);
        drop(tap);
        let _ = worker.join();
    }
}

#[cfg(windows)]
mod platform {
    use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
    use std::sync::mpsc::Sender;
    use std::sync::Arc;
    use std::thread::{self, JoinHandle};
    use std::time::Duration;

    use parking_lot::Mutex;
    use tauri::AppHandle;
    use tracing::{info, warn};
    use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::Threading::GetCurrentThreadId;
    use windows::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, DispatchMessageW, GetMessageW, PeekMessageW, PostThreadMessageW,
        SetWindowsHookExW, TranslateMessage, HC_ACTION, KBDLLHOOKSTRUCT, LLKHF_INJECTED, MSG,
        PM_NOREMOVE, WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP, WM_QUIT, WM_SYSKEYDOWN, WM_SYSKEYUP,
    };

    use super::Modifier;
    use crate::recording;

    const NONE: u8 = 0;
    const CONTROL: u8 = 1;
    const ALT: u8 = 2;
    const SHIFT: u8 = 3;
    const META: u8 = 4;

    // Virtual-key codes. Raw values keep the generated API surface small.
    const VK_SHIFT: u32 = 0x10;
    const VK_CONTROL: u32 = 0x11;
    const VK_MENU: u32 = 0x12;
    const VK_LSHIFT: u32 = 0xA0;
    const VK_RSHIFT: u32 = 0xA1;
    const VK_LCONTROL: u32 = 0xA2;
    const VK_RCONTROL: u32 = 0xA3;
    const VK_LMENU: u32 = 0xA4;
    const VK_RMENU: u32 = 0xA5;
    const VK_LWIN: u32 = 0x5B;
    const VK_RWIN: u32 = 0x5C;

    static TARGET: AtomicU8 = AtomicU8::new(NONE);
    static WAS_DOWN: AtomicBool = AtomicBool::new(false);
    static EVENT_TX: Mutex<Option<crossbeam_channel::Sender<bool>>> = Mutex::new(None);

    pub struct ModifierTap {
        thread_id: u32,
        stopped: Arc<AtomicBool>,
        join: Option<JoinHandle<()>>,
    }

    impl ModifierTap {
        pub fn start(app: &AppHandle, modifier: Modifier) -> Result<Self, String> {
            let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<u32, String>>();
            let stopped = Arc::new(AtomicBool::new(false));
            let thread_stopped = Arc::clone(&stopped);
            let app = app.clone();

            let join = thread::Builder::new()
                .name("speakeasy-modifier-hook".into())
                .spawn(move || run_hook(app, modifier, thread_stopped, ready_tx))
                .map_err(|err| format!("could not start the modifier monitor: {err}"))?;

            match ready_rx.recv_timeout(Duration::from_secs(5)) {
                Ok(Ok(thread_id)) => Ok(Self {
                    thread_id,
                    stopped,
                    join: Some(join),
                }),
                Ok(Err(message)) => {
                    stopped.store(true, Ordering::Relaxed);
                    let _ = join.join();
                    Err(message)
                }
                Err(_) => {
                    stopped.store(true, Ordering::Relaxed);
                    Err("the modifier monitor did not start".into())
                }
            }
        }

        pub fn stop(self) {
            drop(self);
        }

        fn shutdown(&mut self) {
            self.stopped.store(true, Ordering::Relaxed);
            // GetMessage blocks; a posted WM_QUIT is the clean way to wake it.
            let _ = unsafe {
                PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0))
            };
            if let Some(join) = self.join.take() {
                let _ = join.join();
            }
        }
    }

    impl Drop for ModifierTap {
        fn drop(&mut self) {
            self.shutdown();
        }
    }

    fn code_for(modifier: Modifier) -> u8 {
        match modifier {
            Modifier::Control => CONTROL,
            Modifier::Alt => ALT,
            Modifier::Shift => SHIFT,
            Modifier::Meta => META,
            // Windows has no fn key; the caller rejects this binding first.
            Modifier::Fn => NONE,
        }
    }

    fn matches(target: u8, vk: u32) -> bool {
        match target {
            CONTROL => matches!(vk, VK_CONTROL | VK_LCONTROL | VK_RCONTROL),
            ALT => matches!(vk, VK_MENU | VK_LMENU | VK_RMENU),
            SHIFT => matches!(vk, VK_SHIFT | VK_LSHIFT | VK_RSHIFT),
            META => matches!(vk, VK_LWIN | VK_RWIN),
            _ => false,
        }
    }

    fn publish(pressed: bool) {
        if pressed == WAS_DOWN.swap(pressed, Ordering::SeqCst) {
            return;
        }
        if let Some(tx) = EVENT_TX.lock().as_ref() {
            let _ = tx.try_send(pressed);
        }
    }

    /// Development builds can opt into synthetic modifier presses so the hook
    /// can be exercised by an automation script. Release builds always ignore
    /// injected input.
    fn accepts_injected() -> bool {
        cfg!(debug_assertions)
            && std::env::var_os("SPEAKEASY_ALLOW_INJECTED_MODIFIERS").is_some()
    }

    unsafe extern "system" fn keyboard_proc(
        code: i32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        if code == HC_ACTION as i32 {
            let info = unsafe { *(lparam.0 as *const KBDLLHOOKSTRUCT) };
            let injected = info.flags.contains(LLKHF_INJECTED) && !accepts_injected();
            if !injected && matches(TARGET.load(Ordering::SeqCst), info.vkCode) {
                match wparam.0 as u32 {
                    WM_KEYDOWN | WM_SYSKEYDOWN => publish(true),
                    WM_KEYUP | WM_SYSKEYUP => publish(false),
                    _ => {}
                }
            }
        }

        unsafe { CallNextHookEx(None, code, wparam, lparam) }
    }

    fn run_hook(
        app: AppHandle,
        modifier: Modifier,
        stopped: Arc<AtomicBool>,
        ready: Sender<Result<u32, String>>,
    ) {
        let (tx, rx) = crossbeam_channel::unbounded::<bool>();
        let worker = thread::Builder::new()
            .name("speakeasy-modifier-events".into())
            .spawn(move || {
                while let Ok(pressed) = rx.recv() {
                    if pressed {
                        recording::on_hotkey_press(&app);
                    } else {
                        recording::on_hotkey_release(&app);
                    }
                }
            });

        let worker = match worker {
            Ok(worker) => worker,
            Err(err) => {
                let _ = ready.send(Err(format!(
                    "could not start the modifier worker: {err}"
                )));
                return;
            }
        };

        *EVENT_TX.lock() = Some(tx);
        TARGET.store(code_for(modifier), Ordering::SeqCst);
        WAS_DOWN.store(false, Ordering::SeqCst);

        let hook = match unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_proc), None, 0) }
        {
            Ok(hook) => hook,
            Err(err) => {
                TARGET.store(NONE, Ordering::SeqCst);
                *EVENT_TX.lock() = None;
                let _ = ready.send(Err(format!(
                    "could not install the keyboard monitor: {err}"
                )));
                let _ = worker.join();
                return;
            }
        };

        // Force the message queue to exist before stop() can post to it.
        let mut message = MSG::default();
        unsafe {
            let _ = PeekMessageW(&mut message, None, 0, 0, PM_NOREMOVE);
        }

        let thread_id = unsafe { GetCurrentThreadId() };
        if ready.send(Ok(thread_id)).is_err() {
            unsafe {
                let _ = windows::Win32::UI::WindowsAndMessaging::UnhookWindowsHookEx(hook);
            }
            TARGET.store(NONE, Ordering::SeqCst);
            *EVENT_TX.lock() = None;
            let _ = worker.join();
            return;
        }

        info!("modifier-only hotkey monitor running");

        while !stopped.load(Ordering::Relaxed) {
            let result = unsafe { GetMessageW(&mut message, None, 0, 0) };
            if result.0 <= 0 {
                break;
            }
            unsafe {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }

        unsafe {
            let _ = windows::Win32::UI::WindowsAndMessaging::UnhookWindowsHookEx(hook);
        }
        TARGET.store(NONE, Ordering::SeqCst);
        *EVENT_TX.lock() = None;
        let _ = worker.join();
        warn!("modifier-only hotkey monitor stopped");
    }
}

pub use platform::ModifierTap;
