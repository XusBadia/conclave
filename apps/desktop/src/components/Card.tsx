import type { HTMLAttributes, ReactNode } from "react";

import { cn } from "../lib/cn";

export function Card({
  className,
  children,
  ...rest
}: HTMLAttributes<HTMLDivElement>) {
  return (
    <div
      className={cn(
        "rounded-xl border border-border bg-surface shadow-soft",
        className,
      )}
      {...rest}
    >
      {children}
    </div>
  );
}

export function CardHeader({
  title,
  subtitle,
  right,
}: {
  title: ReactNode;
  subtitle?: ReactNode;
  right?: ReactNode;
}) {
  return (
    <div className="flex items-start justify-between border-b border-border-subtle px-5 py-4">
      <div className="min-w-0">
        <h3 className="truncate text-[15px] font-semibold text-ink">{title}</h3>
        {subtitle && (
          <p className="mt-0.5 truncate text-[13px] text-ink-subtle">
            {subtitle}
          </p>
        )}
      </div>
      {right && <div className="no-drag ml-3 shrink-0">{right}</div>}
    </div>
  );
}

export function CardBody({
  className,
  children,
}: {
  className?: string;
  children: ReactNode;
}) {
  return <div className={cn("p-5", className)}>{children}</div>;
}
