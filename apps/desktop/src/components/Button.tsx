import { forwardRef, type ButtonHTMLAttributes, type ReactNode } from "react";

import { cn } from "../lib/cn";

type Variant = "primary" | "secondary" | "ghost" | "danger";
type Size = "sm" | "md" | "lg";

interface Props extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: Variant;
  size?: Size;
  leftIcon?: ReactNode;
  rightIcon?: ReactNode;
  loading?: boolean;
}

const base =
  "inline-flex items-center justify-center gap-2 rounded-pill font-mono uppercase tracking-[0.08em] text-[11px] transition-colors select-none focus:outline-none focus-visible:ring-conclave disabled:opacity-50 disabled:cursor-not-allowed";

const variants: Record<Variant, string> = {
  primary:
    "border border-ink text-ink bg-transparent hover:bg-ink hover:text-bg",
  secondary:
    "border border-border text-ink-dim bg-transparent hover:bg-surface-hover hover:text-ink",
  ghost:
    "text-ink-dim hover:text-ink",
  danger:
    "border border-danger text-danger bg-transparent hover:bg-danger hover:text-bg",
};

const sizes: Record<Size, string> = {
  sm: "h-7 px-3.5",
  md: "h-8 px-4",
  lg: "h-9 px-5",
};

export const Button = forwardRef<HTMLButtonElement, Props>(function Button(
  {
    className,
    variant = "secondary",
    size = "md",
    leftIcon,
    rightIcon,
    loading,
    children,
    disabled,
    ...rest
  },
  ref,
) {
  return (
    <button
      ref={ref}
      disabled={disabled || loading}
      className={cn(base, variants[variant], sizes[size], className)}
      {...rest}
    >
      {loading ? (
        <span className="inline-flex items-center gap-1.5">
          <span className="h-1.5 w-1.5 rounded-full bg-current animate-pulseDot" />
          <span className="h-1.5 w-1.5 rounded-full bg-current animate-pulseDot [animation-delay:120ms]" />
          <span className="h-1.5 w-1.5 rounded-full bg-current animate-pulseDot [animation-delay:240ms]" />
        </span>
      ) : (
        leftIcon
      )}
      {children}
      {!loading && rightIcon}
    </button>
  );
});
