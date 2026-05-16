import type {
  InputHTMLAttributes,
  ReactNode,
  TextareaHTMLAttributes,
} from "react";

import { cn } from "../lib/cn";

const ringBase =
  "block w-full rounded-lg border border-border bg-bg px-3 py-2 text-sm text-ink placeholder:text-ink-faint shadow-soft focus:outline-none focus:ring-conclave focus:border-accent transition";

export function Field({
  label,
  hint,
  error,
  children,
}: {
  label: ReactNode;
  hint?: ReactNode;
  error?: ReactNode;
  children: ReactNode;
}) {
  return (
    <label className="block">
      <span className="mb-1.5 block text-[13px] font-medium text-ink-dim">
        {label}
      </span>
      {children}
      {hint && !error && (
        <span className="mt-1 block text-[12px] text-ink-faint">{hint}</span>
      )}
      {error && (
        <span className="mt-1 block text-[12px] text-danger">{error}</span>
      )}
    </label>
  );
}

export function Input({ className, ...rest }: InputHTMLAttributes<HTMLInputElement>) {
  return <input className={cn(ringBase, className)} {...rest} />;
}

export function Textarea({
  className,
  ...rest
}: TextareaHTMLAttributes<HTMLTextAreaElement>) {
  return (
    <textarea
      className={cn(ringBase, "font-mono leading-6", className)}
      {...rest}
    />
  );
}
