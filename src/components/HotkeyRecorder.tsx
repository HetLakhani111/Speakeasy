import { useCallback, useEffect, useRef, useState } from "react";

import { setHotkeyCapture } from "../lib/api";
import { Keycaps } from "./Keycaps";

interface HotkeyRecorderProps {
  value: string;
  platform: string;
  disabled?: boolean;
  onCommit: (binding: string) => void;
}

type Modifier = "ctrl" | "alt" | "shift" | "meta";

const MODIFIER_BY_KEY: Record<string, Modifier> = {
  Control: "ctrl",
  Alt: "alt",
  Shift: "shift",
  Meta: "meta",
  OS: "meta",
};

const MODIFIER_RANK: Record<Modifier, number> = {
  ctrl: 0,
  alt: 1,
  shift: 2,
  meta: 3,
};

const KEY_FROM_CODE: Record<string, string> = {
  Space: "Space",
  Tab: "Tab",
  Enter: "Enter",
  NumpadEnter: "NumpadEnter",
  Backspace: "Backspace",
  Delete: "Delete",
  Insert: "Insert",
  Home: "Home",
  End: "End",
  PageUp: "PageUp",
  PageDown: "PageDown",
  ArrowUp: "ArrowUp",
  ArrowDown: "ArrowDown",
  ArrowLeft: "ArrowLeft",
  ArrowRight: "ArrowRight",
  Backquote: "Backquote",
  Minus: "Minus",
  Equal: "Equal",
  BracketLeft: "BracketLeft",
  BracketRight: "BracketRight",
  Backslash: "Backslash",
  Semicolon: "Semicolon",
  Quote: "Quote",
  Comma: "Comma",
  Period: "Period",
  Slash: "Slash",
  Numpad0: "Numpad0",
  Numpad1: "Numpad1",
  Numpad2: "Numpad2",
  Numpad3: "Numpad3",
  Numpad4: "Numpad4",
  Numpad5: "Numpad5",
  Numpad6: "Numpad6",
  Numpad7: "Numpad7",
  Numpad8: "Numpad8",
  Numpad9: "Numpad9",
  NumpadAdd: "NumpadAdd",
  NumpadSubtract: "NumpadSubtract",
  NumpadMultiply: "NumpadMultiply",
  NumpadDivide: "NumpadDivide",
  NumpadDecimal: "NumpadDecimal",
};

const PUNCTUATION_BY_KEY: Record<string, string> = {
  " ": "Space",
  "`": "Backquote",
  "-": "Minus",
  "=": "Equal",
  "[": "BracketLeft",
  "]": "BracketRight",
  "\\": "Backslash",
  ";": "Semicolon",
  "'": "Quote",
  ",": "Comma",
  ".": "Period",
  "/": "Slash",
};

const NAMED_KEYS = new Set([
  "Tab",
  "Enter",
  "Backspace",
  "Delete",
  "Insert",
  "Home",
  "End",
  "PageUp",
  "PageDown",
  "ArrowUp",
  "ArrowDown",
  "ArrowLeft",
  "ArrowRight",
]);

function keyFromEvent(event: KeyboardEvent): string | null {
  const code = event.code;
  if (/^Key[A-Z]$/.test(code)) return code;
  if (/^Digit[0-9]$/.test(code)) return code;
  if (/^F([1-9]|1[0-9]|2[0-4])$/.test(code)) return code;
  if (KEY_FROM_CODE[code]) return KEY_FROM_CODE[code];

  // A few environments (injected input, some remote sessions and layouts)
  // report an empty `code`. Fall back to the logical key so a binding can
  // still be recorded instead of rejecting the press outright.
  const key = event.key;
  if (key.length === 1) {
    if (/[a-zA-Z]/.test(key)) return `Key${key.toUpperCase()}`;
    if (/[0-9]/.test(key)) return `Digit${key}`;
    return PUNCTUATION_BY_KEY[key] ?? null;
  }
  if (NAMED_KEYS.has(key)) return key;
  if (/^F([1-9]|1[0-9]|2[0-4])$/.test(key)) return key;
  return null;
}

function modifierName(modifier: Modifier, platform: string): string {
  switch (modifier) {
    case "ctrl":
      return "Ctrl";
    case "alt":
      return "Alt";
    case "shift":
      return "Shift";
    case "meta":
      return platform === "macos" ? "Cmd" : "Super";
  }
}

function ordered(modules: Iterable<Modifier>): Modifier[] {
  return [...modules].sort((left, right) => MODIFIER_RANK[left] - MODIFIER_RANK[right]);
}

interface CaptureState {
  down: Set<Modifier>;
  used: Set<Modifier>;
  /** A rejected key press must not fall through to a modifier-only binding. */
  blocked: boolean;
}

