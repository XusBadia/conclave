import {
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { createPortal } from "react-dom";

import { cn } from "../lib/cn";

// Anchored popover that renders into `document.body` and positions
// itself with `position: fixed` from the trigger's bounding rect.
//
// Portaling sidesteps two traps: clipping by `overflow: hidden`
// ancestors, and the new-containing-block behaviour of ancestors with
// `transform`, `filter`, or `backdrop-filter` — `backdrop-blur` on the
// bulk-action toolbar would otherwise re-anchor a fixed child to the
// toolbar instead of the viewport. Closes on outside click and ESC,
// steals focus to the first focusable on open, and returns focus to
// whatever was focused before on close.

type Side = "top" | "bottom";
type Align = "start" | "center" | "end";

const FOCUSABLE_SELECTOR =
  'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

export function Popover({
  open,
  onOpenChange,
  anchor,
  children,
  side = "bottom",
  align = "end",
  sideOffset = 6,
  width = 280,
  ariaLabel,
}: {
  open: boolean;
  onOpenChange: (next: boolean) => void;
  /** The element the popover anchors to. Set via useState (not a ref)
   * so that swapping the active trigger re-runs positioning. */
  anchor: HTMLElement | null;
  children: ReactNode;
  side?: Side;
  align?: Align;
  sideOffset?: number;
  width?: number;
  ariaLabel?: string;
}) {
  const panelRef = useRef<HTMLDivElement | null>(null);
  const previouslyFocused = useRef<HTMLElement | null>(null);
  const [coords, setCoords] = useState<{ top: number; left: number } | null>(
    null,
  );

  // Position the panel. Runs synchronously before paint, so the
  // initial off-screen render (top:-9999) is never visible.
  useLayoutEffect(() => {
    if (!open || !anchor) {
      setCoords(null);
      return;
    }
    const recompute = () => {
      const panel = panelRef.current;
      const rect = anchor.getBoundingClientRect();
      const panelW = panel?.offsetWidth ?? width;
      const panelH = panel?.offsetHeight ?? 0;
      const top =
        side === "bottom"
          ? rect.bottom + sideOffset
          : rect.top - sideOffset - panelH;
      let left: number;
      if (align === "start") {
        left = rect.left;
      } else if (align === "end") {
        left = rect.right - panelW;
      } else {
        left = rect.left + rect.width / 2 - panelW / 2;
      }
      const margin = 8;
      left = Math.max(
        margin,
        Math.min(left, window.innerWidth - panelW - margin),
      );
      const clampedTop = Math.max(
        margin,
        Math.min(top, window.innerHeight - panelH - margin),
      );
      setCoords({ top: clampedTop, left });
    };
    recompute();
    window.addEventListener("resize", recompute);
    window.addEventListener("scroll", recompute, true);
    return () => {
      window.removeEventListener("resize", recompute);
      window.removeEventListener("scroll", recompute, true);
    };
  }, [open, anchor, side, align, sideOffset, width]);

  // Outside click + ESC.
  useEffect(() => {
    if (!open) return;
    const onPointerDown = (e: MouseEvent) => {
      const target = e.target as Node;
      if (panelRef.current?.contains(target)) return;
      if (anchor?.contains(target)) return;
      onOpenChange(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        onOpenChange(false);
      }
    };
    document.addEventListener("mousedown", onPointerDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onPointerDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [open, anchor, onOpenChange]);

  // Focus management — only re-runs when `open` toggles, so swapping
  // the anchor mid-open doesn't bounce focus around.
  useEffect(() => {
    if (!open) return;
    previouslyFocused.current = document.activeElement as HTMLElement | null;
    const id = window.setTimeout(() => {
      const first = panelRef.current?.querySelector<HTMLElement>(
        FOCUSABLE_SELECTOR,
      );
      (first ?? panelRef.current)?.focus();
    }, 0);
    return () => {
      window.clearTimeout(id);
      previouslyFocused.current?.focus?.();
    };
  }, [open]);

  if (!open || !anchor) return null;

  const transformOrigin = `${side === "bottom" ? "top" : "bottom"} ${
    align === "end" ? "right" : align === "start" ? "left" : "center"
  }`;

  return createPortal(
    <div
      ref={panelRef}
      role="dialog"
      aria-label={ariaLabel}
      tabIndex={-1}
      style={{
        position: "fixed",
        top: coords?.top ?? -9999,
        left: coords?.left ?? -9999,
        width,
        opacity: coords ? 1 : 0,
        transform: coords ? "scale(1)" : "scale(0.96)",
        transformOrigin,
        transition:
          "opacity 140ms ease-out, transform 140ms cubic-bezier(0.16, 1, 0.3, 1)",
      }}
      className={cn(
        "z-40 rounded-lg border border-border bg-bg-elevated shadow-soft",
      )}
    >
      {children}
    </div>,
    document.body,
  );
}
