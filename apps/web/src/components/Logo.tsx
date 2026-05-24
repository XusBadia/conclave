/**
 * Conclave brand mark — inline SVG so it inherits currentColor and ships
 * zero extra requests. Geometry matches public/mark.svg.
 */
type LogoProps = {
  size?: number;
  className?: string;
  ariaLabel?: string;
};

export function Logo({ size = 28, className, ariaLabel }: LogoProps) {
  return (
    <svg
      xmlns="http://www.w3.org/2000/svg"
      viewBox="8 8 48 48"
      width={size}
      height={size}
      className={className}
      role={ariaLabel ? "img" : undefined}
      aria-label={ariaLabel}
      aria-hidden={ariaLabel ? undefined : true}
    >
      <path
        d="M43 20.5C39.9 17.4 35.8 15.5 31.4 15.5C22.5 15.5 15.5 22.8 15.5 32C15.5 41.2 22.5 48.5 31.4 48.5C35.8 48.5 39.9 46.6 43 43.5"
        fill="none"
        stroke="currentColor"
        strokeWidth="7.5"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
      <circle cx="43.5" cy="20.5" r="4.75" fill="var(--color-accent)" />
      <circle cx="43.5" cy="43.5" r="4.75" fill="var(--color-accent)" />
      <circle cx="32" cy="32" r="4.25" fill="currentColor" />
    </svg>
  );
}

export function Wordmark({ className }: { className?: string }) {
  return (
    <span
      className={[
        "font-mono text-[15px] tracking-tight font-medium",
        className ?? "",
      ].join(" ")}
    >
      Conclave
    </span>
  );
}
