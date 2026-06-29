import { clsx } from "clsx";

/**
 * "MD" brand suffix — rendered as a fine superscript after the "Conclave"
 * wordmark, trademark-style. Uses the accent color so it keeps the brand hue
 * while staying discreet. Pair it next to the `app.brand` text.
 */
type MdSuffixProps = React.HTMLAttributes<HTMLElement>;

export function MdSuffix({ className, ...rest }: MdSuffixProps) {
  return (
    <sup
      {...rest}
      className={clsx(
        "ml-0.5 text-[0.6em] font-medium tracking-normal",
        className,
      )}
      style={{ color: "rgb(var(--accent))" }}
    >
      MD
    </sup>
  );
}
