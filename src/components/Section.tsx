import type { ReactNode } from "react";

interface SectionProps {
  eyebrow: string;
  title?: string;
  aside?: ReactNode;
  children: ReactNode;
  className?: string;
}

export function Section({ eyebrow, title, aside, children, className = "" }: SectionProps) {
  return (
    <section
      className={`rounded-card border border-line bg-surface/85 shadow-[0_1px_0_rgba(23,21,15,0.03)] ${className}`}
    >
      <header className="flex items-start justify-between gap-4 px-6 pt-5">
        <div>
          <h2 className="font-mono text-[10px] font-medium uppercase tracking-[0.2em] text-muted">
            {eyebrow}
          </h2>
          {title && (
            <p className="mt-1.5 font-display text-[19px] leading-none text-ink">{title}</p>
          )}
        </div>
        {aside}
      </header>
      <div className="px-6 pb-5 pt-4">{children}</div>
    </section>
  );
}

interface RowProps {
  title: string;
  description: string;
  children: ReactNode;
  last?: boolean;
}

export function SettingRow({ title, description, children, last = false }: RowProps) {
  return (
    <div
      className={`flex items-start justify-between gap-8 py-4 ${
        last ? "" : "border-b border-line/80"
      }`}
    >
      <div className="min-w-0">
        <p className="text-[13.5px] font-medium leading-tight text-ink">{title}</p>
        <p className="mt-1 text-[12px] leading-relaxed text-muted">{description}</p>
      </div>
      <div className="shrink-0 pt-0.5">{children}</div>
    </div>
  );
}
