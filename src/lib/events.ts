/**
 * Event payloads emitted by the Rust side. Keep this file in sync with the
 * `#[derive(Serialize)]` structs in src-tauri/src.
 */

export const EVENT_APP_STATE = "app-state";
export const EVENT_SETTINGS_CHANGED = "settings-changed";
export const EVENT_AUDIO_LEVELS = "audio-levels";
export const EVENT_RECORDING_STARTED = "recording-started";
export const EVENT_RECORDING_STOPPED = "recording-stopped";
export const EVENT_NOTIFICATION = "notification";
export const EVENT_HOTKEY_STATUS = "hotkey-status";
export const EVENT_TRANSCRIPT = "transcript";
export const EVENT_MODEL_STATUS = "model-status";

export interface AppStateEvent {
  paused: boolean;
}

export interface AudioLevels {
  /** Overall loudness, 0.0 (silence) to 1.0 (clipping). */
  level: number;
  /** Smoothed, dB-normalized magnitudes for each log-spaced band. */
  bins: number[];
}

export interface RecordingStarted {
  device: string | null;
}

export interface RecordingStopped {
  /** Audio length sent to Whisper after resampling and trimming. */
  durationMs: number;
  /** Audio length captured from the microphone before processing. */
  capturedMs: number;
  peak: number;
  /** Debug-only path to the saved WAV, if one was written. */
  wavPath: string | null;
}

export type NotificationLevel = "info" | "warning" | "error";

export interface Notification {
  level: NotificationLevel;
  message: string;
}

export interface HotkeyStatus {
  binding: string;
  registered: boolean;
  error: string | null;
  /** True when the binding is served by the macOS fn-key event tap. */
  native: boolean;
}

export interface TranscriptEvent {
  text: string;
  /** Length of the audio Whisper received. */
  audioMs: number;
  /** Time spent loading the model, if this clip triggered the first load. */
  loadMs: number;
  inferenceMs: number;
  /** Hotkey release to paste. The 1.5 s budget refers to this number. */
  totalMs: number;
  /** Debug-only path to the WAV that was transcribed, if one was written. */
  wavPath: string | null;
}

export type ModelPhase = "missing" | "downloading" | "verifying" | "ready" | "error";

export interface ModelStatus {
  phase: ModelPhase;
  /** Catalog id, or `custom` for a file the user supplied. */
  id: string;
  label: string;
  path: string;
  /** Bytes on disk, including a resumable partial download. */
  receivedBytes: number;
  totalBytes: number;
  custom: boolean;
  error: string | null;
}

/** One row of the model picker. */
export interface ModelOption {
  id: string;
  label: string;
  note: string;
  bytes: number;
  installed: boolean;
  selected: boolean;
  custom: boolean;
}

export const platformLabel = (platform: string): string => {
  switch (platform) {
    case "macos":
      return "macOS";
    case "windows":
      return "Windows";
    case "linux":
      return "Linux";
    default:
      return platform;
  }
};
