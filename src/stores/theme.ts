import { create } from "zustand";
import type { ThemeName } from "../lib/types";

export const THEMES: ThemeName[] = ["Dark", "Dracula", "Light"];

/** Stamp the theme onto <html> so the CSS tokens switch. */
function apply(theme: ThemeName) {
  document.documentElement.setAttribute("data-theme", theme.toLowerCase());
}

export function normalizeTheme(name: string | null | undefined): ThemeName {
  return THEMES.find((t) => t.toLowerCase() === (name ?? "").toLowerCase()) ?? "Dark";
}

interface ThemeState {
  theme: ThemeName;
  setTheme: (theme: ThemeName) => void;
  cycle: () => ThemeName;
}

apply("Dark");

export const useThemeStore = create<ThemeState>((set, get) => ({
  theme: "Dark",
  setTheme: (theme) => {
    apply(theme);
    set({ theme });
  },
  cycle: () => {
    const next = THEMES[(THEMES.indexOf(get().theme) + 1) % THEMES.length];
    get().setTheme(next);
    return next;
  },
}));
