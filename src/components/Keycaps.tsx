const LABELS: Record<string, string> = {
  ctrl: "Ctrl",
  control: "Ctrl",
  cmd: "Cmd",
  command: "Cmd",
  meta: "Cmd",
  super: "Win",
  win: "Win",
  alt: "Alt",
  option: "Option",
  shift: "Shift",
  space: "Space",
  fn: "fn",
  tab: "Tab",
  enter: "Enter",
  backspace: "Backspace",
  delete: "Delete",
  insert: "Insert",
  home: "Home",
  end: "End",
  pageup: "PgUp",
  pagedown: "PgDn",
  arrowup: "↑",
  arrowdown: "↓",
  arrowleft: "←",
  arrowright: "→",
  escape: "Esc",
  numpadenter: "Num Enter",
  numpadadd: "Num +",
  numpadsubtract: "Num −",
  numpadmultiply: "Num ×",
  numpaddivide: "Num ÷",
  numpaddecimal: "Num .",
  backquote: "`",
  minus: "−",
  equal: "=",
  bracketleft: "[",
  bracketright: "]",
  backslash: "\\",
  semicolon: ";",
  quote: "'",
  comma: ",",
  period: ".",
  slash: "/",
};

export function labelFor(part: string): string {
  const trimmed = part.trim();
  if (!trimmed) return "";

  const lower = trimmed.toLowerCase();
  if (LABELS[lower]) return LABELS[lower];

  // Browser `event.code` values map cleanly onto global-hotkey's key names.
  const letter = /^key([a-z])$/.exec(lower);
  if (letter) return letter[1]!.toUpperCase();

  const digit = /^digit([0-9])$/.exec(lower);
  if (digit) return digit[1]!;

  const numpad = /^numpad([0-9])$/.exec(lower);
  if (numpad) return `Num ${numpad[1]!}`;

  if (/^f([1-9]|1[0-9]|2[0-4])$/.test(lower)) return lower.toUpperCase();

  return trimmed.toUpperCase();
}

export function Keycaps({ value }: { value: string }) {
  const parts = value
    .split("+")
    .map(labelFor)
    .filter((part) => part.length > 0);

  if (parts.length === 0) {
    return (
      <div className="flex h-11 items-center">
        <span className="rounded-[9px] border border-dashed border-line-strong px-3 py-2 font-mono text-[12px] text-faint">
          Listening…
        </span>
      </div>
    );
  }

  return (
    <div className="flex flex-wrap items-center gap-2" aria-label={`Activation hotkey: ${value}`}>
      {parts.map((part, index) => (
        <span key={`${part}-${index}`} className="flex items-center gap-2">
          {index > 0 && (
            <span aria-hidden="true" className="text-[13px] text-faint">
              +
            </span>
          )}
          <kbd className="inline-flex h-11 min-w-11 items-center justify-center rounded-[9px] border border-line-strong bg-surface px-3 font-mono text-[14px] font-medium text-ink shadow-[0_1px_0_rgba(23,21,15,0.08),inset_0_-2px_0_rgba(23,21,15,0.05)]">
            {part}
          </kbd>
        </span>
      ))}
    </div>
  );
}
