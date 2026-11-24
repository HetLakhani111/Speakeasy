import { useCallback, useEffect, useRef, useState } from "react";
import type { ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { UnlistenFn } from "@tauri-apps/api/event";

import {
  EVENT_AUDIO_LEVELS,
  EVENT_NOTIFICATION,
  EVENT_RECORDING_STARTED,
  EVENT_RECORDING_STOPPED,
  EVENT_TRANSCRIPT,
  type AudioLevels,
  type Notification,
  type NotificationLevel,
} from "../lib/events";
import { Visualizer, type LevelFeed } from "./Visualizer";

type Phase = "idle" | "listening" | "transcribing" | "pasted";

const PHASE_LABEL: Record<Exclude<Phase, "idle">, string> = {
  listening: "Listening",
  transcribing: "Transcribing",
  pasted: "Pasted",
};

const LISTENING_WATCHDOG_MS = 30_000;
const TRANSCRIBING_WATCHDOG_MS = 10_000;
const PASTED_DISMISS_MS = 650;
const NOTICE_DISMISS_MS = 1600;

const MARK_BARS = [5, 10, 15, 9, 4];

interface Notice {
  level: NotificationLevel;
  message: string;
}

function OverlayMark({ live }: { live: boolean }) {
  return (
    <span className={live ? "overlay-mark is-live" : "overlay-mark"} aria-hidden="true">
      <svg viewBox="0 0 24 24" className="h-[15px] w-[15px]">
        {MARK_BARS.map((height, index) => (
          <rect
            key={index}
            x={4 + index * 3.5}
            y={12 - height / 2}
            width="2"
            height={height}
            rx="1"
            fill={index === 2 ? "#e8854d" : "#f5f2ea"}
            opacity={index === 2 ? 1 : 0.8}
          />
        ))}
      </svg>
    </span>
  );
}

function CheckGlyph() {
  return (
    <svg viewBox="0 0 16 16" className="overlay-check" fill="none" aria-hidden="true">
      <path
        d="M3.5 8.4 6.4 11.3 12.5 4.8"
        stroke="currentColor"
        strokeWidth="1.8"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

function OverlayState({ phase, notice }: { phase: Phase; notice: Notice | null }) {
  let content: ReactNode;
  let toneClass = "";

  if (notice) {
    toneClass = notice.level === "info" ? "is-info" : "is-warn";
    content = <span className="overlay-state-dot" />;
  } else if (phase === "transcribing") {
    content = (
      <span className="overlay-dots" aria-hidden="true">
        <i />
        <i />
        <i />
      </span>
    );
  } else if (phase === "pasted") {
    content = <CheckGlyph />;
  } else {
    content = <span className="overlay-live-dot" />;
  }

  return <span className={`overlay-well ${toneClass}`.trim()}>{content}</span>;
}

export function Overlay() {
  const [phase, setPhase] = useState<Phase>("idle");
  const [notice, setNotice] = useState<Notice | null>(null);
  const [session, setSession] = useState(0);

  const phaseRef = useRef<Phase>("idle");
  const feedRef = useRef<LevelFeed | null>(null);

  const transition = useCallback((next: Phase) => {
    phaseRef.current = next;
    setPhase(next);
  }, []);

  const hide = useCallback(() => {
    phaseRef.current = "idle";
    setPhase("idle");
    setNotice(null);
    feedRef.current = null;
    void invoke("hide_overlay").catch(() => undefined);
  }, []);

  useEffect(() => {
    let disposed = false;
    let hideTimer: number | null = null;

    const clearHide = (): void => {
      if (hideTimer !== null) {
        window.clearTimeout(hideTimer);
        hideTimer = null;
      }
    };

    const scheduleHide = (delayMs: number): void => {
      clearHide();
      hideTimer = window.setTimeout(() => {
        hideTimer = null;
        hide();
      }, delayMs);
    };

    const onStarted = (): void => {
      clearHide();
      feedRef.current = null;
      setNotice(null);
      transition("listening");
      setSession((value) => value + 1);
      scheduleHide(LISTENING_WATCHDOG_MS);
    };

    const onStopped = (): void => {
      if (phaseRef.current === "idle") return;
      clearHide();
      setNotice(null);
      transition("transcribing");
      scheduleHide(TRANSCRIBING_WATCHDOG_MS);
    };

    const onTranscript = (): void => {
      if (phaseRef.current === "idle") return;
      clearHide();
      setNotice(null);
      transition("pasted");
      scheduleHide(PASTED_DISMISS_MS);
    };

    const onNotification = (payload: Notification): void => {
      if (phaseRef.current === "idle") return;
      clearHide();
      setNotice({ level: payload.level, message: payload.message });
      scheduleHide(NOTICE_DISMISS_MS);
    };

    const onLevels = (payload: AudioLevels): void => {
      // Fallback for the very first dictation: if the webview had not finished
      // loading when `recording-started` was emitted, the first audio frames
      // still tell us that capture is live.
      if (phaseRef.current === "idle") {
        clearHide();
        transition("listening");
        setSession((value) => value + 1);
        scheduleHide(LISTENING_WATCHDOG_MS);
      }
      feedRef.current = {
        level: payload.level,
        bins: payload.bins,
        at: performance.now(),
      };
    };

    const unlisteners: UnlistenFn[] = [];

    const bind = async (): Promise<void> => {
      const listeners = await Promise.all([
        listen(EVENT_RECORDING_STARTED, onStarted),
        listen(EVENT_RECORDING_STOPPED, onStopped),
        listen<AudioLevels>(EVENT_AUDIO_LEVELS, (event) => onLevels(event.payload)),
        listen<Notification>(EVENT_NOTIFICATION, (event) => onNotification(event.payload)),
        listen(EVENT_TRANSCRIPT, onTranscript),
      ]);

      if (disposed) {
        for (const off of listeners) off();
        return;
      }
      unlisteners.push(...listeners);
    };

    void bind().catch(() => undefined);

    return () => {
      disposed = true;
      clearHide();
      for (const off of unlisteners) off();
      unlisteners.length = 0;
    };
  }, [hide, transition]);

  if (phase === "idle") return null;

  const listening = phase === "listening";
  const noticeTone = notice ? (notice.level === "info" ? "is-info" : "is-warn") : "";
  const label = notice ? notice.message : PHASE_LABEL[phase];

  return (
    <div className="overlay-stage">
      <div
        key={session}
        className={`overlay-capsule overlay-enter${listening ? " is-listening" : ""}`}
        role="status"
        aria-live="polite"
      >
        <OverlayMark live={listening} />

        <div className="overlay-body">
          <p
            className={notice ? `overlay-label overlay-label-notice ${noticeTone}` : "overlay-label"}
            title={notice?.message}
          >
            {label}
          </p>
          <Visualizer feedRef={feedRef} active={listening} />
        </div>

        <OverlayState phase={phase} notice={notice} />
      </div>
    </div>
  );
}
