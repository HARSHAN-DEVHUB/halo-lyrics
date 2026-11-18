// Playback clock + line/word timing.
//
// The backend sends samples {positionMs, positionAt, playing}. Between samples we
// extrapolate. A new sample that disagrees with our prediction by more than
// SEEK_THRESHOLD_MS is treated as a seek (jump); smaller disagreements are eased
// out over EASE_MS so the lyrics never stutter.

import type { Line, Word } from "./types.ts";

export const SEEK_THRESHOLD_MS = 400;
export const EASE_MS = 300;

export type ClockSample = {
  positionMs: number;
  positionAt: number;
  playing: boolean;
  rate?: number;
};

export type UpdateResult = "init" | "seek" | "ease";

const easeOutCubic = (k: number) => 1 - Math.pow(1 - k, 3);

export class PlaybackClock {
  private basePos = 0;
  private baseAt = 0;
  private playing = false;
  private rate = 1;
  private hasSample = false;
  private corrFrom = 0;
  private corrStart = 0;
  private lastOut = -Infinity;
  private readonly nowFn: () => number;

  constructor(nowFn: () => number = () => Date.now()) {
    this.nowFn = nowFn;
  }

  get isPlaying(): boolean {
    return this.playing;
  }

  private raw(t: number): number {
    return this.playing ? this.basePos + (t - this.baseAt) * this.rate : this.basePos;
  }

  private correction(t: number): number {
    if (this.corrFrom === 0) return 0;
    const k = (t - this.corrStart) / EASE_MS;
    if (k >= 1) {
      this.corrFrom = 0;
      return 0;
    }
    return this.corrFrom * (1 - easeOutCubic(Math.max(0, k)));
  }

  /** Current playback position in ms. */
  now(t: number = this.nowFn()): number {
    let v = Math.max(0, this.raw(t) + this.correction(t));
    // While playing, small corrections must never run time backwards (that would
    // flicker word states). Hold until the true position catches up instead.
    // Seeks reset `lastOut`, so real jumps backwards still happen.
    if (this.playing && v < this.lastOut && this.lastOut - v < SEEK_THRESHOLD_MS) v = this.lastOut;
    this.lastOut = v;
    return v;
  }

  /** Forget everything (call on track change). */
  reset(): void {
    this.hasSample = false;
    this.corrFrom = 0;
    this.lastOut = -Infinity;
  }

  update(sample: ClockSample, t: number = this.nowFn()): UpdateResult {
    const rate = sample.rate ?? 1;
    const samplePos = sample.playing
      ? sample.positionMs + Math.max(0, t - sample.positionAt) * rate
      : sample.positionMs;
    const predicted = this.now(t);
    const first = !this.hasSample;

    this.basePos = samplePos;
    this.baseAt = t;
    this.playing = sample.playing;
    this.rate = rate;
    this.hasSample = true;

    if (first) {
      this.corrFrom = 0;
      this.lastOut = -Infinity;
      return "init";
    }
    const diff = samplePos - predicted;
    if (Math.abs(diff) > SEEK_THRESHOLD_MS) {
      this.corrFrom = 0;
      this.lastOut = -Infinity;
      return "seek";
    }
    // Keep showing `predicted` right now, then ease toward the new sample.
    this.corrFrom = predicted - samplePos;
    this.corrStart = t;
    return "ease";
  }
}

/** Index of the last element with startMs <= t, or -1. Binary search. */
export function indexAt<T extends { startMs: number }>(items: T[], t: number): number {
  let lo = 0, hi = items.length - 1, ans = -1;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    if (items[mid].startMs <= t) {
      ans = mid;
      lo = mid + 1;
    } else hi = mid - 1;
  }
  return ans;
}

// ------------------------------------------------------------------ display items

/** Gaps (intro, breaks, breathers) of at least this long show the three dots. */
export const GAP_MS = 4000;
export const BREATHER = "♪";

export type DisplayItem =
  | { kind: "line"; key: string; startMs: number; endMs: number; text: string; words: Word[] }
  | { kind: "gap"; key: string; startMs: number; endMs: number };

/**
 * Turns parsed lines into the sequence the stage walks through: real lines plus
 * "gap" items for intros, instrumental breaks, long breathers and the outro.
 */
export function buildDisplayItems(lines: Line[], durationMs: number): DisplayItem[] {
  const out: DisplayItem[] = [];
  const pushGap = (startMs: number, endMs: number) => {
    if (endMs - startMs < GAP_MS) return;
    const last = out[out.length - 1];
    if (last && last.kind === "gap" && last.endMs >= startMs) {
      last.endMs = Math.max(last.endMs, endMs);
      return;
    }
    out.push({ kind: "gap", key: `g${startMs}`, startMs, endMs });
  };

  if (lines.length > 0) pushGap(0, lines[0].startMs);

  lines.forEach((line, i) => {
    const next = lines[i + 1];
    const isBreather = line.text.trim() === "" || line.text.trim() === BREATHER;
    const until = next ? next.startMs : line.endMs;
    if (isBreather && until - line.startMs >= GAP_MS) {
      pushGap(line.startMs, until);
      return;
    }
    const text = isBreather ? BREATHER : line.text;
    const words =
      line.words && line.words.length > 0
        ? line.words
        : [{ startMs: line.startMs, endMs: line.endMs, text }];
    out.push({ kind: "line", key: `l${i}`, startMs: line.startMs, endMs: line.endMs, text, words });
    if (next) pushGap(line.endMs, next.startMs);
    else if (durationMs > 0) pushGap(line.endMs, durationMs);
  });
  return out;
}

/** Index of the active word in a line at time t (-1 before the first word). */
export function activeWordIndex(words: Word[], t: number): number {
  return indexAt(words, t);
}

export function formatTime(ms: number): string {
  if (!Number.isFinite(ms) || ms < 0) ms = 0;
  const total = Math.floor(ms / 1000);
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const ss = s.toString().padStart(2, "0");
  return h > 0 ? `${h}:${m.toString().padStart(2, "0")}:${ss}` : `${m}:${ss}`;
}