export function HotkeyRecorder({
  value,
  platform,
  disabled = false,
  onCommit,
}: HotkeyRecorderProps) {
  const [capturing, setCapturing] = useState(false);
  const [preview, setPreview] = useState<string[]>([]);
  const [hint, setHint] = useState<string | null>(null);
  const capture = useRef<CaptureState>({ down: new Set(), used: new Set(), blocked: false });

  const stop = useCallback(() => {
    setCapturing(false);
    setPreview([]);
    setHint(null);
    void setHotkeyCapture(false).catch(() => undefined);
  }, []);

  const begin = useCallback(() => {
    capture.current = { down: new Set(), used: new Set(), blocked: false };
    setHint(null);
    setPreview([]);
    setCapturing(true);
    void setHotkeyCapture(true).catch(() => undefined);
  }, []);

  const finish = useCallback(
    (binding: string) => {
      stop();
      onCommit(binding);
    },
    [onCommit, stop],
  );

  useEffect(() => {
    return () => {
      // Leaving the window while listening must never leave dictation
      // suppressed in the backend.
      void setHotkeyCapture(false).catch(() => undefined);
    };
  }, []);

  useEffect(() => {
    if (!capturing) return;

    const handleKeyDown = (event: KeyboardEvent) => {
      event.preventDefault();
      event.stopPropagation();

      if (event.key === "Escape") {
        stop();
        return;
      }

      const modifier = MODIFIER_BY_KEY[event.key];
      if (modifier) {
        if (!event.repeat) {
          capture.current.down.add(modifier);
          capture.current.used.add(modifier);
          setPreview(
            ordered(capture.current.down).map((entry) => modifierName(entry, platform)),
          );
        }
        return;
      }

      if (event.repeat) return;

      const key = keyFromEvent(event);
      if (!key) {
        capture.current.blocked = true;
        setHint("That key can’t be used. Try a letter, number, function key, or punctuation.");
        return;
      }

      const modifiers = ordered(capture.current.down);
      if (modifiers.length === 0 && !/^F\d+$/.test(key)) {
        capture.current.blocked = true;
        setHint("Add Ctrl, Alt, Shift, or Cmd — or use a function key by itself.");
        return;
      }

      finish(
        [...modifiers.map((entry) => modifierName(entry, platform)), key].join("+"),
      );
    };

    const handleKeyUp = (event: KeyboardEvent) => {
      const modifier = MODIFIER_BY_KEY[event.key];
      if (!modifier) return;

      event.preventDefault();
      event.stopPropagation();

      capture.current.down.delete(modifier);
      setPreview(ordered(capture.current.down).map((entry) => modifierName(entry, platform)));

      if (capture.current.down.size > 0) return;
      if (capture.current.blocked) {
        capture.current.used.clear();
        capture.current.blocked = false;
        return;
      }

      const used = ordered(capture.current.used);
      const only = used[0];
      if (used.length === 1 && only) {
        if (only === "meta" && platform === "windows") {
          setHint("The Windows key opens Start. Choose Ctrl, Alt, or Shift.");
          capture.current.used.clear();
          return;
        }
        finish(modifierName(only, platform));
        return;
      }

      if (used.length > 1) {
        setHint(
          "That was several modifiers. Press one modifier alone, or a modifier plus another key.",
        );
        capture.current.used.clear();
      }
    };

    const handleBlur = () => stop();

    window.addEventListener("keydown", handleKeyDown, true);
    window.addEventListener("keyup", handleKeyUp, true);
    window.addEventListener("blur", handleBlur);

    return () => {
      window.removeEventListener("keydown", handleKeyDown, true);
      window.removeEventListener("keyup", handleKeyUp, true);
      window.removeEventListener("blur", handleBlur);
    };
  }, [capturing, finish, platform, stop]);

  return (
    <div>
      <div className="flex flex-wrap items-end gap-3">
        <Keycaps value={capturing ? preview.join("+") : value} />
        <button
          type="button"
          onClick={capturing ? stop : begin}
          disabled={disabled}
          aria-pressed={capturing}
          className="mb-0.5 rounded-lg border border-line-strong bg-surface px-3 py-2 text-[12px] font-medium text-ink transition-colors hover:border-faint disabled:opacity-50"
        >
          {capturing ? "Cancel" : "Change"}
        </button>
      </div>
      {capturing ? (
        <p className="mt-3 text-[11.5px] leading-relaxed text-muted" role="status">
          Press a modifier on its own, or a modifier plus a key. Escape cancels.
        </p>
      ) : (
        <p className="mt-3 text-[11.5px] leading-relaxed text-faint">
          Modifier-only keys work too — try holding just {platform === "macos" ? "Option" : "Alt"}.
        </p>
      )}
      {hint && (
        <p className="mt-2 text-[11.5px] leading-relaxed text-ember" role="alert">
          {hint}
        </p>
      )}
    </div>
  );
}
