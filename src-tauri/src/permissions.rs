//! Opening the right privacy panes and checking macOS Accessibility access.
//!
//! Onboarding links straight to the exact pane instead of asking the user to
//! find it. The checks are advisory: if a check fails the UI shows the button
//! to retry after granting access.

#[cfg(target_os = "macos")]
use std::process::Command;

use crate::error::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    Microphone,
    Accessibility,
    InputMonitoring,
}

impl Pane {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "microphone" => Some(Self::Microphone),
            "accessibility" => Some(Self::Accessibility),
            "input-monitoring" => Some(Self::InputMonitoring),
            _ => None,
        }
    }
}

/// Opens the operating system's privacy settings for `pane`.
pub fn open(pane: Pane) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        let anchor = match pane {
            Pane::Microphone => "Privacy_Microphone",
            Pane::Accessibility => "Privacy_Accessibility",
            Pane::InputMonitoring => "Privacy_ListenEvent",
        };
        let url = format!("x-apple.systempreferences:com.apple.preference.security?{anchor}");
        return spawn("/usr/bin/open", &[url.as_str()]);
    }

    #[cfg(windows)]
    {
        let url = match pane {
            // Windows has no per-app accessibility prompt; microphone access
            // and input monitoring both live under the microphone privacy page.
            Pane::Microphone | Pane::Accessibility | Pane::InputMonitoring => {
                "ms-settings:privacy-microphone"
            }
        };
        return shell_open(url);
    }

    #[cfg(not(any(target_os = "macos", windows)))]
    {
        let _ = pane;
        Err(Error::Invalid(
            "opening privacy settings is not supported on this platform".into(),
        ))
    }
}

/// True when Accessibility access has been granted. Always true outside macOS,
/// where the paste path does not need an explicit grant.
pub fn accessibility_trusted() -> bool {
    #[cfg(target_os = "macos")]
    {
        extern "C" {
            /// ApplicationServices; returns a CFBoolean-style byte.
            fn AXIsProcessTrusted() -> u8;
        }
        return unsafe { AXIsProcessTrusted() != 0 };
    }

    #[cfg(not(target_os = "macos"))]
    {
        true
    }
}

#[cfg(target_os = "macos")]
fn spawn(program: &str, args: &[&str]) -> Result<()> {
    let status = Command::new(program)
        .args(args)
        .status()
        .map_err(|err| Error::Invalid(format!("could not open System Settings: {err}")))?;
    if status.success() {
        Ok(())
    } else {
        Err(Error::Invalid(
            "System Settings could not be opened automatically. Open it from the Apple menu instead.".into(),
        ))
    }
}

#[cfg(windows)]
fn shell_open(url: &str) -> Result<()> {
    use windows::core::HSTRING;
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    let operation = HSTRING::from("open");
    let target = HSTRING::from(url);
    let result = unsafe {
        ShellExecuteW(
            None,
            &operation,
            &target,
            None,
            None,
            SW_SHOWNORMAL,
        )
    };

    // ShellExecuteW reports success as a value greater than 32.
    if result.0 as usize > 32 {
        Ok(())
    } else {
        Err(Error::Invalid(
            "Windows could not open the privacy settings page".into(),
        ))
    }
}
