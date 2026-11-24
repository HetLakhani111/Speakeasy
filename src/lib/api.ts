import { invoke } from "@tauri-apps/api/core";

import type { HotkeyStatus, ModelOption, ModelStatus } from "./events";

export interface Settings {
  hotkey: string;
  microphone: string | null;
  language: string;
  /** Catalog id of the selected Whisper model, or `custom`. */
  modelId: string;
  /** Path used when `modelId` is `custom`. */
  customModelPath: string;
  muteSystemAudio: boolean;
  launchAtLogin: boolean;
  onboardingComplete: boolean;
}

export interface AppInfo {
  version: string;
  platform: string;
  arch: string;
  settingsPath: string;
  logDir: string;
  dataDir: string;
  paused: boolean;
}

export interface InputDeviceInfo {
  name: string;
  isDefault: boolean;
}

export const getSettings = () => invoke<Settings>("get_settings");

export const updateSettings = (settings: Settings) =>
  invoke<Settings>("update_settings", { settings });

export const getAppInfo = () => invoke<AppInfo>("get_app_info");

export const setPaused = (paused: boolean) => invoke<void>("set_paused", { paused });

export const quitApp = () => invoke<void>("quit_app");

export const listInputDevices = () => invoke<InputDeviceInfo[]>("list_input_devices");

export interface MicProbe {
  device: string;
  /** Highest absolute sample value; 0.0 is silence, 1.0 is clipping. */
  peak: number;
  rms: number;
  durationMs: number;
}

/** Records a short sample for onboarding's microphone check. */
export const testMicrophone = (device: string | null) =>
  invoke<MicProbe>("test_microphone", { device });

export type PermissionPane = "microphone" | "accessibility" | "input-monitoring";

export const openPermissionSettings = (pane: PermissionPane) =>
  invoke<void>("open_permission_settings", { pane });

export const accessibilityStatus = () =>
  invoke<{ accessibility: boolean }>("accessibility_status");

export const setHotkeyCapture = (active: boolean) =>
  invoke<void>("set_hotkey_capture", { active });

export const hideSettings = () => invoke<void>("hide_settings");

export const restartApp = () => invoke<void>("restart_app");

export const getHotkeyStatus = () => invoke<HotkeyStatus | null>("get_hotkey_status");

/** Starts a recording without the hotkey. Used by the dictation test. */
export const startTestRecording = () => invoke<void>("start_test_recording");

export const stopTestRecording = () => invoke<void>("stop_test_recording");

/**
 * Marks onboarding's test step. Hotkey dictation shows its transcript in the
 * window instead of pasting it into whatever app has focus.
 */
export const setTestMode = (active: boolean) =>
  invoke<void>("set_test_mode", { active });

export const getModelStatus = () => invoke<ModelStatus>("get_model_status");

/** Every model the user can choose from, including a custom file. */
export const listModels = () => invoke<ModelOption[]>("list_models");

/**
 * Switches the active model. `customPath` is only meaningful with the `custom`
 * id; other ids ignore it.
 */
export const selectModel = (modelId: string, customPath = "") =>
  invoke<ModelStatus>("select_model", { modelId, customPath });

/** Starts or resumes the one-time model download. Returns immediately. */
export const startModelDownload = () => invoke<ModelStatus>("start_model_download");
