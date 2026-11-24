import { useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";

import {
  getAppInfo,
  getHotkeyStatus,
  getModelStatus,
  getSettings,
  listModels,
  quitApp,
  selectModel,
  setPaused,
  startModelDownload,
  updateSettings,
  type AppInfo,
  type Settings,
} from "./lib/api";
import {
  EVENT_APP_STATE,
  EVENT_HOTKEY_STATUS,
  EVENT_MODEL_STATUS,
  EVENT_SETTINGS_CHANGED,
  platformLabel,
  type AppStateEvent,
  type HotkeyStatus,
  type ModelOption,
  type ModelStatus,
} from "./lib/events";
import { LANGUAGES } from "./lib/languages";
import { HotkeyRecorder } from "./components/HotkeyRecorder";
import { Mark } from "./components/Mark";
import { MicrophonePicker } from "./components/MicrophonePicker";
import { ModelSection } from "./components/ModelSection";
import { Notice } from "./components/Notice";
import { Section, SettingRow } from "./components/Section";
import { Toggle } from "./components/Toggle";
import { Onboarding } from "./Onboarding";

function describeError(error: unknown): string {
  if (error instanceof Error) return error.message;
  if (typeof error === "string" && error.length > 0) return error;
  return "Something went wrong. Check the log for details.";
}

export default function App() {
  const [settings, setSettings] = useState<Settings | null>(null);
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [model, setModel] = useState<ModelStatus | null>(null);
  const [modelOptions, setModelOptions] = useState<ModelOption[]>([]);
  const [hotkeyStatus, setHotkeyStatus] = useState<HotkeyStatus | null>(null);
  const [paused, setPausedState] = useState(false);
  const [saving, setSaving] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const load = useCallback(async () => {
    setLoadError(null);
    try {
      const [loadedSettings, loadedInfo, loadedModel, loadedOptions] = await Promise.all([
        getSettings(),
        getAppInfo(),
        getModelStatus(),
        listModels(),
      ]);
      setSettings(loadedSettings);
      setInfo(loadedInfo);
      setModel(loadedModel);
      setModelOptions(loadedOptions);
      setPausedState(loadedInfo.paused);
      void getHotkeyStatus()
        .then(setHotkeyStatus)
        .catch(() => undefined);
    } catch (error) {
      setLoadError(describeError(error));
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  useEffect(() => {
    const listeners = Promise.all([
      listen<AppStateEvent>(EVENT_APP_STATE, (event) => setPausedState(event.payload.paused)),
      listen<Settings>(EVENT_SETTINGS_CHANGED, (event) => setSettings(event.payload)),
      listen<HotkeyStatus>(EVENT_HOTKEY_STATUS, (event) => setHotkeyStatus(event.payload)),
      listen<ModelStatus>(EVENT_MODEL_STATUS, (event) => {
        setModel(event.payload);
        // Progress events arrive every couple of megabytes; only the final
        // state changes which models are installed and selected.
        if (event.payload.phase === "ready" || event.payload.phase === "error") {
          void listModels()
            .then(setModelOptions)
            .catch(() => undefined);
        }
      }),
    ]);

    return () => {
      void listeners.then((unlisten) => unlisten.forEach((stop) => stop()));
    };
  }, []);

  useEffect(() => {
    if (!notice) return;
    const timer = window.setTimeout(() => setNotice(null), 7000);
    return () => window.clearTimeout(timer);
  }, [notice]);

  const save = async (patch: Partial<Settings>) => {
    if (!settings || saving) return;
    const previous = settings;
    setSettings({ ...settings, ...patch });
    setSaving(true);
    try {
      const saved = await updateSettings({ ...settings, ...patch });
      setSettings(saved);
      setNotice(null);
    } catch (error) {
      setSettings(previous);
      setNotice(describeError(error));
    } finally {
      setSaving(false);
    }
  };

  const togglePause = async () => {
    const next = !paused;
    setPausedState(next);
    try {
      await setPaused(next);
    } catch (error) {
      setPausedState(!next);
      setNotice(describeError(error));
    }
  };

  const downloadModel = async () => {
    try {
      const status = await startModelDownload();
      setModel(status);
    } catch (error) {
      setNotice(describeError(error));
    }
  };

  const chooseModel = async (modelId: string, customModelPath = "") => {
    try {
      const status = await selectModel(modelId, customModelPath);
      setModel(status);
      setModelOptions(await listModels());
      if (customModelPath) {
        setNotice(null);
      }
    } catch (error) {
      setNotice(describeError(error));
      void listModels()
        .then(setModelOptions)
        .catch(() => undefined);
    }
  };

  if (loadError) {
    return (
      <main className="app-canvas flex h-full items-center justify-center px-10">
        <div className="w-full max-w-[420px] rounded-card border border-line bg-surface p-7 text-center shadow-sm">
          <Mark className="h-11 w-11" />
          <h1 className="mt-5 font-display text-[24px] leading-tight">
            Settings couldn&rsquo;t load.
          </h1>
          <p className="mt-2 text-[12.5px] leading-relaxed text-muted" role="alert">
            {loadError}
          </p>
          <div className="mt-6 flex items-center justify-center gap-3">
            <button
              type="button"
              onClick={() => void load()}
              className="rounded-lg bg-ink px-4 py-2 text-[12.5px] font-medium text-paper transition-colors hover:bg-ink-soft"
            >
              Try again
            </button>
            <button
              type="button"
              onClick={() => void quitApp()}
              className="rounded-lg px-3 py-2 text-[12.5px] text-muted transition-colors hover:text-ink"
            >
              Quit Speakeasy
            </button>
          </div>
        </div>
      </main>
    );
  }

  if (!settings || !info) {
    return (
      <main className="app-canvas flex h-full items-center justify-center">
        <div className="flex items-center gap-3 text-muted">
          <Mark className="h-8 w-8 animate-pulse" />
          <span className="text-[12.5px]">Loading settings…</span>
        </div>
      </main>
    );
  }

  const isMac = info.platform === "macos";
  const platform = platformLabel(info.platform);

  if (!settings.onboardingComplete) {
    return (
      <>
        <Onboarding
          settings={settings}
          info={info}
          model={model}
          modelOptions={modelOptions}
          hotkeyStatus={hotkeyStatus}
          saving={saving}
          onSave={(patch) => void save(patch)}
          onSelectModel={(modelId, path) => void chooseModel(modelId, path)}
          onDownloadModel={() => void downloadModel()}
          onFinish={() => void save({ onboardingComplete: true })}
          onNotice={setNotice}
        />
        {notice && <Notice message={notice} onDismiss={() => setNotice(null)} />}
      </>
    );
  }

  return (
    <main className="app-canvas min-h-full">
      <div className="mx-auto w-full max-w-[720px] px-8 pb-9 pt-7">
        <header className="flex items-center justify-between gap-4">
          <div className="flex items-center gap-3">
            <Mark />
            <div>
              <p className="font-mono text-[9.5px] uppercase tracking-[0.24em] text-muted">
                Speakeasy
              </p>
              <p className="mt-0.5 font-display text-[16px] leading-none text-ink">Settings</p>
            </div>
          </div>

          <div className="flex items-center gap-2.5">
            <span
              className={`inline-flex items-center gap-1.5 rounded-full border px-2.5 py-1 text-[10.5px] font-medium tracking-wide ${
                paused
                  ? "border-amber/25 bg-amber/10 text-amber"
                  : "border-moss/25 bg-moss/10 text-moss"
              }`}
            >
              <span className="h-1.5 w-1.5 rounded-full bg-current" aria-hidden="true" />
              {paused ? "Paused" : "Ready"}
            </span>
            <button
              type="button"
              onClick={() => void togglePause()}
              className="rounded-full border border-line-strong bg-surface px-3 py-1.5 text-[11px] font-medium text-ink-soft transition-colors hover:border-faint hover:text-ink"
            >
              {paused ? "Resume" : "Pause"}
            </button>
          </div>
        </header>

        <div className="mt-9 animate-rise">
          <h1 className="font-display text-[39px] leading-[1.02] tracking-[-0.015em] text-ink">
            Speak, don&rsquo;t type.
          </h1>
          <p className="mt-3 max-w-[48ch] text-[13.5px] leading-relaxed text-muted">
            Hold your hotkey, say what you mean, release. The words land wherever your cursor is —
            transcribed on this {isMac ? "Mac" : "PC"} and never uploaded.
          </p>
        </div>

        <div className="mt-8 animate-rise-late space-y-4">
          <Section
            eyebrow="Activation"
            title="Hold to talk"
            aside={
              <span className="mt-0.5 font-mono text-[9.5px] uppercase tracking-[0.18em] text-faint">
                {platform}
              </span>
            }
          >
            <HotkeyRecorder
              value={settings.hotkey}
              platform={info.platform}
              disabled={saving}
              onCommit={(hotkey) => void save({ hotkey })}
            />
            <p className="mt-4 max-w-[42ch] text-[12.5px] leading-relaxed text-muted">
              Hold to record, release to transcribe and paste into the app in front of you.
              Recordings shorter than 300&nbsp;ms are ignored.
            </p>
            {hotkeyStatus && !hotkeyStatus.registered && hotkeyStatus.error && (
              <p
                className="mt-3 max-w-[52ch] rounded-lg bg-ember-soft px-3 py-2 text-[11.5px] leading-relaxed text-ember"
                role="alert"
              >
                {hotkeyStatus.error}
              </p>
            )}
            {isMac && settings.hotkey.trim().toLowerCase() === "fn" && (
              <p className="mt-3 text-[11.5px] leading-relaxed text-amber">
                The fn key needs Accessibility and Input Monitoring access. If it stopped working,
                re-grant both in System Settings → Privacy &amp; Security, then restart Speakeasy.
              </p>
            )}
          </Section>

          <ModelSection
            model={model}
            options={modelOptions}
            customPath={settings.customModelPath}
            busy={saving}
            onSelect={(modelId, path) => void chooseModel(modelId, path)}
            onDownload={() => void downloadModel()}
          />

          <Section eyebrow="Input" title="Microphone & language">
            <div className="border-b border-line/80 pb-4">
              <p className="text-[13.5px] font-medium leading-tight text-ink">Microphone</p>
              <p className="mt-1 max-w-[48ch] text-[12px] leading-relaxed text-muted">
                The input Speakeasy records from. Test it once after changing devices.
              </p>
              <div className="mt-3">
                <MicrophonePicker
                  value={settings.microphone}
                  disabled={saving}
                  onChange={(device) => void save({ microphone: device })}
                />
              </div>
            </div>

            <SettingRow
              title="Language"
              description="Whisper listens in one language at a time. Auto-detect works well for most speech."
              last
            >
              <div className="relative">
                <select
                  aria-label="Transcription language"
                  value={settings.language}
                  disabled={saving}
                  onChange={(event) => void save({ language: event.target.value })}
                  className="h-9 w-[186px] appearance-none rounded-lg border border-line-strong bg-surface pl-3 pr-8 text-[12.5px] text-ink transition-colors hover:border-faint focus:border-ink focus:outline-none disabled:opacity-60"
                >
                  <option value="auto">Auto-detect</option>
                  {LANGUAGES.map((language) => (
                    <option key={language.code} value={language.code}>
                      {language.name}
                    </option>
                  ))}
                </select>
                <svg
                  viewBox="0 0 12 12"
                  aria-hidden="true"
                  className="pointer-events-none absolute right-2.5 top-1/2 h-3 w-3 -translate-y-1/2 text-faint"
                >
                  <path
                    d="M2.5 4.5 6 8l3.5-3.5"
                    fill="none"
                    stroke="currentColor"
                    strokeWidth="1.4"
                    strokeLinecap="round"
                    strokeLinejoin="round"
                  />
                </svg>
              </div>
            </SettingRow>
          </Section>

          <Section eyebrow="Preferences">
            <SettingRow
              title="Mute system audio"
              description="Silence music and calls while you dictate. Your previous mute state is restored afterwards, even if Speakeasy quits unexpectedly."
            >
              <Toggle
                label="Mute system audio while recording"
                checked={settings.muteSystemAudio}
                disabled={saving}
                onChange={(value) => void save({ muteSystemAudio: value })}
              />
            </SettingRow>

            <SettingRow
              title="Launch at login"
              description={`Keep Speakeasy waiting in the ${isMac ? "menu bar" : "notification area"}.`}
              last
            >
              <Toggle
                label="Launch Speakeasy at login"
                checked={settings.launchAtLogin}
                disabled={saving}
                onChange={(value) => void save({ launchAtLogin: value })}
              />
            </SettingRow>
          </Section>

          <section className="relative overflow-hidden rounded-card bg-ink px-6 py-6 text-paper">
            <div className="flex items-center gap-2 font-mono text-[9.5px] uppercase tracking-[0.2em] text-paper/50">
              <svg viewBox="0 0 16 16" aria-hidden="true" className="h-3 w-3">
                <rect
                  x="3"
                  y="7"
                  width="10"
                  height="7"
                  rx="1.6"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="1.2"
                />
                <path
                  d="M5.5 7V5a2.5 2.5 0 0 1 5 0v2"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="1.2"
                  strokeLinecap="round"
                />
              </svg>
              Privacy
            </div>
            <p className="mt-3 font-display text-[23px] leading-tight text-paper">
              Nothing leaves this machine.
            </p>
            <div className="mt-4 grid gap-2 text-[12px] leading-relaxed text-paper/70 sm:grid-cols-3">
              <p>Audio stays on-device.</p>
              <p>No account, no API key.</p>
              <p>No analytics, no telemetry.</p>
            </div>
            <p className="mt-4 text-[11px] leading-relaxed text-paper/45">
              The only network request Speakeasy ever makes is the one-time Whisper model download.
            </p>
          </section>

          <details className="group rounded-xl border border-line/80 bg-surface/50">
            <summary className="flex cursor-pointer list-none items-center justify-between px-5 py-3.5 text-[11.5px] text-muted transition-colors hover:text-ink [&::-webkit-details-marker]:hidden">
              Diagnostics
              <svg
                viewBox="0 0 12 12"
                aria-hidden="true"
                className="h-3 w-3 transition-transform duration-200 group-open:rotate-180"
              >
                <path
                  d="M2.5 4.5 6 8l3.5-3.5"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="1.4"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                />
              </svg>
            </summary>
            <dl className="grid gap-2.5 px-5 pb-5 font-mono text-[10.5px] leading-relaxed">
              <div className="grid grid-cols-[92px_1fr] gap-3">
                <dt className="text-faint">Version</dt>
                <dd className="text-ink-soft">
                  {info.version} · {platform} {info.arch}
                </dd>
              </div>
              <div className="grid grid-cols-[92px_1fr] gap-3">
                <dt className="text-faint">Settings</dt>
                <dd className="select-text break-all text-ink-soft">{info.settingsPath}</dd>
              </div>
              <div className="grid grid-cols-[92px_1fr] gap-3">
                <dt className="text-faint">Data</dt>
                <dd className="select-text break-all text-ink-soft">{info.dataDir}</dd>
              </div>
              <div className="grid grid-cols-[92px_1fr] gap-3">
                <dt className="text-faint">Logs</dt>
                <dd className="select-text break-all text-ink-soft">{info.logDir}</dd>
              </div>
            </dl>
          </details>
        </div>

        <footer className="mt-7 flex items-center justify-between border-t border-line/80 pt-4">
          <p className="text-[11px] text-faint">
            Settings save automatically. No restart needed.
          </p>
          <button
            type="button"
            onClick={() => void quitApp()}
            className="rounded-lg px-2.5 py-1.5 text-[11.5px] text-muted transition-colors hover:bg-ember-soft hover:text-ember"
          >
            Quit Speakeasy
          </button>
        </footer>
      </div>

      {notice && <Notice message={notice} onDismiss={() => setNotice(null)} />}
    </main>
  );
}
