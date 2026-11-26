# Speakeasy

Hold a key, say what you mean, release. Speakeasy transcribes your speech on your
own machine and pastes the text into whatever app has focus.

No account. No API key. No telemetry. The only network request the app ever
makes is the one-time download of the speech model.

- **Fully local** — Whisper runs on your CPU or GPU through `whisper-rs`.
- **Push-to-talk** — recording starts on key down and stops on key up; nothing
  is ever captured in the background.
- **Straight into your app** — the transcript is pasted at the cursor, then your
  clipboard is put back the way it was.
- **Small and quiet** — lives in the menu bar (macOS) or notification area
  (Windows), with a translucent level meter while you speak.

## Download

Grab the latest build from [Releases](https://github.com/HetLakhani111/Speakeasy/releases/latest).

| Platform | File | Notes |
| --- | --- | --- |
| macOS 11 or newer, Apple Silicon | `Speakeasy_<version>_aarch64.dmg` | Metal acceleration. Intel Macs are not supported yet. |
| Windows 10/11, x64 | `Speakeasy_<version>_x64-setup.exe` | Recommended. Installs for the current user; no admin prompt. |
| Windows 10/11, x64 | `Speakeasy_<version>_x64_en-US.msi` | For managed or scripted installs. |

The builds are unsigned. On macOS, open the app once with
**System Settings → Privacy & Security → Open Anyway** (or run
`xattr -dr com.apple.quarantine /Applications/Speakeasy.app`). On Windows,
SmartScreen may ask you to confirm the publisher.

### System requirements

- macOS 11+ on Apple Silicon, or Windows 10/11 on x64.
- A working microphone.
- About 1 GB of disk space: the app plus the recommended model.
- A GPU is optional. Speakeasy uses Metal on macOS and Vulkan on Windows when
  available, and falls back to the CPU when it is not.

## First run

The first launch opens a short setup:

1. **Activation** — pick the key you want to hold. The default is `fn` on macOS
   and `Ctrl+Space` on Windows. Any modifier on its own works (`fn`, `Ctrl`,
   `Alt`, `Shift`, `Cmd`), as does a modifier plus another key.
2. **Model** — download the speech model. The default (`Large v3 Turbo · Q5`,
   ~547 MB) is the best balance for most machines. The download resumes if it is
   interrupted and is checked against its published SHA-256.
3. **Microphone** — choose the input device and run the level check.
4. **Test** — hold the hotkey (or the button) and read the sentence on screen.
   The transcript appears in the window; nothing is pasted during the test.

You can skip setup and finish later from the tray menu. After setup, the settings
window is available from the tray icon.

### Default hotkeys

| Platform | Default | Notes |
| --- | --- | --- |
| macOS | `fn` | Needs Accessibility and Input Monitoring. |
| Windows | `Ctrl+Space` | If your IME or another app owns `Ctrl+Space`, Speakeasy reports the conflict and asks you to pick another key. |

Hold the key for as long as you are speaking, then release. Clips shorter than
300 ms are ignored, silence at both ends is trimmed, and a single recording is
capped at five minutes.

## Settings

Everything is stored as plain JSON and applied immediately; no restart needed.

| Setting | What it does |
| --- | --- |
| Activation | Hold-to-talk key, including modifier-only keys. |
| Model | Pick another catalog model or a `.bin` GGML model from your disk. |
| Microphone | Input device, with a level check. |
| Language | Auto-detect or force one Whisper language. |
| Mute system audio | Silences the default output while you record, so music and calls do not bleed into the microphone. |
| Launch at login | Starts Speakeasy in the tray when you sign in. |

Muting is opt-in. Speakeasy records the previous state, restores it when the clip
ends, and starts a tiny watchdog process so the output is restored even if the
app is force-quit or crashes.

## Privacy

- Audio is processed in memory on your machine. It is never uploaded.
- The model download from `huggingface.co` is the only outbound connection.
  There is no telemetry, analytics, crash reporting, or update check.
- Logs stay on disk and contain timings and device names, not transcripts.
  Audio is not written to disk unless you are running a debug build or set
  `SPEAKEASY_KEEP_WAV=1`.
- Pasting preserves a text clipboard. If the clipboard holds something that is
  not text, it is left untouched and the transcript is typed instead.

### Where data lives

| | macOS | Windows |
| --- | --- | --- |
| Settings | `~/Library/Application Support/com.speakeasy.desktop/settings.json` | `%APPDATA%\com.speakeasy.desktop\settings.json` |
| Models | `~/Library/Application Support/com.speakeasy.desktop/models/` | `%APPDATA%\com.speakeasy.desktop\models\` |
| Logs | `~/Library/Logs/com.speakeasy.desktop/` | `%LOCALAPPDATA%\com.speakeasy.desktop\logs\` |

The exact paths are also shown at the bottom of the settings window.

## Troubleshooting

**The hotkey does nothing.** Something else may own it (especially `Ctrl+Space`
with a Windows IME). Pick another binding in Settings → Activation; registration
errors are shown inline. On macOS, `fn` needs both Accessibility and Input
Monitoring, and permission changes only apply after a restart.

**Nothing is pasted.** Grant Accessibility on macOS. On Windows, make sure the
target app accepts synthetic keystrokes (some elevated windows reject input from
a non-elevated app).

**The recording is silent.** Run the microphone check in Settings → Input. On
Windows, check Settings → Privacy & security → Microphone.

**The system stayed muted.** It should not: state is restored on stop, on quit,
and by a watchdog if the process dies. If a previous run was killed before the
watchdog started, unmute from the system volume control.

**The model download fails.** Start it again; it resumes from the partial file.
If the SHA-256 check fails, the partial file is discarded and downloaded fresh.

## Build from source

Prerequisites:

- Node.js 20 or newer (CI uses 22).
- Rust 1.88 or newer with the platform toolchain.
- **macOS:** Xcode Command Line Tools and CMake. Metal is enabled by default.
- **Windows:** Visual Studio Build Tools with the C++ workload, CMake, LLVM
  (for `libclang`), and the [Vulkan SDK](https://vulkan.lunarg.com/sdk/home).
  Whisper's C/C++ code is compiled from source on first build.

On Windows, keep the build directory short (for example
`set CARGO_TARGET_DIR=E:\st`). Deep paths inside the Vulkan shader build can
exceed the 260-character limit.

```sh
npm ci
npm run tauri dev      # development window with hot reload
npm run tauri build    # installers in src-tauri/target/release/bundle/
```

If CMake cannot find `libclang`, point it at LLVM:

```powershell
$env:LIBCLANG_PATH = "C:\Program Files\LLVM\bin"
```

### Tests

```sh
cd src-tauri
cargo test --lib
```

The suite covers resampling, silence trimming, transcript cleanup, settings
normalization, download resume logic, hotkey parsing, and the audio mute guard.
It does not need a model file or a microphone.

### Diagnostics

| Variable | Effect |
| --- | --- |
| `SPEAKEASY_FORCE_CPU=1` | Ignore the GPU and run Whisper on the CPU. |
| `SPEAKEASY_KEEP_WAV=1` | Keep the last raw and processed capture as WAV files. |
| `SPEAKEASY_DEV_OVERLAY=1` | Show the overlay with a synthetic level pattern. |
| `SPEAKEASY_DEV_OPEN_SETTINGS=1` | Open the settings window at launch. |
| `RUST_LOG=debug` | Verbose logs (written to the log directory). |

### How it fits together

```
hotkey (global-hotkey / native modifier hook)
  └─ cpal capture → mono f32 → bounded channel
       └─ resample to 16 kHz → trim silence → WAV frames
            └─ whisper-rs (resident context) → cleanup → enigo paste
```

The UI is React + Tailwind inside a Tauri v2 webview. The overlay is a second,
transparent, click-through window that renders the level meter at ~60 fps.

Measured on a laptop with an RTX 4060 (Vulkan), 6.7 s clip, `Large v3 Turbo · Q5`:
about 0.8 s of inference, roughly 1.3 s end to end once the model is warm. On the
CPU the same clip takes about 25 s; use `Base · Q5` or `Tiny · Q5` there, or a
smaller model on slower hardware.

## License

GNU General Public License v3.0 or later. Copyright © 2026 Speakeasy contributors.

Speakeasy is free software: you can redistribute it and/or modify it under the
terms of the GNU General Public License as published by the Free Software
Foundation, either version 3 of the License, or (at your option) any later
version.

Speakeasy is distributed in the hope that it will be useful, but **without any
warranty**; without even the implied warranty of merchantability or fitness for
a particular purpose. See [`LICENSE`](LICENSE) for the full text.
