import { useCallback, useEffect, useId, useState } from "react";

import {
  listInputDevices,
  testMicrophone,
  type InputDeviceInfo,
  type MicProbe,
} from "../lib/api";

interface MicrophonePickerProps {
  value: string | null;
  disabled?: boolean;
  onChange: (device: string | null) => void;
}

function describeProbe(probe: MicProbe): { text: string; tone: "good" | "warn" | "bad" } {
  if (probe.peak < 0.004) {
    return {
      text: "Silence. Check the system microphone permission, or pick another device.",
      tone: "bad",
    };
  }
  if (probe.peak < 0.03) {
    return {
      text: "Very quiet — move closer or raise the input level.",
      tone: "warn",
    };
  }
  if (probe.peak > 0.98) {
    return { text: "Clipping — lower the input level a little.", tone: "warn" };
  }
  return { text: "Heard you clearly.", tone: "good" };
}

const TONE_CLASS = {
  good: "text-moss",
  warn: "text-amber",
  bad: "text-ember",
} as const;

export function MicrophonePicker({ value, disabled = false, onChange }: MicrophonePickerProps) {
  const selectId = useId();
  const [devices, setDevices] = useState<InputDeviceInfo[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [testing, setTesting] = useState(false);
  const [probe, setProbe] = useState<MicProbe | null>(null);
  const [probeError, setProbeError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      setDevices(await listInputDevices());
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  useEffect(() => {
    setProbe(null);
    setProbeError(null);
  }, [value]);

  const missing = value !== null && !loading && !devices.some((device) => device.name === value);

  const check = async () => {
    setTesting(true);
    setProbeError(null);
    setProbe(null);
    try {
      setProbe(await testMicrophone(value));
    } catch (err) {
      setProbeError(err instanceof Error ? err.message : String(err));
    } finally {
      setTesting(false);
    }
  };

  const result = probe ? describeProbe(probe) : null;

  return (
    <div className="min-w-0">
      <div className="flex flex-wrap items-center gap-2">
        <label className="sr-only" htmlFor={selectId}>
          Microphone
        </label>
        <select
          id={selectId}
          value={value ?? ""}
          disabled={disabled || loading}
          onChange={(event) => onChange(event.target.value || null)}
          className="h-9 max-w-[240px] flex-1 appearance-none rounded-lg border border-line-strong bg-surface px-3 text-[12.5px] text-ink transition-colors hover:border-faint focus:border-ink focus:outline-none disabled:opacity-60"
        >
          <option value="">System default</option>
          {missing && <option value={value}>{value} (not connected)</option>}
          {devices.map((device) => (
            <option key={device.name} value={device.name}>
              {device.name}
              {device.isDefault ? " · default" : ""}
            </option>
          ))}
        </select>

        <button
          type="button"
          onClick={() => void check()}
          disabled={disabled || testing}
          className="h-9 rounded-lg border border-line-strong bg-surface px-3 text-[12px] font-medium text-ink transition-colors hover:border-faint disabled:opacity-60"
        >
          {testing ? "Listening…" : "Test"}
        </button>

        <button
          type="button"
          onClick={() => void refresh()}
          disabled={disabled || loading}
          className="h-9 rounded-lg px-2 text-[11.5px] text-muted transition-colors hover:text-ink disabled:opacity-50"
        >
          Refresh
        </button>
      </div>

      {error && (
        <p className="mt-2 max-w-[46ch] text-[11.5px] leading-relaxed text-ember" role="alert">
          {error}
        </p>
      )}
      {missing && (
        <p className="mt-2 text-[11.5px] leading-relaxed text-amber">
          The saved microphone isn’t connected. Speakeasy will fall back to the system default.
        </p>
      )}
      {probeError && (
        <p className="mt-2 max-w-[46ch] text-[11.5px] leading-relaxed text-ember" role="alert">
          {probeError}
        </p>
      )}

      {probe && result && (
        <div className="mt-2.5">
          <div className="flex h-1.5 w-full max-w-[240px] overflow-hidden rounded-full bg-line">
            <div
              className={`h-full rounded-full transition-[width] duration-150 ${
                result.tone === "good"
                  ? "bg-moss"
                  : result.tone === "warn"
                    ? "bg-amber"
                    : "bg-ember"
              }`}
              style={{ width: `${Math.min(100, Math.max(2, probe.peak * 100))}%` }}
            />
          </div>
          <p className={`mt-1.5 text-[11.5px] leading-relaxed ${TONE_CLASS[result.tone]}`}>
            {result.text}
          </p>
        </div>
      )}
    </div>
  );
}
