// Motion helpers. Every animation here is decoration only: the DOM always ends
// on the exact formatted value, and nothing animates when the user asked the
// system for reduced motion (or when there is no matchMedia, as in tests).

import { useEffect, useRef, useState } from "react";

export function prefersMotion(): boolean {
  if (typeof window === "undefined" || !window.matchMedia) return false;
  return window.matchMedia("(prefers-reduced-motion: no-preference)").matches;
}

/**
 * Ease a number from its previous value to `target`. A `null` target
 * (Unavailable) is never interpolated; it renders immediately. The first known
 * value ramps up from zero purely as an entrance effect.
 */
export function useCountUp(target: number | null, duration = 720) {
  const [value, setValue] = useState(() =>
    target != null && prefersMotion() ? 0 : target,
  );
  const fromRef = useRef<number | null>(null);

  useEffect(() => {
    const from = fromRef.current ?? 0;
    fromRef.current = target;
    if (target == null || from === target || !prefersMotion()) {
      setValue(target);
      return;
    }
    let frame = 0;
    const started = performance.now();
    const step = (now: number) => {
      const t = Math.min(1, (now - started) / duration);
      const eased = 1 - Math.pow(1 - t, 4);
      setValue(t >= 1 ? target : from + (target - from) * eased);
      if (t < 1) frame = requestAnimationFrame(step);
    };
    frame = requestAnimationFrame(step);
    return () => cancelAnimationFrame(frame);
  }, [target, duration]);

  return value;
}

/**
 * One delegated pointer listener drives the cursor-following glow on every
 * `[data-glow]` surface; cheaper than a handler per card and invisible to
 * components that do not opt in.
 */
export function installPointerGlow(): () => void {
  const handle = (event: PointerEvent) => {
    const target = (event.target as Element | null)?.closest<HTMLElement>(
      "[data-glow]",
    );
    if (!target) return;
    const rect = target.getBoundingClientRect();
    target.style.setProperty("--mx", `${event.clientX - rect.left}px`);
    target.style.setProperty("--my", `${event.clientY - rect.top}px`);
  };
  window.addEventListener("pointermove", handle, { passive: true });
  return () => window.removeEventListener("pointermove", handle);
}
