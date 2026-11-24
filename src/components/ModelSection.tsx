import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";

import type { ModelOption, ModelStatus } from "../lib/events";
import { Section } from "./Section";

interface ModelSectionProps {
  model: ModelStatus | null;
  options: ModelOption[];
  /** Path stored in settings for the custom file. */
  customPath: string;
  busy: boolean;
  onSelect: (modelId: string, customPath?: string) => void;
  onDownload: () => void;
}

const formatBytes = (bytes: number): string => {
  if (bytes <= 0) return "0 MB";
  const megabytes = bytes / (1024 * 1024);
  if (megabytes >= 1024) return `${(megabytes / 1024).toFixed(2)} GB`;
  return `${Math.round(megabytes)} MB`;
};

const fileName = (path: string): string => {
  const parts = path.split(/[\\/]/);
  return parts[parts.length - 1] || path;
};

export function ModelSection({
  model,
  options,
  customPath,
  busy: savingSettings,
  onSelect,
  onDownload,
}: ModelSectionProps) {
  const [draftPath, setDraftPath] = useState(customPath);
  const [pathError, setPathError] = useState<string | null>(null);
  // Choosing "Custom file" before a path exists must still reveal the path
  // row, even though the backend keeps the previous selection until a valid
  // path is supplied.
  const [showCustom, setShowCustom] = useState(false);

  useEffect(() => {
    setDraftPath(customPath);
  }, [customPath]);

  const phase = model?.phase ?? "missing";
  const busy = phase === "downloading" || phase === "verifying";
  const custom = model?.custom ?? false;
  const progress =
    model && model.totalBytes > 0
      ? Math.min(100, Math.max(0, (model.receivedBytes / model.totalBytes) * 100))
      : 0;
  const resuming = (model?.receivedBytes ?? 0) > 0;
  const label = busy
    ? phase === "verifying"
      ? "Verifying…"
      : "Downloading…"
    : resuming
      ? "Resume download"
      : "Download model";

  const status = (() => {
    switch (phase) {
      case "ready":
        return { text: "Installed", className: "border-moss/25 bg-moss/10 text-moss" };
      case "downloading":
      case "verifying":
        return {
          text: formatBytes(model?.receivedBytes ?? 0),
          className: "border-amber/25 bg-amber/10 text-amber",
        };
      case "error":
        return { text: "Download failed", className: "border-ember/25 bg-ember/10 text-ember" };
      default:
        return {
          text: "Not installed",
          className: "border-line-strong bg-surface text-muted",
        };
    }
  })();

  const selected = options.find((option) => option.selected);
  const isCustomSelected = custom || Boolean(selected?.custom);
  const note = selected?.note ?? "Runs entirely on this device.";

  const browse = async () => {
    setPathError(null);
    try {
      const picked = await open({
        multiple: false,
        directory: false,
        title: "Choose a GGML Whisper model",
        filters: [{ name: "GGML Whisper model", extensions: ["bin"] }],
      });
      if (typeof picked !== "string") return;
      setDraftPath(picked);
      onSelect("custom", picked);
    } catch (error) {
      setPathError(error instanceof Error ? error.message : String(error));
    }
  };

  const usePath = () => {
    setPathError(null);
    if (!draftPath.trim()) {
      setPathError("Enter the path to a GGML Whisper model first.");
      return;
    }
    onSelect("custom", draftPath.trim());
  };

  return (
    <Section
      eyebrow="Speech model"
      title={model?.label ?? "Whisper"}
      aside={
        <span
          className={`inline-flex items-center gap-1.5 rounded-full border px-2.5 py-1 text-[10.5px] font-medium tracking-wide ${status.className}`}
        >
          <span className="h-1.5 w-1.5 rounded-full bg-current" aria-hidden="true" />
          {status.text}
        </span>
      }
    >
      <div className="flex items-start justify-between gap-8">
        <div className="min-w-0 flex-1">
          <p className="max-w-[52ch] text-[12.5px] leading-relaxed text-muted">{note}</p>
          <p className="mt-1.5 max-w-[52ch] text-[11.5px] leading-relaxed text-faint">
            {custom
              ? "Using a model already on this computer. Speakeasy never uploads audio, and this file stays where it is."
              : "Speakeasy downloads it once, keeps it loaded between dictations, and never sends audio anywhere."}
          </p>
          {model && phase !== "ready" && !custom && (
            <p className="mt-2 font-mono text-[10.5px] text-faint">
              {formatBytes(model.receivedBytes)} of {formatBytes(model.totalBytes)}
            </p>
          )}
          {model?.error && (phase === "error" || phase === "missing") && (
            <p
              className="mt-2 max-w-[52ch] break-words text-[11.5px] leading-relaxed text-ember"
              role="alert"
            >
              {model.error}
            </p>
          )}
        </div>

        {phase === "ready" ? (
          <span className="shrink-0 pt-0.5 font-mono text-[10px] uppercase tracking-[0.16em] text-faint">
            {custom ? fileName(model?.path ?? "") : `~${formatBytes(model?.totalBytes ?? 0)}`}
          </span>
        ) : (
          !custom && (
            <button
              type="button"
              onClick={onDownload}
              disabled={busy}
              className="shrink-0 rounded-lg bg-ink px-4 py-2 text-[12.5px] font-medium text-paper transition-colors hover:bg-ink-soft disabled:cursor-progress disabled:opacity-60"
            >
              {label}
            </button>
          )
        )}
      </div>

      <div className="mt-4 flex flex-wrap items-center gap-3 border-t border-line/80 pt-4">
        <label className="sr-only" htmlFor="model-picker">
          Whisper model
        </label>
        <select
          id="model-picker"
          value={showCustom ? "custom" : (selected?.id ?? "large-v3-turbo-q5")}
          disabled={busy || savingSettings}
          onChange={(event) => {
            const value = event.target.value;
            setShowCustom(value === "custom");
            setPathError(null);
            if (value !== "custom") {
              onSelect(value);
            } else if (draftPath.trim()) {
              onSelect("custom", draftPath.trim());
            }
          }}
          className="min-w-[220px] flex-1 rounded-lg border border-line-strong bg-paper px-3 py-2 text-[12.5px] text-ink outline-none transition-colors focus:border-ink/40 disabled:opacity-60"
        >
          {options.map((option) => (
            <option key={option.id} value={option.id}>
              {option.label}
              {option.custom
                ? ""
                : option.installed
                  ? " · installed"
                  : ` · ${formatBytes(option.bytes)} download`}
            </option>
          ))}
        </select>
        {custom && (
          <span className="font-mono text-[10.5px] text-faint" title={model?.path ?? ""}>
            {fileName(model?.path ?? "")}
          </span>
        )}
      </div>

      {(showCustom || isCustomSelected) && (
        <div className="mt-3">
          <div className="flex items-center gap-2">
            <input
              type="text"
              value={draftPath}
              onChange={(event) => setDraftPath(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Enter") usePath();
              }}
              placeholder="C:\models\ggml-model.bin"
              spellCheck={false}
              className="min-w-0 flex-1 rounded-lg border border-line-strong bg-paper px-3 py-2 font-mono text-[11.5px] text-ink outline-none transition-colors placeholder:text-faint focus:border-ink/40"
            />
            <button
              type="button"
              onClick={() => void browse()}
              className="shrink-0 rounded-lg border border-line-strong px-3 py-2 text-[12px] font-medium text-ink transition-colors hover:bg-paper"
            >
              Browse…
            </button>
            <button
              type="button"
              onClick={usePath}
              className="shrink-0 rounded-lg bg-ink px-3 py-2 text-[12px] font-medium text-paper transition-colors hover:bg-ink-soft"
            >
              Use file
            </button>
          </div>
          {pathError && (
            <p className="mt-2 text-[11.5px] leading-relaxed text-ember" role="alert">
              {pathError}
            </p>
          )}
        </div>
      )}

      {busy && (
        <div className="mt-4" aria-hidden="true">
          <div className="h-1.5 overflow-hidden rounded-full bg-line">
            <div
              className="h-full rounded-full bg-ink transition-[width] duration-300"
              style={{ width: `${phase === "verifying" ? 100 : progress}%` }}
            />
          </div>
        </div>
      )}
    </Section>
  );
}
