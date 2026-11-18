// OKLCH helpers used to turn the artwork palette into a readable tint.

const FALLBACK = "#F4F1EC";

type Rgb = [number, number, number];

function hexToRgb(hex: string): Rgb | null {
  const m = /^#?([0-9a-f]{6})$/i.exec(hex.trim());
  if (!m) return null;
  const n = parseInt(m[1], 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}

const toLinear = (c: number) => {
  const v = c / 255;
  return v <= 0.04045 ? v / 12.92 : Math.pow((v + 0.055) / 1.055, 2.4);
};
const fromLinear = (v: number) => {
  const c = v <= 0.0031308 ? v * 12.92 : 1.055 * Math.pow(v, 1 / 2.4) - 0.055;
  return Math.round(Math.min(1, Math.max(0, c)) * 255);
};

export function rgbToOklch([r, g, b]: Rgb): [number, number, number] {
  const lr = toLinear(r), lg = toLinear(g), lb = toLinear(b);
  const l = Math.cbrt(0.4122214708 * lr + 0.5363325363 * lg + 0.0514459929 * lb);
  const m = Math.cbrt(0.2119034982 * lr + 0.6806995451 * lg + 0.1073969566 * lb);
  const s = Math.cbrt(0.0883024619 * lr + 0.2817188376 * lg + 0.6299787005 * lb);
  const L = 0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s;
  const A = 1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s;
  const B = 0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s;
  const C = Math.hypot(A, B);
  const H = (Math.atan2(B, A) * 180) / Math.PI;
  return [L, C, H < 0 ? H + 360 : H];
}

function oklchToLinear(L: number, C: number, H: number): [number, number, number] {
  const h = (H * Math.PI) / 180;
  const A = C * Math.cos(h), B = C * Math.sin(h);
  const l = (L + 0.3963377774 * A + 0.2158037573 * B) ** 3;
  const m = (L - 0.1055613458 * A - 0.0638541728 * B) ** 3;
  const s = (L - 0.0894841775 * A - 1.291485548 * B) ** 3;
  return [
    4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
    -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
    -0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s,
  ];
}

const inGamut = ([r, g, b]: number[]) =>
  r >= -1e-4 && r <= 1.0001 && g >= -1e-4 && g <= 1.0001 && b >= -1e-4 && b <= 1.0001;

export function oklchToHex(L: number, C: number, H: number): string {
  // Reduce chroma until the colour fits in sRGB.
  let c = C;
  let lin = oklchToLinear(L, c, H);
  for (let i = 0; i < 40 && !inGamut(lin); i++) {
    c *= 0.93;
    lin = oklchToLinear(L, c, H);
  }
  return "#" + lin.map((v) => fromLinear(v).toString(16).padStart(2, "0")).join("").toUpperCase();
}

/**
 * Most vivid palette colour, lightened to L≈85% in OKLCH so it reads on dark.
 * Greyscale artwork (or no artwork) falls back to the warm off-white.
 */
export function tintFromPalette(palette: string[] | undefined): string {
  if (!palette || palette.length === 0) return FALLBACK;
  let best: [number, number, number] | null = null;
  for (const hex of palette) {
    const rgb = hexToRgb(hex);
    if (!rgb) continue;
    const lch = rgbToOklch(rgb);
    if (!best || lch[1] > best[1]) best = lch;
  }
  if (!best || best[1] < 0.045) return FALLBACK;
  return oklchToHex(0.85, Math.min(best[1], 0.15), best[2]);
}

export function withAlpha(hex: string, alpha: number): string {
  const rgb = hexToRgb(hex) ?? [244, 241, 236];
  return `rgba(${rgb[0]}, ${rgb[1]}, ${rgb[2]}, ${alpha})`;
}
