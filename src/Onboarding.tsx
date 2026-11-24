import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";

import {
  accessibilityStatus,
  hideSettings,
  openPermissionSettings,
  restartApp,
  setTestMode,
  startTestRecording,
  stopTestRecording,
  type AppInfo,
  type Settings,
} from "./lib/api";
import {
  EVENT_TRANSCRIPT,
  type HotkeyStatus,
  type ModelOption,
  type ModelStatus,
  type TranscriptEvent,
} from "./lib/events";
import { HotkeyRecorder } from "./components/HotkeyRecorder";
import { Keycaps } from "./components/Keycaps";
import { Mark } from "./components/Mark";
import { MicrophonePicker } from "./components/MicrophonePicker";
import { ModelSection } from "./components/ModelSection";
import { Toggle } from "./components/Toggle";

interface OnboardingProps {
  settings: Settings;
  info: AppInfo;
  model: ModelStatus | null;
  modelOptions: ModelOption[];
  hotkeyStatus: HotkeyStatus | null;
  saving: boolean;
  onSave: (patch: Partial<Settings>) => void;
  onSelectModel: (modelId: string, customPath?: string) => void;
  onDownloadModel: () => void;
  onFinish: () => void;
  onNotice: (message: string) => void;
}

type TestState = "idle" | "recording" | "processing" | "done";

const MAC_STEPS = ["Welcome", "Model", "Microphone", "Permissions", "Test"];
const PC_STEPS = ["Welcome", "Model", "Microphone", "Test"];

