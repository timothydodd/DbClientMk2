// Deterministic, theme-aware accent colors derived from a name (database / connection); mirrors
// crates/dbclient-core/src/name_colors.rs so the tree, the tab badges and any server-side use agree.

import type { ThemeName } from "./types";

function hashName(name: string, lower: boolean): number {
  let hash = 0;
  for (const ch of lower ? name.toLowerCase() : name) {
    hash = (Math.imul(hash, 31) + ch.charCodeAt(0)) | 0;
  }
  return hash;
}

function hslToRgb(h: number, s: number, l: number): [number, number, number] {
  const hue2rgb = (p: number, q: number, t: number) => {
    if (t < 0) t += 1;
    if (t > 1) t -= 1;
    if (t < 1 / 6) return p + (q - p) * 6 * t;
    if (t < 1 / 2) return q;
    if (t < 2 / 3) return p + (q - p) * (2 / 3 - t) * 6;
    return p;
  };
  let r: number, g: number, b: number;
  if (s === 0) {
    r = g = b = l;
  } else {
    const q = l < 0.5 ? l * (1 + s) : l + s - l * s;
    const p = 2 * l - q;
    r = hue2rgb(p, q, h + 1 / 3);
    g = hue2rgb(p, q, h);
    b = hue2rgb(p, q, h - 1 / 3);
  }
  return [Math.round(r * 255), Math.round(g * 255), Math.round(b * 255)];
}

const hex = ([r, g, b]: [number, number, number]) => `#${[r, g, b].map((v) => v.toString(16).padStart(2, "0")).join("")}`;

/** Database color: hue from the lower-cased name, saturation/lightness tuned per theme. */
export function colorForName(name: string, theme: ThemeName): string {
  if (!name) return "#bd93f9";
  const hue = Math.abs(hashName(name, true) % 360) / 360;
  const [s, l] = theme === "Light" ? [0.62, 0.4] : theme === "Dracula" ? [0.78, 0.7] : [0.62, 0.62];
  return hex(hslToRgb(hue, s, l));
}

/** Connection tab color: hue from the name, fixed saturation and lightness. */
export function colorForConnection(name: string): string {
  if (!name) return "#bd93f9";
  const hue = Math.abs(hashName(name, false) % 360) / 360;
  return hex(hslToRgb(hue, 0.65, 0.6));
}
