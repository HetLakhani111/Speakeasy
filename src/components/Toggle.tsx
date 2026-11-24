interface ToggleProps {
  checked: boolean;
  onChange: (checked: boolean) => void;
  label: string;
  disabled?: boolean;
}

export function Toggle({ checked, onChange, label, disabled = false }: ToggleProps) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      disabled={disabled}
      onClick={() => onChange(!checked)}
      className={`relative mt-0.5 inline-flex h-[22px] w-[38px] shrink-0 items-center rounded-full border transition-colors duration-200 ${
        checked
          ? "border-ink bg-ink"
          : "border-line-strong bg-paper-deep hover:border-faint"
      } ${disabled ? "opacity-50" : ""}`}
    >
      <span
        aria-hidden="true"
        className={`inline-block h-[16px] w-[16px] rounded-full shadow-[0_1px_2px_rgba(23,21,15,0.28)] transition-transform duration-200 ease-out ${
          checked ? "translate-x-[18px] bg-surface" : "translate-x-[3px] bg-surface"
        }`}
      />
    </button>
  );
}