export function Onboarding({
  settings,
  info,
  model,
  modelOptions,
  hotkeyStatus,
  saving,
  onSave,
  onSelectModel,
  onDownloadModel,
  onFinish,
  onNotice,
}: OnboardingProps) {
  const isMac = info.platform === "macos";
  const steps = isMac ? MAC_STEPS : PC_STEPS;
  const [index, setIndex] = useState(0);
  const [transcript, setTranscript] = useState<TranscriptEvent | null>(null);
  const [testState, setTestState] = useState<TestState>("idle");
  const [testError, setTestError] = useState<string | null>(null);
  const [trusted, setTrusted] = useState<boolean | null>(null);
  const [checkingAccess, setCheckingAccess] = useState(false);
  const recordingRef = useRef(false);

  const step = steps[index];
  const modelReady = model?.phase === "ready";

  useEffect(() => {
    const unlisten = listen<TranscriptEvent>(EVENT_TRANSCRIPT, (event) => {
      setTranscript(event.payload);
      setTestError(null);
      setTestState("done");
    });
    return () => {
      void unlisten.then((stop) => stop());
    };
  }, []);

  useEffect(() => {
    return () => {
      if (recordingRef.current) {
        recordingRef.current = false;
        void stopTestRecording().catch(() => undefined);
      }
    };
  }, []);

  useEffect(() => {
    if (testState !== "processing") return;
    const timer = window.setTimeout(() => {
      setTestState("idle");
      setTestError("No transcript came back. Check the microphone and try again.");
    }, 15000);
    return () => window.clearTimeout(timer);
  }, [testState]);

  const checkAccessibility = useCallback(async () => {
    setCheckingAccess(true);
    try {
      const status = await accessibilityStatus();
      setTrusted(status.accessibility);
    } catch {
      setTrusted(null);
    } finally {
      setCheckingAccess(false);
    }
  }, []);

  useEffect(() => {
    if (step === "Permissions") void checkAccessibility();
  }, [checkAccessibility, step]);

  // On the test step the real hotkey should work, but its transcript belongs in
  // this window: never paste into an app while setup is still running.
  useEffect(() => {
    if (step !== "Test") return;
    void setTestMode(true).catch(() => undefined);
    return () => {
      void setTestMode(false).catch(() => undefined);
    };
  }, [step]);

  const beginTest = useCallback(async () => {
    if (!modelReady || recordingRef.current) return;
    setTestError(null);
    setTranscript(null);
    try {
      await startTestRecording();
      recordingRef.current = true;
      setTestState("recording");
    } catch (error) {
      recordingRef.current = false;
      setTestState("idle");
      setTestError(error instanceof Error ? error.message : String(error));
    }
  }, [modelReady]);

  const finishTest = useCallback(async () => {
    if (!recordingRef.current) return;
    recordingRef.current = false;
    setTestState("processing");
    try {
      await stopTestRecording();
    } catch (error) {
      setTestState("idle");
      setTestError(error instanceof Error ? error.message : String(error));
    }
  }, []);

  const openPane = (pane: "microphone" | "accessibility" | "input-monitoring") => {
    void openPermissionSettings(pane).catch((error) => {
      onNotice(error instanceof Error ? error.message : String(error));
    });
  };

  const primaryLabel = useMemo(() => {
    if (step === "Test") return "Finish setup";
    return "Continue";
  }, [step]);

  const canContinue = step !== "Model" || modelReady;

  const next = () => {
    if (!canContinue) return;
    if (step === "Test") {
      onFinish();
      return;
    }
    setIndex((current) => Math.min(current + 1, steps.length - 1));
  };

  return (
    <main className="app-canvas min-h-full">
      <div className="mx-auto flex min-h-full w-full max-w-[720px] flex-col px-8 pb-9 pt-7">
        <header className="flex items-center justify-between gap-4">
          <div className="flex items-center gap-3">
            <Mark />
            <div>
              <p className="font-mono text-[9.5px] uppercase tracking-[0.24em] text-muted">
                Speakeasy
              </p>
              <p className="mt-0.5 font-display text-[16px] leading-none text-ink">
                First-run setup
              </p>
            </div>
          </div>
          <button
            type="button"
            onClick={() => void hideSettings()}
            className="rounded-lg px-2.5 py-1.5 text-[11.5px] text-muted transition-colors hover:bg-paper-deep hover:text-ink"
          >
            Set up later
          </button>
        </header>

        <ol className="mt-9 flex items-center gap-2" aria-label="Setup steps">
          {steps.map((label, position) => (
            <li key={label} className="flex flex-1 items-center gap-2">
              <span
                aria-current={position === index ? "step" : undefined}
                className={`h-1 flex-1 rounded-full transition-colors ${
                  position <= index ? "bg-ink" : "bg-line-strong/60"
                }`}
              />
              <span
                className={`hidden font-mono text-[9px] uppercase tracking-[0.16em] sm:block ${
                  position === index ? "text-ink" : "text-faint"
                }`}
              >
                {label}
              </span>
            </li>
          ))}
        </ol>

        <div className="mt-8 flex-1 animate-rise">
          {step === "Welcome" && (
            <div>
              <h1 className="font-display text-[39px] leading-[1.02] tracking-[-0.015em] text-ink">
                Speak, don&rsquo;t type.
              </h1>
              <p className="mt-3 max-w-[52ch] text-[13.5px] leading-relaxed text-muted">
                Hold a key, say what you mean, release. The words land wherever your cursor is.
                Everything is transcribed on this {isMac ? "Mac" : "PC"} — audio never leaves it.
              </p>

              <div className="mt-7 rounded-card border border-line bg-surface/85 p-6">
                <p className="font-mono text-[10px] uppercase tracking-[0.2em] text-muted">
                  Activation
                </p>
                <p className="mt-1.5 font-display text-[19px] leading-none text-ink">
                  Choose your hold-to-talk key
                </p>
                <div className="mt-4">
                  <HotkeyRecorder
                    value={settings.hotkey}
                    platform={info.platform}
                    disabled={saving}
                    onCommit={(hotkey) => onSave({ hotkey })}
                  />
                </div>
                {hotkeyStatus && !hotkeyStatus.registered && hotkeyStatus.error && (
                  <p className="mt-3 rounded-lg bg-ember-soft px-3 py-2 text-[11.5px] leading-relaxed text-ember" role="alert">
                    {hotkeyStatus.error}
                  </p>
                )}
                {isMac && settings.hotkey.trim().toLowerCase() === "fn" && (
                  <p className="mt-3 text-[11.5px] leading-relaxed text-amber">
                    The fn key asks for Accessibility and Input Monitoring access. You can grant
                    both in a couple of steps.
                  </p>
                )}
              </div>

              <div className="mt-4 flex items-start justify-between gap-8 rounded-card border border-line bg-surface/60 px-6 py-4">
                <div>
                  <p className="text-[13.5px] font-medium leading-tight text-ink">
                    Launch at login
                  </p>
                  <p className="mt-1 max-w-[46ch] text-[12px] leading-relaxed text-muted">
                    Keep Speakeasy waiting in the {isMac ? "menu bar" : "notification area"} after a
                    restart.
                  </p>
                </div>
                <Toggle
                  label="Launch Speakeasy at login"
                  checked={settings.launchAtLogin}
                  disabled={saving}
                  onChange={(value) => onSave({ launchAtLogin: value })}
                />
              </div>

              <p className="mt-5 max-w-[52ch] text-[11.5px] leading-relaxed text-faint">
                No account. No API key. No analytics. The only download is the speech model in the
                next step.
              </p>
            </div>
          )}

          {step === "Model" && (
            <div>
              <h1 className="font-display text-[31px] leading-[1.05] tracking-[-0.01em] text-ink">
                Get the speech model.
              </h1>
              <p className="mt-2.5 max-w-[52ch] text-[13px] leading-relaxed text-muted">
                Whisper runs locally. Pick a size — bigger models are more accurate and slower.
                The download is the only network request Speakeasy makes.
              </p>
              <div className="mt-6">
                <ModelSection
                  model={model}
                  options={modelOptions}
                  customPath={settings.customModelPath}
                  busy={saving}
                  onSelect={(modelId, path) => onSelectModel(modelId, path)}
                  onDownload={onDownloadModel}
                />
              </div>
              {modelReady && (
                <p className="mt-3 inline-flex items-center gap-2 text-[11.5px] text-moss" role="status">
                  <span className="h-1.5 w-1.5 rounded-full bg-current" aria-hidden="true" />
                  Model ready. Continue when you are.
                </p>
              )}
            </div>
          )}

          {step === "Microphone" && (
            <div>
              <h1 className="font-display text-[31px] leading-[1.05] tracking-[-0.01em] text-ink">
                Check your microphone.
              </h1>
              <p className="mt-2.5 max-w-[52ch] text-[13px] leading-relaxed text-muted">
                Pick the input you want to dictate with, then say something. The meter only
                listens while the check runs.
              </p>
              <div className="mt-6 rounded-card border border-line bg-surface/85 p-6">
                <MicrophonePicker
                  value={settings.microphone}
                  disabled={saving}
                  onChange={(device) => onSave({ microphone: device })}
                />
              </div>
              <p className="mt-4 max-w-[52ch] text-[11.5px] leading-relaxed text-faint">
                If you hear silence, check the system microphone permission. On Windows this is
                Settings → Privacy &amp; security → Microphone.
              </p>
              {isMac && (
                <button
                  type="button"
                  onClick={() => openPane("microphone")}
                  className="mt-2 text-[11.5px] font-medium text-ember underline-offset-4 hover:underline"
                >
                  Open microphone permission settings
                </button>
              )}
            </div>
          )}

          {step === "Permissions" && (
            <div>
              <h1 className="font-display text-[31px] leading-[1.05] tracking-[-0.01em] text-ink">
                Two macOS permissions.
              </h1>
              <p className="mt-2.5 max-w-[52ch] text-[13px] leading-relaxed text-muted">
                Speakeasy needs to see the hotkey and paste into the app in front of you. It never
                records in the background.
              </p>

              <div className="mt-6 space-y-3">
                <div className="flex items-start justify-between gap-6 rounded-card border border-line bg-surface/85 px-6 py-5">
                  <div>
                    <p className="text-[13.5px] font-medium leading-tight text-ink">Accessibility</p>
                    <p className="mt-1 max-w-[46ch] text-[12px] leading-relaxed text-muted">
                      Lets Speakeasy send the paste keystroke to the focused app.
                    </p>
                    {trusted === true && (
                      <p className="mt-2 text-[11.5px] font-medium text-moss" role="status">
                        Granted
                      </p>
                    )}
                    {trusted === false && (
                      <p className="mt-2 text-[11.5px] text-amber" role="status">
                        Not granted yet
                      </p>
                    )}
                  </div>
                  <div className="flex shrink-0 flex-col items-end gap-2">
                    <button
                      type="button"
                      onClick={() => openPane("accessibility")}
                      className="rounded-lg bg-ink px-3.5 py-2 text-[12px] font-medium text-paper transition-colors hover:bg-ink-soft"
                    >
                      Open settings
                    </button>
                    <button
                      type="button"
                      onClick={() => void checkAccessibility()}
                      disabled={checkingAccess}
                      className="text-[11px] text-muted transition-colors hover:text-ink disabled:opacity-50"
                    >
                      {checkingAccess ? "Checking…" : "Check again"}
                    </button>
                  </div>
                </div>

                <div className="flex items-start justify-between gap-6 rounded-card border border-line bg-surface/85 px-6 py-5">
                  <div>
                    <p className="text-[13.5px] font-medium leading-tight text-ink">
                      Input Monitoring
                    </p>
                    <p className="mt-1 max-w-[46ch] text-[12px] leading-relaxed text-muted">
                      Only needed for the fn key and other modifier-only bindings.
                    </p>
                  </div>
                  <button
                    type="button"
                    onClick={() => openPane("input-monitoring")}
                    className="shrink-0 rounded-lg border border-line-strong bg-surface px-3.5 py-2 text-[12px] font-medium text-ink transition-colors hover:border-faint"
                  >
                    Open settings
                  </button>
                </div>

                <div className="flex items-start justify-between gap-6 rounded-card border border-line bg-surface/85 px-6 py-5">
                  <div>
                    <p className="text-[13.5px] font-medium leading-tight text-ink">Microphone</p>
                    <p className="mt-1 max-w-[46ch] text-[12px] leading-relaxed text-muted">
                      macOS asks the first time a recording starts.
                    </p>
                  </div>
                  <button
                    type="button"
                    onClick={() => openPane("microphone")}
                    className="shrink-0 rounded-lg border border-line-strong bg-surface px-3.5 py-2 text-[12px] font-medium text-ink transition-colors hover:border-faint"
                  >
                    Open settings
                  </button>
                </div>
              </div>

              <p className="mt-4 max-w-[52ch] text-[11.5px] leading-relaxed text-faint">
                macOS applies permission changes to newly launched apps.{" "}
                <button
                  type="button"
                  onClick={() => void restartApp()}
                  className="font-medium text-ink underline-offset-4 hover:underline"
                >
                  Restart Speakeasy
                </button>{" "}
                after granting access.
              </p>
            </div>
          )}

          {step === "Test" && (
            <div>
              <h1 className="font-display text-[31px] leading-[1.05] tracking-[-0.01em] text-ink">
                Say something.
              </h1>
              <p className="mt-2.5 max-w-[52ch] text-[13px] leading-relaxed text-muted">
                Hold your hotkey and read this aloud: <em>“Speakeasy is ready to take dictation.”</em>{" "}
                You can also hold the button below instead.
              </p>

              <div className="mt-6 flex flex-wrap items-center gap-3">
                <Keycaps value={settings.hotkey} />
                <span className="text-[12px] text-muted">or</span>
                <button
                  type="button"
                  disabled={!modelReady}
                  onPointerDown={(event) => {
                    event.preventDefault();
                    void beginTest();
                  }}
                  onPointerUp={(event) => {
                    event.preventDefault();
                    void finishTest();
                  }}
                  onPointerCancel={() => void finishTest()}
                  onPointerLeave={() => void finishTest()}
                  onKeyDown={(event) => {
                    if (event.key === " " || event.key === "Enter") {
                      event.preventDefault();
                      if (!event.repeat) void beginTest();
                    }
                  }}
                  onKeyUp={(event) => {
                    if (event.key === " " || event.key === "Enter") {
                      event.preventDefault();
                      void finishTest();
                    }
                  }}
                  className={`select-none rounded-lg px-4 py-2.5 text-[12.5px] font-medium transition-colors ${
                    testState === "recording"
                      ? "bg-ember text-paper"
                      : "bg-ink text-paper hover:bg-ink-soft"
                  } disabled:cursor-not-allowed disabled:opacity-50`}
                >
                  {testState === "recording"
                    ? "Listening… release to stop"
                    : testState === "processing"
                      ? "Transcribing…"
                      : "Hold to test"}
                </button>
              </div>

              {!modelReady && (
                <p className="mt-3 text-[11.5px] text-amber">
                  The speech model is still downloading. Finish that step first.
                </p>
              )}
              {testError && (
                <p className="mt-3 max-w-[52ch] text-[11.5px] leading-relaxed text-ember" role="alert">
                  {testError}
                </p>
              )}

              {transcript && (
                <div className="mt-6 rounded-card border border-line bg-surface/85 p-6">
                  <p className="font-mono text-[9.5px] uppercase tracking-[0.2em] text-muted">
                    Transcript
                  </p>
                  <p className="mt-2 font-display text-[21px] leading-snug text-ink">
                    {transcript.text}
                  </p>
                  <p className="mt-3 font-mono text-[10.5px] text-faint">
                    {transcript.inferenceMs} ms inference · {transcript.totalMs} ms total
                  </p>
                </div>
              )}

              <p className="mt-5 max-w-[52ch] text-[11.5px] leading-relaxed text-faint">
                Nothing is pasted during this test. From now on the transcript lands in whichever
                app has focus.
              </p>
            </div>
          )}
        </div>

        <footer className="mt-8 flex items-center justify-between border-t border-line/80 pt-4">
          <button
            type="button"
            onClick={() => setIndex((current) => Math.max(0, current - 1))}
            disabled={index === 0}
            className="rounded-lg px-3 py-2 text-[12px] text-muted transition-colors hover:text-ink disabled:invisible"
          >
            Back
          </button>
          <div className="flex items-center gap-3">
            <span className="font-mono text-[10px] uppercase tracking-[0.16em] text-faint">
              {index + 1} / {steps.length}
            </span>
            <button
              type="button"
              onClick={next}
              disabled={!canContinue || saving}
              className="rounded-lg bg-ink px-5 py-2.5 text-[12.5px] font-medium text-paper transition-colors hover:bg-ink-soft disabled:cursor-not-allowed disabled:opacity-50"
            >
              {primaryLabel}
            </button>
          </div>
        </footer>
      </div>
    </main>
  );
}
