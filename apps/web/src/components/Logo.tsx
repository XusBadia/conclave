/**
 * Conclave brand mark — inline SVG so it inherits currentColor and ships
 * zero extra requests. Geometry matches apps/desktop/public/mark.svg.
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
      viewBox="0 0 64 64"
      width={size}
      height={size}
      fill="none"
      stroke="currentColor"
      strokeWidth={5}
      strokeLinecap="round"
      strokeLinejoin="round"
      className={className}
      role={ariaLabel ? "img" : undefined}
      aria-label={ariaLabel}
      aria-hidden={ariaLabel ? undefined : true}
    >
      <circle cx="32" cy="32" r="24" />
      <circle cx="32" cy="8" r="5.5" fill="currentColor" stroke="none" />
      <circle cx="9.17" cy="24.58" r="5.5" fill="currentColor" stroke="none" />
      <circle cx="17.89" cy="51.42" r="5.5" fill="currentColor" stroke="none" />
      <circle cx="46.11" cy="51.42" r="5.5" fill="currentColor" stroke="none" />
      <circle cx="54.83" cy="24.58" r="5.5" fill="currentColor" stroke="none" />
      <circle cx="32" cy="32" r="8" fill="currentColor" stroke="none" />
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
