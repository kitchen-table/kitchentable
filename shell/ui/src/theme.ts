import { useCallback, useEffect, useState } from "react";

/**
 * Appearance.
 *
 * Three states, not two: "system" is the default and is what Settings means by
 * "follows macOS". Choosing light or dark writes `data-theme` on the root
 * element, which tokens.css lets win over the `prefers-color-scheme` query in
 * both directions - so an explicit choice survives the OS flipping at sunset.
 */
export type Appearance = "system" | "light" | "dark";

const KEY = "kt.appearance";

export function stored(): Appearance {
  try {
    const value = localStorage.getItem(KEY);
    return value === "light" || value === "dark" ? value : "system";
  } catch {
    // Storage disabled. Following the system is the right thing to fall back to.
    return "system";
  }
}

export function apply(appearance: Appearance): void {
  const root = document.documentElement;
  if (appearance === "system") {
    root.removeAttribute("data-theme");
  } else {
    root.setAttribute("data-theme", appearance);
  }
}

/** Whether dark is what the viewer is actually seeing, whatever the setting. */
export function isDark(appearance: Appearance): boolean {
  if (appearance !== "system") return appearance === "dark";
  return window.matchMedia?.("(prefers-color-scheme: dark)").matches ?? false;
}

export function useAppearance() {
  const [appearance, setAppearance] = useState<Appearance>(stored);

  useEffect(() => {
    apply(appearance);
    try {
      if (appearance === "system") localStorage.removeItem(KEY);
      else localStorage.setItem(KEY, appearance);
    } catch {
      // The setting still applies for this session; it just will not persist.
    }
  }, [appearance]);

  // Re-render when the OS flips, so the toggle's own icon stays honest while
  // the setting is "system".
  const [, force] = useState(0);
  useEffect(() => {
    if (appearance !== "system") return;
    const query = window.matchMedia?.("(prefers-color-scheme: dark)");
    if (!query) return;
    const onChange = () => force((n) => n + 1);
    query.addEventListener("change", onChange);
    return () => query.removeEventListener("change", onChange);
  }, [appearance]);

  /**
   * The title bar control is a single button, so it toggles between light and
   * dark rather than cycling through "system" - which would be a state nobody
   * can see the effect of. Settings offers the third option explicitly.
   */
  const toggle = useCallback(() => {
    setAppearance((current) => (isDark(current) ? "light" : "dark"));
  }, []);

  return { appearance, setAppearance, toggle, dark: isDark(appearance) };
}
