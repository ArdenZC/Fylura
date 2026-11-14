import { useEffect, useRef } from "react";
import { playBadgePop, prefersReducedMotion } from "../../utils/motion";

/**
 * Numeric badge that gives one short spring pulse when its value changes.
 *
 * The pulse is a transform-only WAAPI run on this element, so it never
 * reflows the row it sits in and never stacks with a previous run.
 */
export function CountBadge({
  value,
  className,
  label
}: {
  value: number;
  className?: string;
  label?: string;
}) {
  const ref = useRef<HTMLSpanElement | null>(null);
  const previousRef = useRef(value);

  useEffect(() => {
    if (previousRef.current === value) return;
    previousRef.current = value;
    if (prefersReducedMotion()) return;
    playBadgePop(ref.current);
  }, [value]);

  return (
    <span ref={ref} className={className} aria-label={label}>
      {value}
    </span>
  );
}
