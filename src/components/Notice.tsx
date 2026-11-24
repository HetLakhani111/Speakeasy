interface NoticeProps {
  message: string;
  onDismiss: () => void;
}

/** Bottom-center toast shared by settings and onboarding. */
export function Notice({ message, onDismiss }: NoticeProps) {
  return (
    <div
      role="alert"
      className="fixed bottom-5 left-1/2 z-50 flex max-w-[520px] -translate-x-1/2 items-start gap-3 rounded-xl bg-ink px-4 py-3 text-paper shadow-lg"
    >
      <p className="text-[11.5px] leading-relaxed">{message}</p>
      <button
        type="button"
        aria-label="Dismiss"
        onClick={onDismiss}
        className="text-paper/50 transition-colors hover:text-paper"
      >
        <svg viewBox="0 0 12 12" className="h-3 w-3" aria-hidden="true">
          <path
            d="m3 3 6 6M9 3l-6 6"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.4"
            strokeLinecap="round"
          />
        </svg>
      </button>
    </div>
  );
}
