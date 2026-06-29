import { clsx } from "clsx";

type LogoProps = {
  size?: number;
  className?: string;
  title?: string;
};

export function Logo({ size = 28, className, title = "Conclave MD" }: LogoProps) {
  return (
    <svg
      xmlns="http://www.w3.org/2000/svg"
      viewBox="8 8 48 48"
      width={size}
      height={size}
      role="img"
      aria-label={title}
      className={clsx("inline-block text-ink", className)}
    >
      <path
        d="M43 20.5C39.9 17.4 35.8 15.5 31.4 15.5C22.5 15.5 15.5 22.8 15.5 32C15.5 41.2 22.5 48.5 31.4 48.5C35.8 48.5 39.9 46.6 43 43.5"
        fill="none"
        stroke="currentColor"
        strokeWidth={7.5}
        strokeLinecap="round"
        strokeLinejoin="round"
      />
      <circle cx={43.5} cy={20.5} r={4.75} fill="rgb(var(--accent))" />
      <circle cx={43.5} cy={43.5} r={4.75} fill="rgb(var(--accent))" />
      <circle cx={32} cy={32} r={4.25} fill="currentColor" />
    </svg>
  );
}
