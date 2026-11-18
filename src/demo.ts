// Demo track for running the frontend without the backend (plain `npm run dev`
// in a browser, or `?demo` in the URL). The lyrics below were written for this
// project as test data — they are not from any real song.

import type { Line, LyricsEvent, NowPlaying, Word } from "./types.ts";

export const DEMO_LRC = `[ti:Second Cup]
[ar:Halo Test Ensemble]
[offset:0]
[00:05.00]Steam is rising slowly from the cup between my hands
[00:09.20]The window's grey but every corner of the room is warm
[00:13.40]
[00:19.00]<00:19.00>Every <00:19.55>light <00:19.95>in <00:20.20>the <00:20.45>city <00:21.30>hums <00:21.90>along <00:23.20>
[00:23.40]Counting all the seconds out in quarter notes
[00:27.60]Hold this note a little longer
[00:31.00]
[00:33.00]Let the kettle sing its one-note song
[00:37.20]Nothing here is late and nothing's gone
[00:41.40]Steam is rising slowly, slowly
[00:46.00]`;

const DEMO_DURATION_MS = 52_000;

function parseTs(s: string): number {
  const m = /^(\d+):(\d+)(?:[.:](\d+))?$/.exec(s);
  if (!m) return NaN;
  const frac = m[3] ? Number((m[3] + "00").slice(0, 3)) : 0;
  return Number(m[1]) * 60_000 + Number(m[2]) * 1000 + frac;
}

/** Tiny mirror of the Rust parser (lyrics/lrc.rs), just enough for the demo. */
function synthWords(text: string, start: number, end: number): Word[] {
  const toks = text.split(/\s+/).filter(Boolean);
  if (!toks.length) return [];
  const avail = (end - start) * 0.85;
  const total = toks.reduce((n, w) => n + w.length, 0);
  let t = start;
  return toks.map((w, i) => {
    const d = Math.max(120, (avail * w.length) / total);
    const word = { startMs: Math.round(t), endMs: i === toks.length - 1 ? end : Math.round(t + d), text: w };
    t += d;
    return word;
  });
}

export function demoLines(): Line[] {
  const raw: { t: number; text: string; words?: Word[] }[] = [];
  for (const row of DEMO_LRC.split("\n")) {
    const m = /^\[(\d+:\d+(?:\.\d+)?)\](.*)$/.exec(row.trim());
    if (!m) continue;
    const t = parseTs(m[1]);
    let text = m[2];
    let words: Word[] | undefined;
    if (text.includes("<")) {
      const parts = [...text.matchAll(/<(\d+:\d+\.\d+)>([^<]*)/g)];
      words = [];
      for (let i = 0; i < parts.length; i++) {
        const w = parts[i][2].trim();
        if (!w) continue;
        words.push({ startMs: parseTs(parts[i][1]), endMs: parseTs(parts[i + 1]?.[1] ?? parts[i][1]), text: w });
      }
      text = words.map((w) => w.text).join(" ");
    }
    raw.push({ t, text: text.trim(), words });
  }
  return raw.map((r, i) => {
    const next = raw[i + 1];
    const endMs = Math.min(next ? next.t : DEMO_DURATION_MS, r.t + 8000);
    const text = r.text || "♪";
    return {
      startMs: r.t,
      endMs,
      text,
      words: r.words ?? synthWords(text, r.t, endMs),
    };
  });
}

function demoArtwork(): string {
  const c = document.createElement("canvas");
  c.width = c.height = 512;
  const g = c.getContext("2d")!;
  g.fillStyle = "#1b1220";
  g.fillRect(0, 0, 512, 512);
  const blob = (x: number, y: number, r: number, col: string) => {
    const grad = g.createRadialGradient(x, y, 0, x, y, r);
    grad.addColorStop(0, col);
    grad.addColorStop(1, "rgba(0,0,0,0)");
    g.fillStyle = grad;
    g.fillRect(0, 0, 512, 512);
  };
  blob(150, 160, 320, "rgba(232,112,64,0.95)");
  blob(390, 340, 300, "rgba(120,58,120,0.9)");
  blob(330, 90, 200, "rgba(250,190,110,0.8)");
  blob(80, 430, 220, "rgba(40,70,130,0.85)");
  g.strokeStyle = "rgba(255,240,225,0.85)";
  g.lineWidth = 22;
  g.beginPath();
  g.arc(256, 256, 120, 0, Math.PI * 2);
  g.stroke();
  return c.toDataURL("image/jpeg", 0.9);
}

export function createDemo(): {
  nowPlaying: () => NowPlaying;
  lyrics: LyricsEvent;
  toggle: () => void;
  skip: (dir: 1 | -1) => void;
} {
  const art = demoArtwork();
  // `?demo&t=20000` starts the demo 20 s in (handy for screenshots).
  const startAt = Number(new URLSearchParams(location.search).get("t") ?? 2000) || 0;
  let startedAt = Date.now() - startAt;
  let pausedAt: number | null = null;
  const pos = () => {
    const now = pausedAt ?? Date.now();
    const p = now - startedAt;
    if (p > DEMO_DURATION_MS) {
      startedAt = now;
      return 0;
    }
    return p;
  };
  const trackKey = "demo-second-cup";
  return {
    nowPlaying: () => ({
      sourceApp: "Demo",
      title: "Second Cup",
      artist: "Halo Test Ensemble",
      album: "Test Patterns",
      durationMs: DEMO_DURATION_MS,
      positionMs: pos(),
      positionAt: Date.now(),
      playing: pausedAt === null,
      artwork: { dataUrl: art, palette: ["#E87040", "#FABE6E", "#783A78", "#28468A", "#1B1220"] },
      artworkKey: "demo",
      trackKey,
    }),
    lyrics: { trackKey, lyrics: { kind: "synced", lines: demoLines() } },
    toggle: () => {
      if (pausedAt === null) pausedAt = Date.now();
      else {
        startedAt += Date.now() - pausedAt;
        pausedAt = null;
      }
    },
    skip: (dir) => {
      startedAt += dir * -10_000;
    },
  };
}
