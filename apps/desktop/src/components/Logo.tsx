import { clsx } from "clsx";

type LogoProps = {
  size?: number;
  className?: string;
  title?: string;
};

export function Logo({ size = 28, className, title = "Conclave" }: LogoProps) {
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
      role="img"
      aria-label={title}
      className={clsx("inline-block text-ink", className)}
    >
      <circle cx={32} cy={32} r={24} />
      <circle cx={32} cy={8} r={5.5} fill="currentColor" stroke="none" />
      <circle cx={9.17} cy={24.58} r={5.5} fill="currentColor" stroke="none" />
      <circle cx={17.89} cy={51.42} r={5.5} fill="currentColor" stroke="none" />
      <circle cx={46.11} cy={51.42} r={5.5} fill="currentColor" stroke="none" />
      <circle cx={54.83} cy={24.58} r={5.5} fill="currentColor" stroke="none" />
      <circle cx={32} cy={32} r={8} fill="currentColor" stroke="none" />
    </svg>
  );
}
