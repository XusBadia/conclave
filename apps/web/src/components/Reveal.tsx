"use client";

import { useEffect, useRef } from "react";
import type { CSSProperties, ElementType, ReactNode } from "react";

/**
 * Reveal — fades + slides children in when they scroll into view.
 *
 * One IntersectionObserver per mount (cheap; one element each); element
 * disconnects once revealed so we never re-trigger. CSS handles the visual
 * transition via the rules on [data-reveal] in globals.css. Honoring
 * prefers-reduced-motion is also CSS-level so we don't duplicate the check.
 *
 * Stagger: pass an `index` prop (and optional `stagger` step in ms) and the
 * delay is computed inline. Default stagger step is 80ms — short enough that
 * the eye perceives the group, long enough to feel intentional.
 */
type RevealProps = {
  children: ReactNode;
  as?: ElementType;
  className?: string;
  /** Item index in a list — multiplied by `stagger` to compute the delay. */
  index?: number;
  /** Per-item stagger step in ms. */
  stagger?: number;
  /** Explicit delay in ms — overrides `index * stagger`. */
  delay?: number;
  /** rootMargin handed to IntersectionObserver. Lower values trigger sooner. */
  rootMargin?: string;
  style?: CSSProperties;
};

export function Reveal({
  children,
  as: Tag = "div",
  className,
  index = 0,
  stagger = 80,
  delay,
  rootMargin = "0px 0px -10% 0px",
  style,
}: RevealProps) {
  const ref = useRef<HTMLElement>(null);

  useEffect(() => {
    const el = ref.current;
    if (!el) return;

    // If reduced motion is set, skip the observer and reveal immediately —
    // the CSS @media query already disables the transition, so this just
    // flips the visibility flag.
    if (
      typeof window !== "undefined" &&
      window.matchMedia("(prefers-reduced-motion: reduce)").matches
    ) {
      el.dataset.revealed = "true";
      return;
    }

    // If already in viewport at mount (above the fold), reveal on the next
    // frame so the transition still plays from the initial hidden state.
    const rect = el.getBoundingClientRect();
    const inViewport = rect.top < window.innerHeight && rect.bottom > 0;
    if (inViewport) {
      requestAnimationFrame(() => {
        el.dataset.revealed = "true";
      });
      return;
    }

    const observer = new IntersectionObserver(
      (entries) => {
        for (const entry of entries) {
          if (entry.isIntersecting) {
            (entry.target as HTMLElement).dataset.revealed = "true";
            observer.unobserve(entry.target);
          }
        }
      },
      { rootMargin, threshold: 0.05 },
    );

    observer.observe(el);
    return () => observer.disconnect();
  }, [rootMargin]);

  const computedDelay = delay ?? index * stagger;

  return (
    <Tag
      ref={ref}
      data-reveal=""
      className={className}
      style={
        computedDelay
          ? ({
              ...style,
              "--reveal-delay": `${computedDelay}ms`,
            } as CSSProperties)
          : style
      }
    >
      {children}
    </Tag>
  );
}
