interface MarkProps {
  className?: string;
}

const BARS = [
  { height: 5 },
  { height: 10 },
  { height: 15 },
  { height: 9 },
  { height: 4 },
];

export function Mark({ className = "h-9 w-9" }: MarkProps) {
  return (
    <span
      className={`inline-flex shrink-0 items-center justify-center rounded-[11px] bg-ink ${className}`}
      aria-hidden="true"
    >
      <svg viewBox="0 0 24 24" className="h-[19px] w-[19px]">
        {BARS.map((bar, index) => (
          <rect
            key={index}
            x={4 + index * 3.5}
            y={12 - bar.height / 2}
            width="2"
            height={bar.height}
            rx="1"
            fill={index === 2 ? "#E8854D" : "#F5F2EA"}
            opacity={index === 2 ? 1 : 0.82}
          />
        ))}
      </svg>
    </span>
  );
}
